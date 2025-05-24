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

# Directories and files
HOON_DIR = hoon
ASSETS_DIR = assets
SCRIPTS_DIR = scripts
HOONC_BIN = $(TARGET_DIR)/hoonc
HOONC_TIMESTAMP = .hoonc_built

# Find all Hoon source files
HOON_SOURCES = $(shell find $(HOON_DIR) -name '*.hoon' 2>/dev/null)

# Hoon targets
HOON_TARGETS = $(ASSETS_DIR)/dumb.jam $(ASSETS_DIR)/wal.jam $(ASSETS_DIR)/miner.jam

.DEFAULT_GOAL := build

.PHONY: help
help: ## Show this help message
	@echo "Nockchain Build System - Optimized Version"
	@echo ""
	@echo "Available targets:"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  %-20s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@echo ""
	@echo "Build modes:"
	@echo "  make build              # Fast development build"
	@echo "  make build BUILD_MODE=release  # Optimized release build"
	@echo "  make fast-build         # Use optimized build script"
	@echo ""
	@echo "Environment variables:"
	@echo "  BUILD_MODE={dev|release}  # Build configuration (default: dev)"
	@echo "  CARGO_BUILD_JOBS=N        # Parallel jobs (default: $(CARGO_BUILD_JOBS))"

.PHONY: show-config
show-config: ## Show current build configuration
	@echo "Build configuration:"
	@echo "  BUILD_MODE: $(BUILD_MODE)"
	@echo "  CARGO_FLAGS: $(CARGO_FLAGS)"
	@echo "  CARGO_BUILD_JOBS: $(CARGO_BUILD_JOBS)"
	@echo "  TARGET_DIR: $(TARGET_DIR)"
	@echo "  HOONC_BIN: $(HOONC_BIN)"

.PHONY: build
build: $(HOON_TARGETS) build-rust ## Build everything (fast incremental)
	$(call show_env_vars)
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

$(HOONC_TIMESTAMP): crates/hoonc/src/**/*.rs crates/hoonc/Cargo.toml
	@echo "Building hoonc compiler..."
	CARGO_BUILD_JOBS=$(CARGO_BUILD_JOBS) cargo build $(CARGO_FLAGS) --bin hoonc
	@touch $(HOONC_TIMESTAMP)

$(HOONC_BIN): $(HOONC_TIMESTAMP)
	@test -f $(HOONC_BIN) || (echo "Error: hoonc binary not found at $(HOONC_BIN)" && exit 1)

.PHONY: install-hoonc
install-hoonc: $(HOONC_BIN) nuke-hoonc-data ## Install hoonc from this repo
	$(call show_env_vars)
	@echo "Installing hoonc..."
	cargo install --locked --force --path crates/hoonc --bin hoonc

.PHONY: update-hoonc
update-hoonc: $(HOONC_BIN) ## Ensure hoonc is built and up-to-date
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
build-hoon-all: $(HOON_TARGETS) ## Build all Hoon assets

.PHONY: build-hoon
build-hoon: $(HOON_TARGETS) ## Build Hoon assets (incremental)

.PHONY: build-trivial
build-trivial: ensure-dirs $(HOONC_BIN)
	@echo "Building trivial Hoon..."
	@echo '%trivial' > $(HOON_DIR)/trivial.hoon
	$(HOONC_BIN) --arbitrary $(HOON_DIR)/trivial.hoon

# Optimized asset building with proper dependencies
$(ASSETS_DIR)/dumb.jam: $(HOONC_BIN) $(HOON_DIR)/apps/dumbnet/outer.hoon $(HOON_SOURCES) | ensure-dirs
	$(call show_env_vars)
	@echo "Building dumb.jam..."
	@cd $(CURDIR) && $(HOONC_BIN) $(HOON_DIR)/apps/dumbnet/outer.hoon $(HOON_DIR)
	@mv out.jam $@

$(ASSETS_DIR)/wal.jam: $(HOONC_BIN) $(HOON_DIR)/apps/wallet/wallet.hoon $(HOON_SOURCES) | ensure-dirs
	$(call show_env_vars)
	@echo "Building wal.jam..."
	@cd $(CURDIR) && $(HOONC_BIN) $(HOON_DIR)/apps/wallet/wallet.hoon $(HOON_DIR)
	@mv out.jam $@

$(ASSETS_DIR)/miner.jam: $(HOONC_BIN) $(HOON_DIR)/apps/dumbnet/miner.hoon $(HOON_SOURCES) | ensure-dirs
	$(call show_env_vars)
	@echo "Building miner.jam..."
	@cd $(CURDIR) && $(HOONC_BIN) $(HOON_DIR)/apps/dumbnet/miner.hoon $(HOON_DIR)
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
run-nockchain: build ## Run nockchain node
	$(call show_env_vars)
	@echo "Running nockchain node..."
	mkdir -p miner-node
	cd miner-node && rm -f nockchain.sock && \
	RUST_BACKTRACE=1 $(TARGET_DIR)/nockchain \
		--npc-socket nockchain.sock \
		--mining-pubkey $(MINING_PUBKEY) \
		--mine

.PHONY: status
status: ## Show build status
	@echo "Build status:"
	@echo "  Hoonc built: $(if $(wildcard $(HOONC_TIMESTAMP)),✓,✗)"
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
	@echo "  hoonc timestamp: $(if $(wildcard $(HOONC_TIMESTAMP)),$$(stat -c %Y $(HOONC_TIMESTAMP)),missing)"
	@for target in $(HOON_TARGETS); do \
		echo "  $$(basename $$target): $(if $(wildcard $$target),$$(stat -c %Y $$target),missing)"; \
	done

# Environment variable display function
define show_env_vars
	@echo "Environment variables loaded from .env"
endef

# Make sure we don't run into issues with file names as targets
.PHONY: $(HOON_TARGETS)
