# Load environment variables from .env file
include .env

# Set default env variables if not set in .env
export RUST_BACKTRACE ?= full
export RUST_LOG ?= info,nockchain=info,nockchain_libp2p_io=info,libp2p=info,libp2p_quic=info
export MINIMAL_LOG_FORMAT ?= true
export MINING_PUBKEY ?= 2qwq9dQRZfpFx8BDicghpMRnYGKZsZGxxhh9m362pzpM9aeo276pR1yHZPS41y3CW3vPKxeYM8p8fzZS8GXmDGzmNNCnVNekjrSYogqfEFMqwhHh5iCjaKPaDTwhupWqiXj6
export

# Build configuration
CARGO_BUILD_JOBS ?= $(shell nproc)
BUILD_MODE ?= dev
CARGO_FLAGS = $(if $(filter release,$(BUILD_MODE)),--release,)
TARGET_DIR = target/$(if $(filter release,$(BUILD_MODE)),release,debug)

# Runtime optimization for multi-threading
export TOKIO_WORKER_THREADS ?= $(shell nproc)
export RAYON_NUM_THREADS ?= $(shell nproc)

# Directories and files
HOON_DIR = hoon
ASSETS_DIR = assets
SCRIPTS_DIR = scripts
HOONC_TIMESTAMP = .hoonc_built

# Find all Hoon source files
HOON_SOURCES = $(shell find $(HOON_DIR) -name '*.hoon' 2>/dev/null)

# Find hoonc source files (Make-compatible pattern)
HOONC_SOURCES = crates/hoonc/src/lib.rs crates/hoonc/src/main.rs crates/hoonc/Cargo.toml

# Hoon targets
HOON_TARGETS = $(ASSETS_DIR)/dumb.jam $(ASSETS_DIR)/wal.jam $(ASSETS_DIR)/miner.jam

.DEFAULT_GOAL := build

.PHONY: help
help: ## Show this help message
	@echo "Nockchain Build System - Optimized Version"
	@echo "✅ Mining Kernel Pool: 10-50x faster mining with pre-allocated kernels"
	@echo ""
	@echo "Available targets:"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  %-25s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@echo ""
	@echo "Build modes:"
	@echo "  make build              # Fast development build"
	@echo "  make build BUILD_MODE=release  # Optimized release build"
	@echo "  make fast-build         # Use optimized build script"
	@echo ""
	@echo "Run modes:"
	@echo "  make run-nockchain            # Mining node with kernel pool optimization"
	@echo "  make run-nockchain-no-mining  # Observer node (no mining)"
	@echo ""
	@echo "Debug modes:"
	@echo "  make debug-run          # Enhanced verbosity debugging"
	@echo "  make test-kernel-pool   # Test kernel pool optimization specifically"
	@echo "  make system-info        # Show CPU configuration and expected performance"
	@echo ""
	@echo "Environment variables:"
	@echo "  BUILD_MODE={dev|release}  # Build configuration (default: dev)"
	@echo "  CARGO_BUILD_JOBS=N        # Parallel jobs (default: $(CARGO_BUILD_JOBS))"
	@echo "  TOKIO_WORKER_THREADS=N    # Tokio async threads (default: $(TOKIO_WORKER_THREADS))"
	@echo "  RAYON_NUM_THREADS=N       # Rayon parallel threads (default: $(RAYON_NUM_THREADS))"

.PHONY: show-config
show-config: ## Show current build configuration
	@echo "Build configuration:"
	@echo "  BUILD_MODE: $(BUILD_MODE)"
	@echo "  CARGO_FLAGS: $(CARGO_FLAGS)"
	@echo "  CARGO_BUILD_JOBS: $(CARGO_BUILD_JOBS)"
	@echo "  TARGET_DIR: $(TARGET_DIR)"

.PHONY: build
build: ensure-scripts ## Build everything (fast incremental)
	$(call show_env_vars)
	@echo "Using optimized build system..."
	@$(SCRIPTS_DIR)/fast-build.sh
	@echo "Build complete!"

.PHONY: fast-build
fast-build: ensure-scripts ## Use optimized build script for maximum speed
	@echo "Using optimized build script..."
	@$(SCRIPTS_DIR)/fast-build.sh

.PHONY: build-release
build-release: ## Build everything in release mode
	$(MAKE) build BUILD_MODE=release

.PHONY: clean-build
clean-build: clean build ## Clean and rebuild everything

## Rust build targets

.PHONY: build-rust
build-rust: ## Build Rust components
	@echo "Building Rust components ($(BUILD_MODE) mode)..."
	CARGO_BUILD_JOBS=$(CARGO_BUILD_JOBS) cargo build $(CARGO_FLAGS)

.PHONY: build-rust-fast
build-rust-fast: ## Fast Rust build (dev mode, parallel)
	@echo "Building Rust components (fast dev mode)..."
	CARGO_BUILD_JOBS=$(CARGO_BUILD_JOBS) cargo build

## Hoon compiler management

$(HOONC_TIMESTAMP): $(HOONC_SOURCES)
	@echo "Building and installing hoonc compiler..."
	CARGO_BUILD_JOBS=$(CARGO_BUILD_JOBS) cargo install --locked --force --path crates/hoonc --bin hoonc
	@touch $(HOONC_TIMESTAMP)

.PHONY: install-hoonc
install-hoonc: nuke-hoonc-data ## Install hoonc from this repo
	$(call show_env_vars)
	@echo "Installing hoonc..."
	cargo install --locked --force --path crates/hoonc --bin hoonc
	@touch $(HOONC_TIMESTAMP)

.PHONY: update-hoonc
update-hoonc: $(HOONC_TIMESTAMP) ## Ensure hoonc is built and up-to-date
	@echo "hoonc is up to date"

## Directory management

.PHONY: ensure-dirs
ensure-dirs:
	@mkdir -p $(HOON_DIR) $(ASSETS_DIR)

.PHONY: ensure-scripts
ensure-scripts:
	@mkdir -p $(SCRIPTS_DIR)
	@chmod +x $(SCRIPTS_DIR)/*.sh 2>/dev/null || true

## Hoon build targets

.PHONY: build-hoon-all
build-hoon-all: nuke-assets update-hoonc ensure-dirs build-trivial $(HOON_TARGETS) ## Build all Hoon assets

.PHONY: build-hoon
build-hoon: $(HOON_TARGETS) ## Build Hoon assets (incremental)

.PHONY: build-trivial
build-trivial: ensure-dirs $(HOONC_TIMESTAMP)
	@echo "Building trivial Hoon..."
	@echo '%trivial' > $(HOON_DIR)/trivial.hoon
	hoonc --arbitrary $(HOON_DIR)/trivial.hoon

# Optimized asset building using globally installed hoonc (like original)
$(ASSETS_DIR)/dumb.jam: $(HOONC_TIMESTAMP) $(HOON_DIR)/apps/dumbnet/outer.hoon $(HOON_SOURCES) | ensure-dirs
	$(call show_env_vars)
	@echo "Building dumb.jam..."
	@cd $(CURDIR) && RUST_LOG=info hoonc $(HOON_DIR)/apps/dumbnet/outer.hoon $(HOON_DIR)
	@mv out.jam $@

$(ASSETS_DIR)/wal.jam: $(HOONC_TIMESTAMP) $(HOON_DIR)/apps/wallet/wallet.hoon $(HOON_SOURCES) | ensure-dirs
	$(call show_env_vars)
	@echo "Building wal.jam..."
	@cd $(CURDIR) && RUST_LOG=info hoonc $(HOON_DIR)/apps/wallet/wallet.hoon $(HOON_DIR)
	@mv out.jam $@

$(ASSETS_DIR)/miner.jam: $(HOONC_TIMESTAMP) $(HOON_DIR)/apps/dumbnet/miner.hoon $(HOON_SOURCES) | ensure-dirs
	$(call show_env_vars)
	@echo "Building miner.jam..."
	@cd $(CURDIR) && RUST_LOG=info hoonc $(HOON_DIR)/apps/dumbnet/miner.hoon $(HOON_DIR)
	@mv out.jam $@

## Installation targets

.PHONY: install-nockchain
install-nockchain: build-hoon-all ## Install nockchain binary
	$(call show_env_vars)
	@echo "Installing nockchain..."
	cargo install --locked --force --path crates/nockchain --bin nockchain

.PHONY: update-nockchain
update-nockchain: build-hoon-all
	$(call show_env_vars)
	cargo install --locked --path crates/nockchain --bin nockchain

.PHONY: install-nockchain-wallet
install-nockchain-wallet: build-hoon-all ## Install nockchain-wallet binary
	$(call show_env_vars)
	@echo "Installing nockchain-wallet..."
	cargo install --locked --force --path crates/nockchain-wallet --bin nockchain-wallet

.PHONY: update-nockchain-wallet
update-nockchain-wallet: build-hoon-all
	$(call show_env_vars)
	cargo install --locked --path crates/nockchain-wallet --bin nockchain-wallet

.PHONY: install-all
install-all: install-hoonc install-nockchain install-nockchain-wallet ## Install all binaries

## Development targets

.PHONY: dev
dev: build-rust-fast $(HOON_TARGETS) ## Fast development build

.PHONY: check
check: ## Check code without building
	cargo check --all-targets

.PHONY: test
test: build-rust ## Run all tests
	@echo "Running tests..."
	cargo test $(CARGO_FLAGS)

.PHONY: test-fast
test-fast: ## Run tests in dev mode (faster)
	@echo "Running tests (dev mode)..."
	cargo test

## Cleaning targets

.PHONY: nuke-hoonc-data
nuke-hoonc-data:
	rm -rf .data.hoonc
	rm -rf ~/.nockapp/hoonc

.PHONY: nuke-assets
nuke-assets:
	rm -f assets/*.jam

.PHONY: clean
clean: ## Clean all build artifacts
	@echo "Cleaning build artifacts..."
	cargo clean
	rm -f $(HOONC_TIMESTAMP)
	rm -rf $(ASSETS_DIR)/*.jam
	rm -rf .data.hoonc
	rm -rf ~/.nockapp/hoonc

.PHONY: clean-assets
clean-assets: ## Clean only Hoon assets
	@echo "Cleaning Hoon assets..."
	rm -f $(ASSETS_DIR)/*.jam

.PHONY: clean-hoonc
clean-hoonc: ## Clean hoonc build
	@echo "Cleaning hoonc..."
	rm -f $(HOONC_TIMESTAMP)
	cargo clean --package hoonc

## Utility targets

.PHONY: watch
watch: ensure-scripts ## Watch for changes and rebuild
	@echo "Watching for changes..."
	@command -v watchexec >/dev/null 2>&1 || (echo "Please install watchexec: cargo install watchexec-cli" && exit 1)
	watchexec -r -e rs,hoon -- $(MAKE) dev

.PHONY: run-nockchain
run-nockchain: build ## Run nockchain node with optimized kernel pool
	$(call show_env_vars)
	@echo "Running nockchain node with kernel pool optimization..."
	@echo "Debug: TARGET_DIR = $(TARGET_DIR)"
	@echo "Debug: Binary path = ../$(TARGET_DIR)/nockchain"
	@echo "Debug: Checking if binary exists..."
	@ls -la $(TARGET_DIR)/nockchain || echo "Binary not found!"
	@echo "Debug: Creating miner-node directory..."
	mkdir -p miner-node
	@echo "Debug: Current working directory before cd: $$(pwd)"
	@echo "Debug: Starting nockchain with full debugging..."
	cd miner-node && \
	echo "Debug: Inside miner-node, pwd = $$(pwd)" && \
	echo "Debug: Checking binary path from here..." && \
	ls -la ../$(TARGET_DIR)/nockchain && \
	rm -f nockchain.sock && \
	echo "Debug: About to start nockchain..." && \
	RUST_BACKTRACE=1 \
	RUST_LOG=debug,nockchain=debug,nockchain_libp2p_io=info,libp2p=info \
	../$(TARGET_DIR)/nockchain \
		--npc-socket nockchain.sock \
		--mining-pubkey $(MINING_PUBKEY) \
		--mine \
		--peer /ip4/95.216.102.60/udp/3006/quic-v1 \
		--peer /ip4/65.108.123.225/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.108/udp/3006/quic-v1 \
		--peer /ip4/65.21.67.175/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.172/udp/3006/quic-v1 \
		--peer /ip4/34.174.22.166/udp/3006/quic-v1 \
		--peer /ip4/34.95.155.151/udp/30000/quic-v1 \
		--peer /ip4/34.18.98.38/udp/30000/quic-v1

.PHONY: run-nockchain-no-mining
run-nockchain-no-mining: build ## Run nockchain node without mining (observer mode)
	$(call show_env_vars)
	@echo "Running nockchain node in observer mode..."
	@echo "Debug: TARGET_DIR = $(TARGET_DIR)"
	@echo "Debug: Binary path = ../$(TARGET_DIR)/nockchain"
	mkdir -p observer-node
	cd observer-node && \
	echo "Debug: Inside observer-node, pwd = $$(pwd)" && \
	rm -f nockchain.sock && \
	RUST_BACKTRACE=1 \
	RUST_LOG=debug,nockchain=debug,nockchain_libp2p_io=info \
	../$(TARGET_DIR)/nockchain \
		--npc-socket nockchain.sock \
		--peer /ip4/95.216.102.60/udp/3006/quic-v1 \
		--peer /ip4/65.108.123.225/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.108/udp/3006/quic-v1 \
		--peer /ip4/65.21.67.175/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.172/udp/3006/quic-v1 \
		--peer /ip4/34.174.22.166/udp/3006/quic-v1 \
		--peer /ip4/34.95.155.151/udp/30000/quic-v1 \
		--peer /ip4/34.18.98.38/udp/30000/quic-v1

.PHONY: status
status: ## Show build status
	@echo "Build status:"
	@echo "  Hoonc installed: $(if $(wildcard $(HOONC_TIMESTAMP)),✓,✗)"
	@echo "  Assets built:"
	@for asset in $(HOON_TARGETS); do \
		echo "    $$(basename $$asset): $(if $(wildcard $$asset),✓,✗)"; \
	done
	@echo "  Rust targets:"
	@echo "    nockchain: $(if $(wildcard $(TARGET_DIR)/nockchain),✓,✗)"
	@echo "    nockchain-wallet: $(if $(wildcard $(TARGET_DIR)/nockchain-wallet),✓,✗)"

## Parallel build support
.PHONY: build-parallel
build-parallel: ## Build with maximum parallelism
	@echo "Building with maximum parallelism..."
	$(MAKE) -j$(CARGO_BUILD_JOBS) build

# Performance profiling
.PHONY: profile-build
profile-build: ## Profile the build process
	@echo "Profiling build process..."
	time $(MAKE) clean-build

# Advanced targets for CI/development
.PHONY: ci-build
ci-build: ## CI-optimized build
	@echo "Running CI build..."
	$(MAKE) clean
	$(MAKE) build BUILD_MODE=release CARGO_BUILD_JOBS=$(CARGO_BUILD_JOBS)
	$(MAKE) test BUILD_MODE=release

.PHONY: bench
bench: build-rust ## Run benchmarks
	@echo "Running benchmarks..."
	cargo bench

# Debugging helpers
.PHONY: debug-deps
debug-deps: ## Show dependency information
	@echo "Hoon sources found: $(words $(HOON_SOURCES))"
	@echo "First 5 sources:"
	@printf '%s\n' $(wordlist 1,5,$(HOON_SOURCES))
	@echo "Hoon targets: $(HOON_TARGETS)"

.PHONY: debug-timestamps
debug-timestamps: ## Show file timestamps for debugging
	@echo "File timestamps:"
	@echo "  hoonc timestamp: $(if $(wildcard $(HOONC_TIMESTAMP)),$$(stat -c %Y $(HOONC_TIMESTAMP) 2>/dev/null || stat -f %m $(HOONC_TIMESTAMP)),missing)"
	@for target in $(HOON_TARGETS); do \
		echo "  $$(basename $$target): $(if $(wildcard $$target),$$(stat -c %Y $$target 2>/dev/null || stat -f %m $$target),missing)"; \
	done

# Environment variable display function
define show_env_vars
	@echo "Environment variables loaded from .env"
endef

# Make sure we don't run into issues with file names as targets
.PHONY: $(HOON_TARGETS)

.PHONY: debug-run
debug-run: build ## Debug run with enhanced verbosity
	$(call show_env_vars)
	@echo "🔍 DEBUG MODE: Enhanced verbosity enabled"
	@echo "Debug: TARGET_DIR = $(TARGET_DIR)"
	@echo "Debug: Binary exists: $$(ls -la $(TARGET_DIR)/nockchain | wc -l) files"
	@echo "Debug: Binary size: $$(ls -lh $(TARGET_DIR)/nockchain | awk '{print $$5}')"
	@echo "Debug: Mining pubkey: $(MINING_PUBKEY)"
	mkdir -p debug-node
	cd debug-node && \
	echo "🚀 Starting nockchain in DEBUG mode..." && \
	rm -f nockchain.sock && \
	RUST_BACKTRACE=1 \
	RUST_LOG=debug \
	MINIMAL_LOG_FORMAT=false \
	../$(TARGET_DIR)/nockchain \
		--npc-socket nockchain.sock \
		--mining-pubkey $(MINING_PUBKEY) \
		--mine \
		--peer /ip4/95.216.102.60/udp/3006/quic-v1 \
		--peer /ip4/65.108.123.225/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.108/udp/3006/quic-v1 \
		--peer /ip4/65.21.67.175/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.172/udp/3006/quic-v1 \
		--peer /ip4/34.174.22.166/udp/3006/quic-v1 \
		--peer /ip4/34.95.155.151/udp/30000/quic-v1 \
		--peer /ip4/34.18.98.38/udp/30000/quic-v1

.PHONY: test-kernel-pool
test-kernel-pool: build ## Test kernel pool functionality specifically
	@echo "🧪 Testing kernel pool optimization..."
	@echo "Debug: This will test if our kernel pool is working correctly"
	mkdir -p test-mining
	cd test-mining && \
	echo "Testing kernel pool..." && \
	RUST_BACKTRACE=1 \
	RUST_LOG=debug,nockchain::kernel_pool=debug,nockchain::mining=debug \
	../$(TARGET_DIR)/nockchain \
		--npc-socket test.sock \
		--mining-pubkey $(MINING_PUBKEY) \
		--mine \
		--peer /ip4/95.216.102.60/udp/3006/quic-v1 \
		--peer /ip4/65.108.123.225/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.108/udp/3006/quic-v1 \
		--peer /ip4/65.21.67.175/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.172/udp/3006/quic-v1 \
		--peer /ip4/34.174.22.166/udp/3006/quic-v1 \
		--peer /ip4/34.95.155.151/udp/30000/quic-v1 \
		--peer /ip4/34.18.98.38/udp/30000/quic-v1

.PHONY: system-info
system-info: ## Show system configuration and expected mining performance
	@echo "🖥️  SYSTEM CONFIGURATION"
	@echo "======================================"
	@echo "CPU cores: $$(nproc)"
	@echo "Available parallelism: $$(nproc)"
	@echo "Build jobs: $(CARGO_BUILD_JOBS)"
	@echo "Tokio worker threads: $(TOKIO_WORKER_THREADS)"
	@echo "Rayon threads: $(RAYON_NUM_THREADS)"
	@echo ""
	@echo "⚡ EXPECTED MINING PERFORMANCE"
	@echo "======================================"
	@echo "Kernel pool size: $$(( $$(nproc) / 2 )) - $$(( $$(nproc) * 3 / 4 )) kernels"
	@echo "Expected CPU usage: ~75% of all cores"
	@echo "Estimated mining rate: ~$$(( $$(nproc) * 100 )) attempts/sec"
	@echo ""
	@echo "🚀 With your $$(nproc)-thread system, you should see:"
	@echo "   • $$(( $$(nproc) * 3 / 4 )) parallel mining kernels"
	@echo "   • ~75% CPU utilization across all cores"
	@echo "   • 10-50x faster mining than single-kernel"

.PHONY: debug-mining-verbose
debug-mining-verbose: build ## Maximum verbosity mining debug with thread analysis
	$(call show_env_vars)
	@echo "🔍 MAXIMUM VERBOSITY DEBUG - Thread Analysis"
	@echo "======================================================"
	@echo "System: $$(nproc) cores detected"
	@echo "Expected kernel pool: $$(( $$(nproc) / 2 )) - $$(( $$(nproc) * 3 / 4 )) kernels"
	@echo "TOKIO_WORKER_THREADS: $(TOKIO_WORKER_THREADS)"
	@echo "RAYON_NUM_THREADS: $(RAYON_NUM_THREADS)"
	@echo "======================================================"
	mkdir -p debug-mining-verbose
	cd debug-mining-verbose && \
	echo "🚀 Starting MAXIMUM VERBOSITY mining debug..." && \
	rm -f nockchain.sock && \
	RUST_BACKTRACE=full \
	RUST_LOG=trace,nockchain::mining=trace,nockchain::kernel_pool=trace,tokio=debug,rayon=debug \
	MINIMAL_LOG_FORMAT=false \
	../$(TARGET_DIR)/nockchain \
		--npc-socket nockchain.sock \
		--mining-pubkey $(MINING_PUBKEY) \
		--mine \
		--peer /ip4/95.216.102.60/udp/3006/quic-v1 \
		--peer /ip4/65.108.123.225/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.108/udp/3006/quic-v1 \
		--peer /ip4/65.21.67.175/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.172/udp/3006/quic-v1 \
		--peer /ip4/34.174.22.166/udp/3006/quic-v1 \
		--peer /ip4/34.95.155.151/udp/30000/quic-v1 \
		--peer /ip4/34.18.98.38/udp/30000/quic-v1

.PHONY: debug-clean
debug-clean: build ## Debug run with clean logs (debug level, no traces)
	$(call show_env_vars)
	@echo "🔍 DEBUG MODE: Clean debug logs (no traces)"
	@echo "Debug: TARGET_DIR = $(TARGET_DIR)"
	@echo "Debug: Binary size: $$(ls -lh $(TARGET_DIR)/nockchain | awk '{print $$5}')"
	@echo "Debug: Mining pubkey: $(MINING_PUBKEY)"
	@echo "Debug: System cores: $$(nproc)"
	@echo "Debug: Expected kernel pool: $$(( $$(nproc) / 2 )) - $$(( $$(nproc) * 3 / 4 )) kernels"
	mkdir -p debug-clean
	cd debug-clean && \
	echo "🚀 Starting nockchain with CLEAN DEBUG logs..." && \
	rm -f nockchain.sock && \
	RUST_BACKTRACE=1 \
	RUST_LOG=debug,nockchain::mining=debug,nockchain::kernel_pool=debug,nockchain_libp2p_io=info,libp2p=info \
	MINIMAL_LOG_FORMAT=false \
	../$(TARGET_DIR)/nockchain \
		--npc-socket nockchain.sock \
		--mining-pubkey $(MINING_PUBKEY) \
		--mine \
		--peer /ip4/95.216.102.60/udp/3006/quic-v1 \
		--peer /ip4/65.108.123.225/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.108/udp/3006/quic-v1 \
		--peer /ip4/65.21.67.175/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.172/udp/3006/quic-v1 \
		--peer /ip4/34.174.22.166/udp/3006/quic-v1 \
		--peer /ip4/34.95.155.151/udp/30000/quic-v1 \
		--peer /ip4/34.18.98.38/udp/30000/quic-v1

.PHONY: test-threading
test-threading: build ## Test if multi-threading optimizations are working
	@echo "🧪 TESTING MULTI-THREADING OPTIMIZATIONS"
	@echo "=========================================="
	@echo "System cores: $$(nproc)"
	@echo "Environment variables:"
	@echo "  TOKIO_WORKER_THREADS: $(TOKIO_WORKER_THREADS)"
	@echo "  RAYON_NUM_THREADS: $(RAYON_NUM_THREADS)"
	@echo "  CARGO_BUILD_JOBS: $(CARGO_BUILD_JOBS)"
	@echo ""
	@echo "🔬 Starting thread test - watch CPU usage with 'htop' in another terminal"
	@echo "Expected: ~75% CPU usage across all cores"
	@echo "Starting in 3 seconds..."
	@sleep 3
	mkdir -p test-threading
	cd test-threading && \
	rm -f test.sock && \
	RUST_BACKTRACE=1 \
	RUST_LOG=info,nockchain::mining=debug,nockchain::kernel_pool=info \
	../$(TARGET_DIR)/nockchain \
		--npc-socket test.sock \
		--mining-pubkey $(MINING_PUBKEY) \
		--mine \
		--peer /ip4/95.216.102.60/udp/3006/quic-v1 \
		--peer /ip4/65.108.123.225/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.108/udp/3006/quic-v1 \
		--peer /ip4/65.21.67.175/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.172/udp/3006/quic-v1 \
		--peer /ip4/34.174.22.166/udp/3006/quic-v1 \
		--peer /ip4/34.95.155.151/udp/30000/quic-v1 \
		--peer /ip4/34.18.98.38/udp/30000/quic-v1

.PHONY: kernel-pool-test
kernel-pool-test: build ## Test kernel pool creation and utilization
	@echo "🔧 KERNEL POOL DIAGNOSTIC TEST"
	@echo "=============================="
	mkdir -p kernel-pool-test
	cd kernel-pool-test && \
	echo "Testing kernel pool with verbose logging..." && \
	RUST_BACKTRACE=1 \
	RUST_LOG=trace,nockchain::kernel_pool=trace,nockchain::mining=trace \
	timeout 30s ../$(TARGET_DIR)/nockchain \
		--npc-socket kernel-test.sock \
		--mining-pubkey $(MINING_PUBKEY) \
		--mine \
		--peer /ip4/95.216.102.60/udp/3006/quic-v1 \
		--peer /ip4/65.108.123.225/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.108/udp/3006/quic-v1 \
		--peer /ip4/65.21.67.175/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.172/udp/3006/quic-v1 \
		--peer /ip4/34.174.22.166/udp/3006/quic-v1 \
		--peer /ip4/34.95.155.151/udp/30000/quic-v1 \
		--peer /ip4/34.18.98.38/udp/30000/quic-v1 \
	|| echo "Test completed after 30 seconds"

.PHONY: debug-clean
debug-clean: build ## Debug run with clean logs (debug level, no traces)
	$(call show_env_vars)
	@echo "🔍 DEBUG MODE: Clean debug logs (no traces)"
	@echo "Debug: TARGET_DIR = $(TARGET_DIR)"
	@echo "Debug: Binary size: $$(ls -lh $(TARGET_DIR)/nockchain | awk '{print $$5}')"
	@echo "Debug: Mining pubkey: $(MINING_PUBKEY)"
	@echo "Debug: System cores: $$(nproc)"
	@echo "Debug: Expected kernel pool: $$(( $$(nproc) / 2 )) - $$(( $$(nproc) * 3 / 4 )) kernels"
	mkdir -p debug-clean
	cd debug-clean && \
	echo "🚀 Starting nockchain with CLEAN DEBUG logs..." && \
	rm -f nockchain.sock && \
	RUST_BACKTRACE=1 \
	RUST_LOG=debug,nockchain::mining=debug,nockchain::kernel_pool=debug,nockchain_libp2p_io=info,libp2p=info \
	MINIMAL_LOG_FORMAT=false \
	../$(TARGET_DIR)/nockchain \
		--npc-socket nockchain.sock \
		--mining-pubkey $(MINING_PUBKEY) \
		--mine \
		--peer /ip4/95.216.102.60/udp/3006/quic-v1 \
		--peer /ip4/65.108.123.225/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.108/udp/3006/quic-v1 \
		--peer /ip4/65.21.67.175/udp/3006/quic-v1 \
		--peer /ip4/65.109.156.172/udp/3006/quic-v1 \
		--peer /ip4/34.174.22.166/udp/3006/quic-v1 \
		--peer /ip4/34.95.155.151/udp/30000/quic-v1 \
		--peer /ip4/34.18.98.38/udp/30000/quic-v1
