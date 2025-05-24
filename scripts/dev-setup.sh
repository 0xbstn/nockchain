#!/bin/bash
set -euo pipefail

# Development setup and workflow automation for Nockchain

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() {
    echo -e "${BLUE}[DEV]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[DEV]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[DEV]${NC} $1"
}

log_error() {
    echo -e "${RED}[DEV]${NC} $1"
}

# Check prerequisites
check_prerequisites() {
    log_info "Checking prerequisites..."

    # Check if required tools are installed
    local missing_tools=()

    if ! command -v cargo >/dev/null 2>&1; then
        missing_tools+=("cargo (Rust toolchain)")
    fi

    if ! command -v make >/dev/null 2>&1; then
        missing_tools+=("make")
    fi

    if ! command -v rsync >/dev/null 2>&1; then
        missing_tools+=("rsync")
    fi

    if [[ ${#missing_tools[@]} -gt 0 ]]; then
        log_error "Missing required tools:"
        for tool in "${missing_tools[@]}"; do
            echo "  - $tool"
        done
        exit 1
    fi

    log_success "All prerequisites satisfied"
}

# Setup development environment
setup_dev_env() {
    log_info "Setting up development environment..."

    # Initialize build cache
    "$SCRIPT_DIR/build-cache.sh" init

    # Create .env file if it doesn't exist
    if [[ ! -f ".env" ]]; then
        if [[ -f ".env_example" ]]; then
            cp .env_example .env
            log_info "Created .env from .env_example"
        else
            log_warn ".env_example not found, you may need to create .env manually"
        fi
    fi

    # Install watchexec for watch mode (optional)
    if ! command -v watchexec >/dev/null 2>&1; then
        log_info "Installing watchexec for watch mode..."
        cargo install watchexec-cli || log_warn "Failed to install watchexec, watch mode will not be available"
    fi

    log_success "Development environment setup complete"
}

# Quick build
quick_build() {
    log_info "Running quick development build..."
    "$SCRIPT_DIR/fast-build.sh" "$@"
}

# Quick build with cache restore
cached_build() {
    log_info "Running cached build..."
    "$SCRIPT_DIR/build-cache.sh" restore
    "$SCRIPT_DIR/fast-build.sh" "$@"
    "$SCRIPT_DIR/build-cache.sh" cache
}

# Development watch mode
watch_mode() {
    log_info "Starting development watch mode..."

    if ! command -v watchexec >/dev/null 2>&1; then
        log_error "watchexec not installed. Install with: cargo install watchexec-cli"
        exit 1
    fi

    # Initial build
    quick_build

    log_info "Watching for changes in Rust and Hoon files..."
    watchexec \
        --restart \
        --clear \
        --watch crates \
        --watch hoon \
        --exts rs,hoon,toml \
        --ignore target/ \
        --ignore assets/ \
        --ignore .build-cache/ \
        -- "$SCRIPT_DIR/fast-build.sh"
}

# Clean everything and rebuild
clean_rebuild() {
    log_info "Cleaning and rebuilding..."
    make clean
    "$SCRIPT_DIR/build-cache.sh" clean
    quick_build "$@"
}

# Show build status
build_status() {
    echo "Nockchain Build Status"
    echo "====================="
    make status
    echo ""
    "$SCRIPT_DIR/build-cache.sh" status
}

# Performance benchmark
benchmark_build() {
    log_info "Benchmarking build performance..."

    echo "Cleaning build..."
    make clean >/dev/null 2>&1
    "$SCRIPT_DIR/build-cache.sh" clean >/dev/null 2>&1

    echo "Cold build (no cache):"
    time "$SCRIPT_DIR/fast-build.sh" --force

    echo ""
    echo "Incremental build (with cache):"
    time "$SCRIPT_DIR/fast-build.sh"

    echo ""
    echo "No-op build (everything up to date):"
    time "$SCRIPT_DIR/fast-build.sh"
}

# Show help
show_help() {
    cat << EOF
Nockchain Development Tools

Usage: $0 <command> [options]

Commands:
    setup               Setup development environment
    build               Quick development build
    build-cached        Build with cache restore/save
    watch               Start watch mode for continuous building
    clean               Clean everything and rebuild
    status              Show build status
    benchmark           Benchmark build performance
    help                Show this help

Build options (for build commands):
    --release           Build in release mode
    --force             Force rebuild of all components
    --verbose           Verbose output
    --jobs N            Use N parallel jobs

Examples:
    $0 setup                    # Initial setup
    $0 build                    # Quick dev build
    $0 build --release          # Release build
    $0 build-cached --verbose   # Cached build with verbose output
    $0 watch                    # Start watch mode
    $0 clean --release          # Clean and rebuild in release mode

EOF
}

# Parse command line arguments
case "${1:-help}" in
    setup)
        shift
        check_prerequisites
        setup_dev_env
        ;;
    build)
        shift
        quick_build "$@"
        ;;
    build-cached)
        shift
        cached_build "$@"
        ;;
    watch)
        shift
        watch_mode
        ;;
    clean)
        shift
        clean_rebuild "$@"
        ;;
    status)
        build_status
        ;;
    benchmark)
        benchmark_build
        ;;
    help|--help|-h)
        show_help
        ;;
    *)
        log_error "Unknown command: $1"
        echo ""
        show_help
        exit 1
        ;;
esac