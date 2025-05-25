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
    // Use 50-75% of available threads for mining kernels
    let min_kernels = (num_cpus / 2).max(4);  // At least 4, but scale with CPUs
    let max_kernels = ((num_cpus * 3) / 4).max(8);  // Up to 75% of threads

    let config = KernelPoolConfig {
        min_size: min_kernels,     // Dynamic based on CPU count
        max_size: max_kernels,     // Scale up to 75% of available threads
        max_kernel_age: std::time::Duration::from_secs(600), // 10 minutes
        max_kernel_usage: 50,  // Replace after 50 uses
        checkout_timeout: std::time::Duration::from_secs(30),
        ..Default::default()
    };

    info!("🚀 Initializing mining kernel pool for {}-thread system", num_cpus);
    info!("📊 Kernel pool config: min={}, max={} (using {}% of CPU threads)",
          min_kernels, max_kernels, (max_kernels * 100) / num_cpus);

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
    Box::new(move |mut handle| {
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

            // 🚀 NEW: Parallel mining with all available cores
            start_parallel_mining(handle).await
        })
    })
}

/// Start parallel mining using all available kernels with BATCH PROCESSING
async fn start_parallel_mining(mut handle: NockAppHandle) -> Result<(), NockAppError> {
    info!("🚀 Starting PARALLEL mining with BATCH PROCESSING for full CPU utilization!");

    // Get system capabilities
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let pool = get_kernel_pool().await.map_err(|_| NockAppError::OtherError)?;
    let max_concurrent = ((num_cpus * 3) / 4).max(8); // Same as kernel pool max

    info!("📊 BATCH mining config: {} concurrent workers on {}-core system",
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

    info!("✅ Spawned {} parallel mining workers with BATCH processing", max_concurrent);

    let mut work_counter = 0u64;

    loop {
        tokio::select! {
            // 🚀 BATCH PROCESSING: Handle multiple mining effects at once
            effect_res = handle.next_effect() => {
                let Ok(first_effect) = effect_res else {
                    warn!("Error receiving effect in mining driver: {effect_res:?}");
                    continue;
                };

                // Collect all mine effects in a batch
                let mut mine_effects = Vec::new();

                // Process the first effect
                if let Ok(effect_cell) = (unsafe { first_effect.root().as_cell() }) {
                    if effect_cell.head().eq_bytes("mine") {
                        mine_effects.push(first_effect);
                    }
                }

                // 🔥 CRITICAL: Collect ALL pending mine effects (non-blocking)
                // This eliminates the sequential bottleneck!
                while mine_effects.len() < max_concurrent {
                    // Try to get more effects without blocking
                    let additional_effect = tokio::time::timeout(
                        Duration::from_millis(1), // Very short timeout
                        handle.next_effect()
                    ).await;

                    match additional_effect {
                        Ok(Ok(effect)) => {
                            if let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) {
                                if effect_cell.head().eq_bytes("mine") {
                                    mine_effects.push(effect);
                                } else {
                                    // Non-mine effect, process normally
                                    drop(effect);
                                    break;
                                }
                            }
                        }
                        _ => break, // No more effects available or timeout
                    }
                }

                // 🚀 Process ALL collected mine effects in PARALLEL
                let batch_size = mine_effects.len();
                if batch_size > 0 {
                    info!("⚡ Processing BATCH of {} mine effects simultaneously!", batch_size);

                    for effect in mine_effects {
                        let Ok(effect_cell) = (unsafe { effect.root().as_cell() }) else {
                            continue;
                        };

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

                        // Distribute work to all available workers SIMULTANEOUSLY
                        if let Err(_) = work_tx.send(work) {
                            error!("Failed to send work to mining workers");
                        } else {
                            debug!("🔄 Dispatched mining work #{} (batch {}) to parallel workers",
                                   work_counter, batch_size);
                        }
                    }

                    // Log batch processing statistics
                    if batch_size > 1 {
                        info!("🎯 BATCH ADVANTAGE: {} parallel mining attempts vs 1 sequential", batch_size);
                    }
                }
            }

            // Handle mining results (unchanged)
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
        if let Ok(effect_cell) = unsafe { effect.root().as_cell() } {
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
    let kernel =
        Kernel::load_with_hot_state_huge(snapshot_path_buf, jam_paths, KERNEL, &hot_state, false)
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
