use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use kernels::miner::KERNEL;
use nockapp::kernel::checkpoint::JamPaths;
use nockapp::kernel::form::Kernel;
use tempfile::{TempDir, tempdir};
use tracing::{info, warn, debug, instrument};
use tokio::sync::Semaphore;

/// A reusable mining kernel with its associated temporary directory
pub struct PooledKernel {
    pub kernel: Kernel,
    pub temp_dir: TempDir,
    pub created_at: Instant,
    pub usage_count: u64,
}

impl PooledKernel {
    /// Create a new pooled kernel with hot state
    #[instrument(skip_all)]
    async fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let temp_dir = tokio::task::spawn_blocking(|| {
            tempdir().map_err(|e| format!("Failed to create temporary directory: {}", e))
        })
        .await??;

        let hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
        let snapshot_path_buf = temp_dir.path().to_path_buf();
        let jam_paths = JamPaths::new(temp_dir.path());

        // MEMORY OPTIMIZATION: Use mining-optimized kernel (2GB instead of 8GB or 32GB)
        // Mining doesn't need huge stack size - this reduces memory usage by 75%
        let kernel = Kernel::load_with_hot_state_mining(
            snapshot_path_buf,
            jam_paths,
            KERNEL,
            &hot_state,
            false,
        )
        .await
        .map_err(|e| format!("Could not load mining kernel: {}", e))?;

        Ok(PooledKernel {
            kernel,
            temp_dir,
            created_at: Instant::now(),
            usage_count: 0,
        })
    }

    /// Reset kernel state for reuse (if needed)
    pub fn reset(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.usage_count += 1;
        // Add any necessary cleanup/reset logic here
        debug!("Kernel reset, usage count: {}", self.usage_count);
        Ok(())
    }

    /// Check if kernel should be replaced (due to age or usage)
    pub fn should_refresh(&self, max_age: Duration, max_usage: u64) -> bool {
        self.created_at.elapsed() > max_age || self.usage_count > max_usage
    }
}

/// Configuration for the kernel pool
#[derive(Debug, Clone)]
pub struct KernelPoolConfig {
    /// Minimum number of kernels to keep in pool
    pub min_size: usize,
    /// Maximum number of kernels to keep in pool
    pub max_size: usize,
    /// Maximum age before refreshing a kernel
    pub max_kernel_age: Duration,
    /// Maximum usage count before refreshing a kernel
    pub max_kernel_usage: u64,
    /// Timeout when waiting for an available kernel
    pub checkout_timeout: Duration,
}

impl Default for KernelPoolConfig {
    fn default() -> Self {
        Self {
            min_size: 4,
            max_size: 8,
            max_kernel_age: Duration::from_secs(300), // 5 minutes
            max_kernel_usage: 100,
            checkout_timeout: Duration::from_secs(10),
        }
    }
}

/// Thread-safe pool of mining kernels for high-performance mining
pub struct KernelPool {
    pool: Arc<Mutex<VecDeque<PooledKernel>>>,
    semaphore: Arc<Semaphore>,
    config: KernelPoolConfig,
    stats: Arc<Mutex<PoolStats>>,
}

#[derive(Debug, Default)]
pub struct PoolStats {
    pub total_checkouts: u64,
    pub total_checkins: u64,
    pub total_created: u64,
    pub total_refreshed: u64,
    pub current_pool_size: usize,
    pub average_checkout_time_ms: f64,
}

impl KernelPool {
    /// Create a new kernel pool with the given configuration
    #[instrument(skip_all)]
    pub async fn new(config: KernelPoolConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        info!("Creating kernel pool with config: {:?}", config);

        let pool = Arc::new(Mutex::new(VecDeque::new()));
        let semaphore = Arc::new(Semaphore::new(config.max_size));
        let stats = Arc::new(Mutex::new(PoolStats::default()));

        let kernel_pool = KernelPool {
            pool: pool.clone(),
            semaphore,
            config: config.clone(),
            stats: stats.clone(),
        };

        // Pre-populate the pool with minimum number of kernels
        info!("Pre-populating pool with {} kernels...", config.min_size);
        for i in 0..config.min_size {
            debug!("Creating initial kernel {}/{}", i + 1, config.min_size);
            let kernel = PooledKernel::new().await?;

            {
                let mut pool_guard = pool.lock().unwrap();
                pool_guard.push_back(kernel);
                let mut stats_guard = stats.lock().unwrap();
                stats_guard.total_created += 1;
                stats_guard.current_pool_size += 1;
            }
        }

        info!("Kernel pool initialized successfully with {} kernels", config.min_size);
        Ok(kernel_pool)
    }

    /// Checkout a kernel from the pool (blocks until one is available)
    #[instrument(skip_all)]
    pub async fn checkout(&self) -> Result<PooledKernel, Box<dyn std::error::Error + Send + Sync>> {
        let start_time = Instant::now();

        // Wait for a semaphore permit (enforces max pool size)
        let _permit = tokio::time::timeout(
            self.config.checkout_timeout,
            self.semaphore.acquire()
        )
        .await
        .map_err(|_| "Timeout waiting for available kernel")?
        .map_err(|e| format!("Semaphore error: {}", e))?;

        // Try to get an existing kernel from the pool
        let mut kernel = {
            let mut pool = self.pool.lock().unwrap();
            pool.pop_front()
        };

        // If no kernel available or kernel needs refresh, create a new one
        if kernel.is_none() || kernel.as_ref().unwrap().should_refresh(
            self.config.max_kernel_age,
            self.config.max_kernel_usage
        ) {
            if let Some(old_kernel) = kernel {
                debug!("Refreshing old kernel (age: {:?}, usage: {})",
                      old_kernel.created_at.elapsed(), old_kernel.usage_count);

                let mut stats = self.stats.lock().unwrap();
                stats.total_refreshed += 1;
            }

            debug!("Creating new kernel for checkout");
            kernel = Some(PooledKernel::new().await?);

            let mut stats = self.stats.lock().unwrap();
            stats.total_created += 1;
        }

        let mut kernel = kernel.unwrap();
        kernel.reset()?;

        // Update statistics
        {
            let mut stats = self.stats.lock().unwrap();
            stats.total_checkouts += 1;
            stats.current_pool_size = self.pool.lock().unwrap().len();

            let checkout_time_ms = start_time.elapsed().as_millis() as f64;
            stats.average_checkout_time_ms =
                (stats.average_checkout_time_ms * (stats.total_checkouts - 1) as f64 + checkout_time_ms)
                / stats.total_checkouts as f64;
        }

        debug!("Kernel checked out in {:?}", start_time.elapsed());
        Ok(kernel)
    }

    /// Return a kernel to the pool
    #[instrument(skip_all)]
    pub fn checkin(&self, kernel: PooledKernel) {
        let start_time = Instant::now();

        // Only return to pool if it's still healthy and under limits
        if !kernel.should_refresh(self.config.max_kernel_age, self.config.max_kernel_usage) {
            let mut pool = self.pool.lock().unwrap();
            if pool.len() < self.config.max_size {
                pool.push_back(kernel);
                debug!("Kernel returned to pool");
            } else {
                debug!("Pool full, discarding kernel");
            }
        } else {
            debug!("Kernel too old/used, discarding");
        }

        // Update statistics
        {
            let mut stats = self.stats.lock().unwrap();
            stats.total_checkins += 1;
            stats.current_pool_size = self.pool.lock().unwrap().len();
        }

        debug!("Kernel checked in in {:?}", start_time.elapsed());
    }

    /// Get pool statistics
    pub fn stats(&self) -> PoolStats {
        let stats = self.stats.lock().unwrap();
        let current_pool_size = self.pool.lock().unwrap().len();

        PoolStats {
            current_pool_size,
            ..*stats
        }
    }

    /// Force refresh of all kernels in the pool
    #[instrument(skip_all)]
    pub async fn refresh_all(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Refreshing all kernels in pool");

        // Clear the current pool
        {
            let mut pool = self.pool.lock().unwrap();
            pool.clear();
        }

        // Recreate minimum number of fresh kernels
        for i in 0..self.config.min_size {
            debug!("Creating refreshed kernel {}/{}", i + 1, self.config.min_size);
            let kernel = PooledKernel::new().await?;

            let mut pool = self.pool.lock().unwrap();
            pool.push_back(kernel);
        }

        let mut stats = self.stats.lock().unwrap();
        stats.total_refreshed += self.config.min_size as u64;
        stats.current_pool_size = self.config.min_size;

        info!("Pool refresh completed");
        Ok(())
    }
}

/// RAII wrapper for automatic kernel checkin
pub struct KernelGuard {
    kernel: Option<PooledKernel>,
    pool: Arc<KernelPool>,
}

impl KernelGuard {
    pub fn new(kernel: PooledKernel, pool: Arc<KernelPool>) -> Self {
        Self {
            kernel: Some(kernel),
            pool,
        }
    }

    /// Get a reference to the kernel
    pub fn kernel(&self) -> &Kernel {
        &self.kernel.as_ref().unwrap().kernel
    }

    /// Get a mutable reference to the kernel
    pub fn kernel_mut(&mut self) -> &mut Kernel {
        &mut self.kernel.as_mut().unwrap().kernel
    }
}

impl Drop for KernelGuard {
    fn drop(&mut self) {
        if let Some(kernel) = self.kernel.take() {
            self.pool.checkin(kernel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_kernel_pool_creation() {
        let config = KernelPoolConfig {
            min_size: 2,
            max_size: 4,
            ..Default::default()
        };

        let pool = KernelPool::new(config).await.unwrap();
        let stats = pool.stats();

        assert_eq!(stats.current_pool_size, 2);
        assert_eq!(stats.total_created, 2);
    }

    #[tokio::test]
    async fn test_kernel_checkout_checkin() {
        let config = KernelPoolConfig {
            min_size: 1,
            max_size: 2,
            ..Default::default()
        };

        let pool = Arc::new(KernelPool::new(config).await.unwrap());

        // Checkout kernel
        let kernel = pool.checkout().await.unwrap();
        let stats = pool.stats();
        assert_eq!(stats.total_checkouts, 1);
        assert_eq!(stats.current_pool_size, 0);

        // Checkin kernel
        pool.checkin(kernel);
        let stats = pool.stats();
        assert_eq!(stats.total_checkins, 1);
        assert_eq!(stats.current_pool_size, 1);
    }

    #[tokio::test]
    async fn test_kernel_guard() {
        let config = KernelPoolConfig {
            min_size: 1,
            max_size: 2,
            ..Default::default()
        };

        let pool = Arc::new(KernelPool::new(config).await.unwrap());

        {
            let kernel = pool.checkout().await.unwrap();
            let _guard = KernelGuard::new(kernel, pool.clone());
            // Guard automatically returns kernel on drop
        }

        let stats = pool.stats();
        assert_eq!(stats.total_checkins, 1);
    }
}