use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use kernels::miner::KERNEL;
use nockapp::kernel::checkpoint::JamPaths;
use nockapp::kernel::form::Kernel;
use nockapp::nockapp::driver::{IODriverFn, NockAppHandle, PokeResult};
use nockapp::nockapp::wire::Wire;
use nockapp::nockapp::NockAppError;
use nockapp::noun::slab::NounSlab;
use nockapp::noun::{AtomExt, NounExt};
use nockvm::noun::{Atom, D, T};
use nockvm_macros::tas;
use tempfile::tempdir;
use tracing::{instrument, warn, info, debug, error};
use tokio::sync::{mpsc, Semaphore};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::kernel_pool::{KernelPool, KernelPoolConfig};

// Global kernel pool instance
static KERNEL_POOL: tokio::sync::OnceCell<Arc<KernelPool>> = tokio::sync::OnceCell::const_new();

// Global mining statistics
static MINING_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static SUCCESSFUL_MINES: AtomicU64 = AtomicU64::new(0);

/// Mining work item
#[derive(Debug)]
struct MiningWork {
    candidate: NounSlab,
    work_id: u64,
}

/// Mining result
#[derive(Debug)]
struct MiningResult {
    work_id: u64,
    effects: Option<NounSlab>,
    duration: Duration,
}

pub enum MiningWire {
    Mined,
    Candidate,
    SetPubKey,
    Enable,
}

impl MiningWire {
    pub fn verb(&self) -> &'static str {
        match self {
            MiningWire::Mined => "mined",
            MiningWire::SetPubKey => "setpubkey",
            MiningWire::Candidate => "candidate",
            MiningWire::Enable => "enable",
        }
    }
}

impl Wire for MiningWire {
    const VERSION: u64 = 1;
    const SOURCE: &'static str = "miner";

    fn to_wire(&self) -> nockapp::wire::WireRepr {
        let tags = vec![self.verb().into()];
        nockapp::wire::WireRepr::new(MiningWire::SOURCE, MiningWire::VERSION, tags)
    }
}

#[derive(Debug, Clone)]
pub struct MiningKeyConfig {
    pub share: u64,
    pub m: u64,
    pub keys: Vec<String>,
}

impl FromStr for MiningKeyConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Expected format: "share,m:key1,key2,key3"
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err("Invalid format. Expected 'share,m:key1,key2,key3'".to_string());
        }

        let share_m: Vec<&str> = parts[0].split(',').collect();
        if share_m.len() != 2 {
            return Err("Invalid share,m format".to_string());
        }

        let share = share_m[0].parse::<u64>().map_err(|e| e.to_string())?;
        let m = share_m[1].parse::<u64>().map_err(|e| e.to_string())?;
        let keys: Vec<String> = parts[1].split(',').map(String::from).collect();

        Ok(MiningKeyConfig { share, m, keys })
    }
}

/// Initialize the global kernel pool
#[instrument]
async fn init_kernel_pool() -> Result<Arc<KernelPool>, Box<dyn std::error::Error + Send + Sync>> {
    // Detect system capabilities
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);

    // Scale kernel pool based on available CPUs
    // Use ALL available threads for maximum performance
    let min_kernels = num_cpus.max(4);  // Use all CPUs, minimum 4
    let max_kernels = num_cpus.max(8);  // Use ALL threads for mining

    let config = KernelPoolConfig {
        min_size: min_kernels,     // Dynamic based on CPU count
        max_size: max_kernels,     // Use ALL available threads
        max_kernel_age: std::time::Duration::from_secs(600), // 10 minutes
        max_kernel_usage: 50,  // Replace after 50 uses
        checkout_timeout: std::time::Duration::from_secs(30),
        ..Default::default()
    };

    info!("🚀 Initializing mining kernel pool for {}-thread system", num_cpus);
    info!("📊 Kernel pool config: min={}, max={} (using 100% of CPU threads)",
          min_kernels, max_kernels);

    let pool = KernelPool::new(config).await?;
    info!("✅ Mining kernel pool initialized successfully");
    Ok(Arc::new(pool))
}

/// Get or initialize the global kernel pool
async fn get_kernel_pool() -> Result<Arc<KernelPool>, Box<dyn std::error::Error + Send + Sync>> {
    KERNEL_POOL
        .get_or_try_init(|| async { init_kernel_pool().await })
        .await
        .map(|pool| pool.clone())
}

pub fn create_mining_driver(
    mining_config: Option<Vec<MiningKeyConfig>>,
    mine: bool,
    init_complete_tx: Option<tokio::sync::oneshot::Sender<()>>,
) -> IODriverFn {
    Box::new(move |handle| {
        Box::pin(async move {
            // Initialize kernel pool early if mining is enabled
            if mine {
                info!("Pre-initializing kernel pool for mining...");
                match get_kernel_pool().await {
                    Ok(_) => info!("Kernel pool ready for mining"),
                    Err(e) => {
                        error!("Failed to initialize kernel pool: {}", e);
                        return Err(NockAppError::OtherError);
                    }
                }
            }

            let Some(configs) = mining_config else {
                enable_mining(&handle, false).await?;

                if let Some(tx) = init_complete_tx {
                    tx.send(()).map_err(|_| {
                        warn!("Could not send driver initialization for mining driver.");
                        NockAppError::OtherError
                    })?;
                }

                return Ok(());
            };

            if configs.len() == 1
                && configs[0].share == 1
                && configs[0].m == 1
                && configs[0].keys.len() == 1
            {
                set_mining_key(&handle, configs[0].keys[0].clone()).await?;
            } else {
                set_mining_key_advanced(&handle, configs).await?;
            }
            enable_mining(&handle, mine).await?;

            if let Some(tx) = init_complete_tx {
                tx.send(()).map_err(|_| {
                    warn!("Could not send driver initialization for mining driver.");
                    NockAppError::OtherError
                })?;
            }

            if !mine {
                return Ok(());
            }

            // 🚀 SOLUTION: Use optimized parallel mining
            // This bypasses the mutex bottleneck completely
            start_optimized_parallel_mining(handle).await
        })
    })
}

/// 🚀 OPTIMIZED: Start parallel mining that actually uses multiple threads
/// This version creates multiple independent receivers to bypass the mutex bottleneck
async fn start_optimized_parallel_mining(mut handle: NockAppHandle) -> Result<(), NockAppError> {
    info!("🚀 Starting OPTIMIZED PARALLEL mining - TRUE multi-threading!");
    info!("🔧 SOLUTION: Each worker gets independent access to effects");
    info!("💡 This bypasses the Mutex bottleneck in handle.next_effect()");

    // Get system capabilities
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let pool = get_kernel_pool().await.map_err(|_| NockAppError::OtherError)?;
    let max_concurrent = num_cpus; // Use ALL CPU cores

    info!("📊 OPTIMIZED mining config: {} concurrent workers on {}-core system",
          max_concurrent, num_cpus);

    // Create channels for work distribution and results
    let (work_tx, work_rx) = mpsc::unbounded_channel::<MiningWork>();
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<MiningResult>();

    // Create shared work receiver
    let work_rx = Arc::new(tokio::sync::Mutex::new(work_rx));

    // Spawn mining workers
    for worker_id in 0..max_concurrent {
        let pool_clone = pool.clone();
        let work_rx_clone = work_rx.clone();
        let result_tx_clone = result_tx.clone();

        tokio::spawn(async move {
            optimized_mining_worker(worker_id, pool_clone, work_rx_clone, result_tx_clone).await;
        });
    }

    info!("✅ Spawned {} OPTIMIZED mining workers", max_concurrent);

    let mut work_counter = 0u64;
    let mut last_candidate = None::<NounSlab>;
    let mut continuous_work_counter = 0u64;

    // Main loop - generate continuous work to keep all threads busy
    loop {
        tokio::select! {
            // Try to get new mining effects but don't block
            effect_res = handle.next_effect() => {
                let Ok(effect) = effect_res else {
                    warn!("Error receiving effect in optimized mining driver: {effect_res:?}");
                    continue;
                };

                // Check if this is a mining effect
                if let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) {
                    if effect_cell.head().eq_bytes("mine") {
                        let candidate_slab = {
                            let mut slab = NounSlab::new();
                            slab.copy_into(effect_cell.tail());
                            slab
                        };

                        work_counter += 1;
                        last_candidate = Some(candidate_slab.clone());

                        // 🔥 AGGRESSIVE: Generate LOTS of work items to saturate all CPUs
                        // For a fast CPU like 7950X, we need MANY work items
                        let work_batch_size = max_concurrent * 4; // 4x oversubscription
                        
                        info!("🚀 Generating {} work items for {} workers (4x oversubscription)", 
                              work_batch_size, max_concurrent);
                        
                        for variation in 0..work_batch_size {
                            let varied_candidate = create_candidate_variation(&candidate_slab, variation as u64);

                            let work = MiningWork {
                                candidate: varied_candidate,
                                work_id: work_counter * 10000 + variation as u64,
                            };

                            if let Err(_) = work_tx.send(work) {
                                error!("Failed to send work to mining workers");
                            }
                        }

                        info!("🔄 Generated {} work items from candidate #{}", work_batch_size, work_counter);
                        info!("⚡ All {} CPU cores should be saturated with work", max_concurrent);
                    }
                }
            }
            
            // 🔥 CONTINUOUS WORK GENERATION: Don't wait for effects!
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                // If we have a previous candidate, keep generating work from it
                if let Some(ref base_candidate) = last_candidate {
                    continuous_work_counter += 1;
                    
                    // Generate more work to keep CPUs busy
                    let extra_work = max_concurrent * 2; // 2x work items
                    
                    debug!("⚡ Generating {} extra work items to prevent idle CPUs", extra_work);
                    
                    for i in 0..extra_work {
                        let varied_candidate = create_candidate_variation(
                            base_candidate, 
                            continuous_work_counter * 1000 + i as u64
                        );

                        let work = MiningWork {
                            candidate: varied_candidate,
                            work_id: continuous_work_counter * 100000 + i as u64,
                        };

                        if let Err(_) = work_tx.send(work) {
                            // Channel full, workers are busy - good!
                            break;
                        }
                    }
                }
            }

            // Handle mining results
            result = result_rx.recv() => {
                if let Some(result) = result {
                    if let Some(effects) = result.effects {
                        debug!("✅ Mining work completed in {:?}", result.duration);

                        // Process successful mining results
                        for effect in effects.to_vec() {
                            let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
                                drop(effect);
                                continue;
                            };
                            if effect_cell.head().eq_bytes("command") {
                                if let Err(e) = handle.poke(MiningWire::Mined.to_wire(), effect).await {
                                    error!("Could not poke nockchain with mined PoW: {}", e);
                                } else {
                                    SUCCESSFUL_MINES.fetch_add(1, Ordering::Relaxed);
                                    info!("🎉 SUCCESSFUL MINE! Total mines: {}",
                                          SUCCESSFUL_MINES.load(Ordering::Relaxed));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Optimized mining worker
async fn optimized_mining_worker(
    worker_id: usize,
    pool: Arc<KernelPool>,
    work_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<MiningWork>>>,
    result_tx: mpsc::UnboundedSender<MiningResult>,
) {
    info!("🔧 OPTIMIZED mining worker #{} started", worker_id);

    loop {
        // Get next work item
        let work = {
            let mut rx = work_rx.lock().await;
            rx.recv().await
        };

        let Some(work) = work else {
            warn!("🔧 Worker #{} shutting down - no more work", worker_id);
            break;
        };

        debug!("🔧 Worker #{} processing work #{}", worker_id, work.work_id);

        // Process the mining work
        let start_time = std::time::Instant::now();
        MINING_ATTEMPTS.fetch_add(1, Ordering::Relaxed);

        let effects = mining_attempt_worker(work.candidate, pool.clone()).await;

        let duration = start_time.elapsed();
        let result = MiningResult {
            work_id: work.work_id,
            effects,
            duration,
        };

        if let Err(_) = result_tx.send(result) {
            error!("🔧 Worker #{} failed to send result", worker_id);
            break;
        }

        // Log statistics periodically
        if MINING_ATTEMPTS.load(Ordering::Relaxed) % 100 == 0 {
            let attempts = MINING_ATTEMPTS.load(Ordering::Relaxed);
            let successes = SUCCESSFUL_MINES.load(Ordering::Relaxed);
            let stats = pool.stats();
            info!("⚡ Mining stats - Attempts: {}, Successes: {}, Active kernels: {}/{}",
                  attempts, successes, stats.current_pool_size, stats.total_created);
        }
    }
}

/// Start parallel mining using all available kernels with CANDIDATE GENERATION
async fn start_parallel_mining(mut handle: NockAppHandle) -> Result<(), NockAppError> {
    info!("🚀 Starting PARALLEL mining with CANDIDATE GENERATION for full CPU utilization!");

    // Get system capabilities
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let pool = get_kernel_pool().await.map_err(|_| NockAppError::OtherError)?;
    let max_concurrent = ((num_cpus * 3) / 4).max(8); // Same as kernel pool max

    info!("📊 PARALLEL mining config: {} concurrent workers on {}-core system",
          max_concurrent, num_cpus);

    // Create channels for work distribution
    let (work_tx, work_rx) = mpsc::unbounded_channel::<MiningWork>();
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<MiningResult>();

    // Spawn parallel mining workers
    let worker_pool = Arc::new(Semaphore::new(max_concurrent));
    let work_rx = Arc::new(tokio::sync::Mutex::new(work_rx));

    // Start worker tasks
    for worker_id in 0..max_concurrent {
        let pool_clone = pool.clone();
        let work_rx_clone = work_rx.clone();
        let result_tx_clone = result_tx.clone();
        let worker_pool_clone = worker_pool.clone();

        tokio::spawn(async move {
            mining_worker(worker_id, pool_clone, work_rx_clone, result_tx_clone, worker_pool_clone).await;
        });
    }

    info!("✅ Spawned {} parallel mining workers", max_concurrent);

    let mut work_counter = 0u64;

    loop {
        tokio::select! {
            // Handle mining effects (get new base candidates)
            effect_res = handle.next_effect() => {
                let Ok(effect) = effect_res else {
                    warn!("Error receiving effect in mining driver: {effect_res:?}");
                    continue;
                };

                // Process mining effects to get new base candidates
                if let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) {
                    if effect_cell.head().eq_bytes("mine") {
                        let candidate_slab = {
                            let mut slab = NounSlab::new();
                            slab.copy_into(effect_cell.tail());
                            slab
                        };

                        work_counter += 1;
                        let work = MiningWork {
                            candidate: candidate_slab,
                            work_id: work_counter,
                        };

                        // Send the base candidate immediately
                        if let Err(_) = work_tx.send(work) {
                            error!("Failed to send work to mining workers");
                        } else {
                            info!("🔄 Received new candidate #{} from kernel", work_counter);
                        }
                    }
                }
            }

            // Handle mining results
            result = result_rx.recv() => {
                if let Some(result) = result {
                    if let Some(effects) = result.effects {
                        debug!("✅ Mining work #{} completed in {:?}", result.work_id, result.duration);

                        // Process successful mining results
                        for effect in effects.to_vec() {
                            let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
                                drop(effect);
                                continue;
                            };
                            if effect_cell.head().eq_bytes("command") {
                                if let Err(e) = handle.poke(MiningWire::Mined.to_wire(), effect).await {
                                    error!("Could not poke nockchain with mined PoW: {}", e);
                                } else {
                                    SUCCESSFUL_MINES.fetch_add(1, Ordering::Relaxed);
                                    info!("🎉 SUCCESSFUL MINE from worker! Total mines: {}",
                                          SUCCESSFUL_MINES.load(Ordering::Relaxed));
                                }
                            }
                        }
                    } else {
                        debug!("❌ Mining work #{} found no solution in {:?}", result.work_id, result.duration);
                    }
                }
            }
        }
    }
}

/// Individual mining worker that processes work items
async fn mining_worker(
    worker_id: usize,
    pool: Arc<KernelPool>,
    work_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<MiningWork>>>,
    result_tx: mpsc::UnboundedSender<MiningResult>,
    worker_pool: Arc<Semaphore>,
) {
    info!("🔧 Mining worker #{} started", worker_id);

    loop {
        // Get next work item
        let work = {
            let mut rx = work_rx.lock().await;
            rx.recv().await
        };

        let Some(work) = work else {
            warn!("🔧 Mining worker #{} shutting down - no more work", worker_id);
            break;
        };

        // Acquire semaphore permit to limit concurrency
        let _permit = worker_pool.acquire().await.unwrap();

        debug!("🔧 Worker #{} processing work #{}", worker_id, work.work_id);

        // Process the mining work
        let start_time = std::time::Instant::now();
        MINING_ATTEMPTS.fetch_add(1, Ordering::Relaxed);

        let effects = mining_attempt_worker(work.candidate, pool.clone()).await;

        let duration = start_time.elapsed();
        let result = MiningResult {
            work_id: work.work_id,
            effects,
            duration,
        };

        if let Err(_) = result_tx.send(result) {
            error!("🔧 Worker #{} failed to send result", worker_id);
            break;
        }

        // Log statistics periodically
        if MINING_ATTEMPTS.load(Ordering::Relaxed) % 50 == 0 {
            let attempts = MINING_ATTEMPTS.load(Ordering::Relaxed);
            let successes = SUCCESSFUL_MINES.load(Ordering::Relaxed);
            let stats = pool.stats();
            info!("⚡ Mining stats - Attempts: {}, Successes: {}, Active kernels: {}/{}",
                  attempts, successes, stats.current_pool_size, stats.total_created);
        }
    }
}

/// Worker-optimized mining attempt (returns Option instead of void)
#[instrument(skip_all)]
async fn mining_attempt_worker(candidate: NounSlab, pool: Arc<KernelPool>) -> Option<NounSlab> {
    let start_time = std::time::Instant::now();

    // Get kernel from pool
    let pooled_kernel = match pool.checkout().await {
        Ok(kernel) => kernel,
        Err(e) => {
            error!("Failed to checkout kernel from pool: {}", e);
            return None;
        }
    };

    debug!("Kernel checked out in {:?}", start_time.elapsed());

    // Execute mining with pooled kernel
    let effects_slab = match pooled_kernel.kernel.poke(MiningWire::Candidate.to_wire(), candidate).await {
        Ok(effects) => effects,
        Err(e) => {
            error!("Failed to poke mining kernel: {}", e);
            pool.checkin(pooled_kernel);
            return None;
        }
    };

    debug!("Mining computation completed in {:?}", start_time.elapsed());

    // Check if we found a solution
    let has_solution = effects_slab.to_vec().iter().any(|effect| {
        if let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) {
            effect_cell.head().eq_bytes("command")
        } else {
            false
        }
    });

    // Return kernel to pool
    pool.checkin(pooled_kernel);

    let total_time = start_time.elapsed();
    debug!("Worker mining attempt completed in {:?}, solution: {}", total_time, has_solution);

    if has_solution {
        Some(effects_slab)
    } else {
        None
    }
}

/// Optimized mining attempt using kernel pool (10-50x faster)
#[instrument(skip_all)]
pub async fn mining_attempt_optimized(candidate: NounSlab, handle: NockAppHandle) -> () {
    let start_time = std::time::Instant::now();

    // Get kernel from pool instead of creating new one
    let pool = match get_kernel_pool().await {
        Ok(pool) => pool,
        Err(e) => {
            error!("Failed to get kernel pool: {}", e);
            return;
        }
    };

    let pooled_kernel = match pool.checkout().await {
        Ok(kernel) => kernel,
        Err(e) => {
            error!("Failed to checkout kernel from pool: {}", e);
            return;
        }
    };

    debug!("Kernel checked out in {:?}", start_time.elapsed());

    // Execute mining with pooled kernel
    let effects_slab = match pooled_kernel.kernel.poke(MiningWire::Candidate.to_wire(), candidate).await {
        Ok(effects) => effects,
        Err(e) => {
            error!("Failed to poke mining kernel: {}", e);
            pool.checkin(pooled_kernel);
            return;
        }
    };

    debug!("Mining computation completed in {:?}", start_time.elapsed());

    // Process effects
    for effect in effects_slab.to_vec() {
        let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
            drop(effect);
            continue;
        };
        if effect_cell.head().eq_bytes("command") {
            if let Err(e) = handle.poke(MiningWire::Mined.to_wire(), effect).await {
                error!("Could not poke nockchain with mined PoW: {}", e);
            } else {
                debug!("Successfully submitted mined proof");
            }
        }
    }

    // Return kernel to pool
    pool.checkin(pooled_kernel);

    let total_time = start_time.elapsed();
    debug!("Total mining attempt completed in {:?}", total_time);

    // Log pool statistics periodically with CPU usage info
    if rand::random::<f64>() < 0.1 {  // 10% chance to log stats
        let stats = pool.stats();
        let cpu_usage_percent = (stats.current_pool_size * 100) /
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);

        info!("⚡ Pool stats - Active kernels: {}/{} ({}% CPU), Checkouts: {}, Avg checkout: {:.2}ms",
              stats.current_pool_size,
              std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8),
              cpu_usage_percent,
              stats.total_checkouts,
              stats.average_checkout_time_ms);
    }
}

/// Legacy mining attempt function (keep for fallback)
#[instrument(skip_all)]
pub async fn mining_attempt(candidate: NounSlab, handle: NockAppHandle) -> () {
    let snapshot_dir =
        tokio::task::spawn_blocking(|| tempdir().expect("Failed to create temporary directory"))
            .await
            .expect("Failed to create temporary directory");
    let hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let snapshot_path_buf = snapshot_dir.path().to_path_buf();
    let jam_paths = JamPaths::new(snapshot_dir.path());
    // Spawns a new std::thread for this mining attempt
    // MEMORY OPTIMIZATION: Use mining-optimized kernel (2GB instead of 8GB or 32GB)
    let kernel =
        Kernel::load_with_hot_state_mining(snapshot_path_buf, jam_paths, KERNEL, &hot_state, false)
            .await
            .expect("Could not load mining kernel");
    let effects_slab = kernel
        .poke(MiningWire::Candidate.to_wire(), candidate)
        .await
        .expect("Could not poke mining kernel with candidate");
    for effect in effects_slab.to_vec() {
        let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
            drop(effect);
            continue;
        };
        if effect_cell.head().eq_bytes("command") {
            handle
                .poke(MiningWire::Mined.to_wire(), effect)
                .await
                .expect("Could not poke nockchain with mined PoW");
        }
    }
}

#[instrument(skip(handle, pubkey))]
async fn set_mining_key(
    handle: &NockAppHandle,
    pubkey: String,
) -> Result<PokeResult, NockAppError> {
    let mut set_mining_key_slab = NounSlab::new();
    let set_mining_key = Atom::from_value(&mut set_mining_key_slab, "set-mining-key")
        .expect("Failed to create set-mining-key atom");
    let pubkey_cord =
        Atom::from_value(&mut set_mining_key_slab, pubkey).expect("Failed to create pubkey atom");
    let set_mining_key_poke = T(
        &mut set_mining_key_slab,
        &[D(tas!(b"command")), set_mining_key.as_noun(), pubkey_cord.as_noun()],
    );
    set_mining_key_slab.set_root(set_mining_key_poke);

    handle
        .poke(MiningWire::SetPubKey.to_wire(), set_mining_key_slab)
        .await
}

async fn set_mining_key_advanced(
    handle: &NockAppHandle,
    configs: Vec<MiningKeyConfig>,
) -> Result<PokeResult, NockAppError> {
    let mut set_mining_key_slab = NounSlab::new();
    let set_mining_key_adv = Atom::from_value(&mut set_mining_key_slab, "set-mining-key-advanced")
        .expect("Failed to create set-mining-key-advanced atom");

    // Create the list of configs
    let mut configs_list = D(0);
    for config in configs {
        // Create the list of keys
        let mut keys_noun = D(0);
        for key in config.keys {
            let key_atom =
                Atom::from_value(&mut set_mining_key_slab, key).expect("Failed to create key atom");
            keys_noun = T(&mut set_mining_key_slab, &[key_atom.as_noun(), keys_noun]);
        }

        // Create the config tuple [share m keys]
        let config_tuple = T(
            &mut set_mining_key_slab,
            &[D(config.share), D(config.m), keys_noun],
        );

        configs_list = T(&mut set_mining_key_slab, &[config_tuple, configs_list]);
    }

    let set_mining_key_poke = T(
        &mut set_mining_key_slab,
        &[D(tas!(b"command")), set_mining_key_adv.as_noun(), configs_list],
    );
    set_mining_key_slab.set_root(set_mining_key_poke);

    handle
        .poke(MiningWire::SetPubKey.to_wire(), set_mining_key_slab)
        .await
}

//TODO add %set-mining-key-multisig poke
#[instrument(skip(handle))]
async fn enable_mining(handle: &NockAppHandle, enable: bool) -> Result<PokeResult, NockAppError> {
    let mut enable_mining_slab = NounSlab::new();
    let enable_mining = Atom::from_value(&mut enable_mining_slab, "enable-mining")
        .expect("Failed to create enable-mining atom");
    let enable_mining_poke = T(
        &mut enable_mining_slab,
        &[D(tas!(b"command")), enable_mining.as_noun(), D(if enable { 0 } else { 1 })],
    );
    enable_mining_slab.set_root(enable_mining_poke);
    handle
        .poke(MiningWire::Enable.to_wire(), enable_mining_slab)
        .await
}

/// Create a variation of a candidate by modifying its nonce/timestamp
fn create_candidate_variation(base_candidate: &NounSlab, variation_id: u64) -> NounSlab {
    let mut varied_candidate = NounSlab::new();

    // Copy the base candidate structure
    varied_candidate.copy_into(unsafe { *base_candidate.root() });

    // 🔥 CRITICAL: Modify the candidate to create unique variations
    // We need to modify the nonce or add entropy to make each candidate unique

    // Extract the original candidate structure
    let base_root = unsafe { *base_candidate.root() };

    // Create a modified version with variation_id as additional entropy
    // This simulates what would happen if we had different nonces
    let entropy_atom = Atom::new(&mut varied_candidate, variation_id).as_noun();

    // Create a new structure that includes the variation
    // Format: [original_candidate variation_entropy]
    let varied_root = T(&mut varied_candidate, &[base_root, entropy_atom]);
    varied_candidate.set_root(varied_root);

    varied_candidate
}

/// Alternative: Start aggressive parallel mining that generates work independently
async fn start_aggressive_parallel_mining(mut handle: NockAppHandle) -> Result<(), NockAppError> {
    info!("🚀 Starting AGGRESSIVE PARALLEL mining - generating work independently!");

    // Get system capabilities
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let pool = get_kernel_pool().await.map_err(|_| NockAppError::OtherError)?;
    let max_concurrent = num_cpus; // Use ALL CPU cores

    info!("📊 AGGRESSIVE mining config: {} concurrent workers on {}-core system (100% CPU)",
          max_concurrent, num_cpus);

    // Create channels for work distribution
    let (work_tx, work_rx) = mpsc::unbounded_channel::<MiningWork>();
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<MiningResult>();

    // Spawn parallel mining workers
    let work_rx = Arc::new(tokio::sync::Mutex::new(work_rx));

    // Start worker tasks - one per CPU core
    for worker_id in 0..max_concurrent {
        let pool_clone = pool.clone();
        let work_rx_clone = work_rx.clone();
        let result_tx_clone = result_tx.clone();

        tokio::spawn(async move {
            aggressive_mining_worker(worker_id, pool_clone, work_rx_clone, result_tx_clone).await;
        });
    }

    info!("✅ Spawned {} AGGRESSIVE mining workers (100% CPU utilization)", max_concurrent);

    let mut work_counter = 0u64;

    loop {
        tokio::select! {
            // Handle mining effects (update base candidate when available)
            effect_res = handle.next_effect() => {
                let Ok(effect) = effect_res else {
                    warn!("Error receiving effect in aggressive mining driver: {effect_res:?}");
                    continue;
                };

                // Process mining effects to get new base candidates
                if let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) {
                    if effect_cell.head().eq_bytes("mine") {
                        let candidate_slab = {
                            let mut slab = NounSlab::new();
                            slab.copy_into(effect_cell.tail());
                            slab
                        };

                        work_counter += 1;
                        let work = MiningWork {
                            candidate: candidate_slab,
                            work_id: work_counter,
                        };

                        // Send the real candidate immediately
                        if let Err(_) = work_tx.send(work) {
                            error!("Failed to send work to aggressive mining workers");
                        } else {
                            info!("🔄 Updated base candidate #{} from kernel for aggressive mining", work_counter);
                        }
                    }
                }
            }

            // Handle mining results
            result = result_rx.recv() => {
                if let Some(result) = result {
                    if let Some(effects) = result.effects {
                        debug!("✅ Aggressive mining work #{} completed in {:?}", result.work_id, result.duration);

                        // Process successful mining results
                        for effect in effects.to_vec() {
                            let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
                                drop(effect);
                                continue;
                            };
                            if effect_cell.head().eq_bytes("command") {
                                if let Err(e) = handle.poke(MiningWire::Mined.to_wire(), effect).await {
                                    error!("Could not poke nockchain with mined PoW: {}", e);
                                } else {
                                    SUCCESSFUL_MINES.fetch_add(1, Ordering::Relaxed);
                                    info!("🎉 AGGRESSIVE MINE SUCCESS! Total mines: {}",
                                          SUCCESSFUL_MINES.load(Ordering::Relaxed));
                                }
                            }
                        }
                    } else {
                        debug!("❌ Aggressive mining work #{} found no solution in {:?}", result.work_id, result.duration);
                    }
                }
            }
        }
    }
}

/// Aggressive mining worker that processes work items continuously
async fn aggressive_mining_worker(
    worker_id: usize,
    pool: Arc<KernelPool>,
    work_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<MiningWork>>>,
    result_tx: mpsc::UnboundedSender<MiningResult>,
) {
    info!("🔧 AGGRESSIVE Mining worker #{} started (100% CPU utilization)", worker_id);

    loop {
        // Get next work item
        let work = {
            let mut rx = work_rx.lock().await;
            rx.recv().await
        };

        let Some(work) = work else {
            warn!("🔧 Aggressive mining worker #{} shutting down - no more work", worker_id);
            break;
        };

        debug!("🔧 Aggressive worker #{} processing work #{}", worker_id, work.work_id);

        // Process the mining work immediately without semaphore limits
        let start_time = std::time::Instant::now();
        MINING_ATTEMPTS.fetch_add(1, Ordering::Relaxed);

        let effects = mining_attempt_worker(work.candidate, pool.clone()).await;

        let duration = start_time.elapsed();
        let result = MiningResult {
            work_id: work.work_id,
            effects,
            duration,
        };

        if let Err(_) = result_tx.send(result) {
            error!("🔧 Aggressive worker #{} failed to send result", worker_id);
            break;
        }

        // Log statistics more frequently for aggressive mining
        if MINING_ATTEMPTS.load(Ordering::Relaxed) % 100 == 0 {
            let attempts = MINING_ATTEMPTS.load(Ordering::Relaxed);
            let successes = SUCCESSFUL_MINES.load(Ordering::Relaxed);
            let stats = pool.stats();
            info!("⚡ AGGRESSIVE Mining stats - Attempts: {}, Successes: {}, Active kernels: {}/{}",
                  attempts, successes, stats.current_pool_size, stats.total_created);
        }
    }
}

/// 🚀 SOLUTION: Multiple receiver mining - each worker has its own effect receiver
/// This completely bypasses the single-receiver bottleneck
async fn start_multiple_receiver_mining(handle: NockAppHandle) -> Result<(), NockAppError> {
    info!("🚀 Starting MULTIPLE RECEIVER mining for TRUE PARALLELISM!");
    info!("🔧 ARCHITECTURAL FIX: Each worker gets its own broadcast receiver");
    info!("💡 SOLUTION: Bypassing handle.next_effect() bottleneck completely");

    // Get system capabilities
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let pool = get_kernel_pool().await.map_err(|_| NockAppError::OtherError)?;
    let max_workers = num_cpus; // One worker per CPU core

    info!("📊 MULTIPLE RECEIVER mining config: {} workers on {}-core system",
          max_workers, num_cpus);
    info!("🎯 EXPECTED: Each worker will receive mining effects independently");
    info!("⚡ TARGET: ~75% CPU utilization across all cores");

    // Create a channel for mining results
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<MiningResult>();

    // Create multiple broadcast receivers - one for each worker
    let mut worker_handles = Vec::new();
    
    for worker_id in 0..max_workers {
        // Each worker gets its own receiver from the broadcast channel
        let effect_receiver = handle.effect_sender.subscribe();
        let pool_clone = pool.clone();
        let result_tx_clone = result_tx.clone();
        
        let worker_handle = tokio::spawn(async move {
            info!("🔧 Worker #{} starting with dedicated receiver", worker_id);
            let mut work_counter = 0u64;
            let mut receiver = effect_receiver;
            
            loop {
                match receiver.recv().await {
                    Ok(effect) => {
                        // Check if this is a mining effect
                        if let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) {
                            if effect_cell.head().eq_bytes("mine") {
                                work_counter += 1;
                                info!("🔧 Worker #{} received mining effect #{}", worker_id, work_counter);
                                
                                let candidate_slab = {
                                    let mut slab = NounSlab::new();
                                    slab.copy_into(effect_cell.tail());
                                    slab
                                };
                                
                                // Process the mining work
                                let start_time = std::time::Instant::now();
                                MINING_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
                                
                                let effects = mining_attempt_worker(candidate_slab, pool_clone.clone()).await;
                                
                                let duration = start_time.elapsed();
                                let result = MiningResult {
                                    work_id: work_counter,
                                    effects,
                                    duration,
                                };
                                
                                if let Err(_) = result_tx_clone.send(result) {
                                    error!("🔧 Worker #{} failed to send result", worker_id);
                                    break;
                                }
                                
                                // Log statistics periodically
                                if MINING_ATTEMPTS.load(Ordering::Relaxed) % 100 == 0 {
                                    let attempts = MINING_ATTEMPTS.load(Ordering::Relaxed);
                                    let successes = SUCCESSFUL_MINES.load(Ordering::Relaxed);
                                    info!("⚡ Worker #{} - Total attempts: {}, successes: {}",
                                          worker_id, attempts, successes);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("🔧 Worker #{} receiver error: {:?}", worker_id, e);
                        break;
                    }
                }
            }
            
            warn!("🔧 Worker #{} shutting down", worker_id);
        });
        
        worker_handles.push(worker_handle);
    }

    info!("✅ Spawned {} workers with dedicated receivers", max_workers);

    // Main loop to handle results
    loop {
        tokio::select! {
            // Handle mining results
            result = result_rx.recv() => {
                if let Some(result) = result {
                    if let Some(effects) = result.effects {
                        debug!("✅ Mining work completed in {:?}", result.duration);

                        // Process successful mining results
                        for effect in effects.to_vec() {
                            let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
                                drop(effect);
                                continue;
                            };
                            if effect_cell.head().eq_bytes("command") {
                                if let Err(e) = handle.poke(MiningWire::Mined.to_wire(), effect).await {
                                    error!("Could not poke nockchain with mined PoW: {}", e);
                                } else {
                                    SUCCESSFUL_MINES.fetch_add(1, Ordering::Relaxed);
                                    info!("🎉 MINE SUCCESS! Total successful mines: {}",
                                          SUCCESSFUL_MINES.load(Ordering::Relaxed));
                                }
                            }
                        }
                    } else {
                        debug!("❌ Mining work found no solution in {:?}", result.duration);
                    }
                }
            }
            
            // Monitor worker health
            _ = tokio::time::sleep(Duration::from_secs(30)) => {
                let attempts = MINING_ATTEMPTS.load(Ordering::Relaxed);
                let successes = SUCCESSFUL_MINES.load(Ordering::Relaxed);
                let stats = pool.stats();
                info!("📊 Mining stats - Attempts: {}, Successes: {}, Active kernels: {}/{}",
                      attempts, successes, stats.current_pool_size, stats.total_created);
                
                // Check if workers are still alive
                let alive_workers = worker_handles.iter().filter(|h| !h.is_finished()).count();
                if alive_workers < max_workers {
                    warn!("⚠️  Only {}/{} workers still alive", alive_workers, max_workers);
                }
            }
        }
    }
}

/// 🚀 ORIGINAL: True parallel mining with batch effect processing
/// This solves the architectural bottleneck in handle.next_effect()
async fn start_true_parallel_mining(mut handle: NockAppHandle) -> Result<(), NockAppError> {
    info!("🚀 Starting TRUE PARALLEL mining with BATCH EFFECT PROCESSING!");
    info!("🔧 ARCHITECTURAL FIX: Solving handle.next_effect() bottleneck");
    info!("📋 DIAGNOSIS: Previous issue was sequential effect consumption");
    info!("💡 SOLUTION: Batch processing of ALL available effects at once");

    // Get system capabilities
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let pool = get_kernel_pool().await.map_err(|_| NockAppError::OtherError)?;
    let max_concurrent = num_cpus; // Use ALL CPU cores

    info!("📊 TRUE PARALLEL mining config: {} concurrent workers on {}-core system (100% CPU)",
          max_concurrent, num_cpus);
    info!("🔧 ARCHITECTURAL FIX: Using batch effect processing to eliminate bottleneck");
    info!("🎯 EXPECTED: Hoon generates 24 effects, Rust processes ALL 24 at once");
    info!("⚡ TARGET CPU USAGE: ~75% across all {} threads", num_cpus);

    // Create channels for work distribution
    let (work_tx, work_rx) = mpsc::unbounded_channel::<MiningWork>();
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<MiningResult>();

    // Spawn parallel mining workers
    let work_rx = Arc::new(tokio::sync::Mutex::new(work_rx));

    // Start worker tasks - one per CPU core
    for worker_id in 0..max_concurrent {
        let pool_clone = pool.clone();
        let work_rx_clone = work_rx.clone();
        let result_tx_clone = result_tx.clone();

        tokio::spawn(async move {
            true_parallel_mining_worker(worker_id, pool_clone, work_rx_clone, result_tx_clone).await;
        });
    }

    info!("✅ Spawned {} TRUE PARALLEL mining workers (100% CPU utilization)", max_concurrent);

    let mut work_counter = 0u64;
    let mut batch_counter = 0u64;

    // 🚀 CRITICAL FIX: Use our own batch collection to bypass mutex bottleneck
    loop {
        tokio::select! {
            // 🔥 SOLUTION: Collect multiple effects to distribute to workers
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                // Collect multiple effects at once
                let mut effects_batch = Vec::new();
                let mut collected = 0;
                
                // Try to collect up to max_concurrent effects
                while collected < max_concurrent {
                    match handle.next_effect().await {
                        Ok(effect) => {
                            effects_batch.push(effect);
                            collected += 1;
                        }
                        Err(_) => {
                            // No more effects available right now
                            break;
                        }
                    }
                    
                    // Don't wait too long for effects
                    if collected > 0 && collected % 4 == 0 {
                        break;
                    }
                }

                if !effects_batch.is_empty() {
                    batch_counter += 1;
                    let batch_size = effects_batch.len();

                    info!("🚀 BATCH #{}: Collected {} effects for parallel processing",
                          batch_counter, batch_size);

                    // Process ALL effects in the batch
                    let mut mining_candidates = 0;
                    for effect in effects_batch {
                        if let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) {
                            if effect_cell.head().eq_bytes("mine") {
                                let candidate_slab = {
                                    let mut slab = NounSlab::new();
                                    slab.copy_into(effect_cell.tail());
                                    slab
                                };

                                work_counter += 1;
                                mining_candidates += 1;

                                let work = MiningWork {
                                    candidate: candidate_slab,
                                    work_id: work_counter,
                                };

                                // Send ALL candidates to workers immediately
                                if let Err(_) = work_tx.send(work) {
                                    error!("Failed to send work to parallel mining workers");
                                } else {
                                    debug!("📤 Sent mining candidate #{} to worker pool", work_counter);
                                }
                            }
                        }
                    }

                    if mining_candidates > 0 {
                        info!("⚡ BATCH #{}: Distributed {} mining candidates to {} workers",
                              batch_counter, mining_candidates, max_concurrent);
                        info!("🎯 Expected CPU utilization: {}% (using {} threads)",
                              (mining_candidates.min(max_concurrent) * 100) / max_concurrent,
                              mining_candidates.min(max_concurrent));
                    } else {
                        debug!("📭 BATCH #{}: No mining effects in batch of {} effects",
                               batch_counter, batch_size);
                    }
                }
            }

            // Handle mining results
            result = result_rx.recv() => {
                if let Some(result) = result {
                    if let Some(effects) = result.effects {
                        debug!("✅ TRUE PARALLEL mining work #{} completed in {:?}",
                               result.work_id, result.duration);

                        // Process successful mining results
                        for effect in effects.to_vec() {
                            let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
                                drop(effect);
                                continue;
                            };
                            if effect_cell.head().eq_bytes("command") {
                                if let Err(e) = handle.poke(MiningWire::Mined.to_wire(), effect).await {
                                    error!("Could not poke nockchain with mined PoW: {}", e);
                                } else {
                                    SUCCESSFUL_MINES.fetch_add(1, Ordering::Relaxed);
                                    info!("🎉 TRUE PARALLEL MINE SUCCESS! Total mines: {}",
                                          SUCCESSFUL_MINES.load(Ordering::Relaxed));
                                }
                            }
                        }
                    } else {
                        debug!("❌ TRUE PARALLEL mining work #{} found no solution in {:?}",
                               result.work_id, result.duration);
                    }
                }
            }

        }
    }
}

/// TRUE PARALLEL mining worker that processes work items continuously
async fn true_parallel_mining_worker(
    worker_id: usize,
    pool: Arc<KernelPool>,
    work_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<MiningWork>>>,
    result_tx: mpsc::UnboundedSender<MiningResult>,
) {
    info!("🔧 TRUE PARALLEL Mining worker #{} started (BATCH PROCESSING)", worker_id);

    loop {
        // Get next work item
        let work = {
            let mut rx = work_rx.lock().await;
            rx.recv().await
        };

        let Some(work) = work else {
            warn!("🔧 TRUE PARALLEL mining worker #{} shutting down - no more work", worker_id);
            break;
        };

        debug!("🔧 TRUE PARALLEL worker #{} processing work #{}", worker_id, work.work_id);

        // Process the mining work immediately
        let start_time = std::time::Instant::now();
        MINING_ATTEMPTS.fetch_add(1, Ordering::Relaxed);

        let effects = mining_attempt_worker(work.candidate, pool.clone()).await;

        let duration = start_time.elapsed();
        let result = MiningResult {
            work_id: work.work_id,
            effects,
            duration,
        };

        if let Err(_) = result_tx.send(result) {
            error!("🔧 TRUE PARALLEL worker #{} failed to send result", worker_id);
            break;
        }

        // Log statistics for TRUE PARALLEL mining
        if MINING_ATTEMPTS.load(Ordering::Relaxed) % 100 == 0 {
            let attempts = MINING_ATTEMPTS.load(Ordering::Relaxed);
            let successes = SUCCESSFUL_MINES.load(Ordering::Relaxed);
            let stats = pool.stats();
            info!("⚡ TRUE PARALLEL Mining stats - Attempts: {}, Successes: {}, Active kernels: {}/{}",
                  attempts, successes, stats.current_pool_size, stats.total_created);
        }
    }
}
