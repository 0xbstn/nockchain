#!/bin/bash
set -euo pipefail

# Fast build script with intelligent caching and parallel execution

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

# Configuration
BUILD_MODE="${BUILD_MODE:-dev}"
FORCE_REBUILD="${FORCE_REBUILD:-false}"
VERBOSE="${VERBOSE:-false}"
JOBS="${JOBS:-$(nproc)}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if we need to rebuild hoonc
check_hoonc_needs_rebuild() {
    local hoonc_timestamp=".hoonc_built"

    if [[ ! -f "$hoonc_timestamp" ]]; then
        return 0  # Need to build
    fi

    # Check if any hoonc source files are newer than timestamp
    if find crates/hoonc -name "*.rs" -newer "$hoonc_timestamp" 2>/dev/null | grep -q .; then
        return 0  # Need to rebuild
    fi

    if find crates/hoonc -name "Cargo.toml" -newer "$hoonc_timestamp" 2>/dev/null | grep -q .; then
        return 0  # Need to rebuild
    fi

    return 1  # No rebuild needed
}

# Smart hoonc build (install globally like Makefile)
build_hoonc() {
    if [[ "$FORCE_REBUILD" == "true" ]] || check_hoonc_needs_rebuild; then
        log_info "Building and installing hoonc compiler..."

        CARGO_BUILD_JOBS="$JOBS" cargo install --locked --force --path crates/hoonc --bin hoonc
        touch .hoonc_built
        log_success "hoonc installed successfully"
    else
        log_info "hoonc is up to date, skipping build"
    fi

    # Verify hoonc is available globally
    if ! command -v hoonc >/dev/null 2>&1; then
        log_error "hoonc not found in PATH after installation"
        exit 1
    fi
}

# Check if asset needs rebuild
asset_needs_rebuild() {
    local asset_file="$1"
    local main_source="$2"

    # If asset doesn't exist, needs rebuild
    if [[ ! -f "$asset_file" ]]; then
        return 0
    fi

    # If main source is newer than asset, needs rebuild
    if [[ "$main_source" -nt "$asset_file" ]]; then
        return 0
    fi

    # If any Hoon source is newer than asset, needs rebuild
    if find hoon -name "*.hoon" -newer "$asset_file" 2>/dev/null | grep -q .; then
        return 0
    fi

    # If hoonc is newer than asset, needs rebuild
    if [[ ".hoonc_built" -nt "$asset_file" ]]; then
        return 0
    fi

    return 1
}

# Build a single Hoon asset (using global hoonc like Makefile)
build_hoon_asset() {
    local asset_name="$1"
    local source_file="$2"
    local asset_file="assets/${asset_name}.jam"

    mkdir -p assets

    if [[ "$FORCE_REBUILD" == "true" ]] || asset_needs_rebuild "$asset_file" "$source_file"; then
        log_info "Building $asset_name.jam..."

        # Use same command as Makefile: RUST_LOG=info hoonc (reduced from trace to eliminate warnings)
        if [[ "$VERBOSE" == "true" ]]; then
            RUST_LOG=trace hoonc "$source_file" hoon
        else
            RUST_LOG=info hoonc "$source_file" hoon >/dev/null 2>&1
        fi

        # Check if out.jam was created
        if [[ ! -f "out.jam" ]]; then
            log_error "Failed to generate out.jam for $asset_name"
            return 1
        fi

        mv out.jam "$asset_file"
        log_success "$asset_name.jam built successfully"
    else
        log_info "$asset_name.jam is up to date, skipping build"
    fi
}

# Build all Hoon assets (SEQUENTIALLY to avoid out.jam conflicts)
build_hoon_assets() {
    log_info "Building Hoon assets..."

    # Build trivial first if needed
    if [[ ! -f "hoon/trivial.hoon" ]]; then
        echo '%trivial' > hoon/trivial.hoon
    fi

    # Build assets sequentially to avoid out.jam conflicts
    build_hoon_asset "dumb" "hoon/apps/dumbnet/outer.hoon" || {
        log_error "Failed to build dumb.jam"
        exit 1
    }

    build_hoon_asset "wal" "hoon/apps/wallet/wallet.hoon" || {
        log_error "Failed to build wal.jam"
        exit 1
    }

    build_hoon_asset "miner" "hoon/apps/dumbnet/miner.hoon" || {
        log_error "Failed to build miner.jam"
        exit 1
    }

    log_success "All Hoon assets built successfully"
}

# Build Rust components
build_rust() {
    log_info "Building Rust components..."

    local cargo_flags=""
    if [[ "$BUILD_MODE" == "release" ]]; then
        cargo_flags="--release"
    fi

    if [[ "$VERBOSE" == "true" ]]; then
        CARGO_BUILD_JOBS="$JOBS" cargo build $cargo_flags
    else
        CARGO_BUILD_JOBS="$JOBS" cargo build $cargo_flags >/dev/null 2>&1
    fi

    log_success "Rust components built successfully"
}

# Main build function
main() {
    local start_time=$(date +%s)

    log_info "Starting fast build (mode: $BUILD_MODE, jobs: $JOBS)"

    # Create necessary directories
    mkdir -p hoon assets

    # Build components in optimal order
    build_hoonc

    # Build Hoon assets first (sequential, no conflicts)
    build_hoon_assets

    # Then build Rust (can be parallel with nothing)
    build_rust

    local end_time=$(date +%s)
    local duration=$((end_time - start_time))

    log_success "Build completed in ${duration}s"
}

# Handle command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --release)
            BUILD_MODE="release"
            shift
            ;;
        --force)
            FORCE_REBUILD="true"
            shift
            ;;
        --verbose|-v)
            VERBOSE="true"
            shift
            ;;
        --jobs|-j)
            JOBS="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --release     Build in release mode"
            echo "  --force       Force rebuild of all components"
            echo "  --verbose,-v  Verbose output"
            echo "  --jobs,-j N   Use N parallel jobs"
            echo "  --help,-h     Show this help"
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Run the build
main