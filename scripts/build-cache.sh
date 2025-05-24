#!/bin/bash
set -euo pipefail

# Build cache management for Nockchain

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

CACHE_DIR=".build-cache"
RUST_CACHE_DIR="$CACHE_DIR/rust"
HOON_CACHE_DIR="$CACHE_DIR/hoon"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() {
    echo -e "${BLUE}[CACHE]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[CACHE]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[CACHE]${NC} $1"
}

# Initialize cache directories
init_cache() {
    log_info "Initializing build cache..."
    mkdir -p "$RUST_CACHE_DIR" "$HOON_CACHE_DIR"

    # Create cache metadata
    cat > "$CACHE_DIR/metadata.json" <<EOF
{
    "created": "$(date -Iseconds)",
    "version": "1.0",
    "last_cleanup": "$(date -Iseconds)"
}
EOF

    log_success "Cache initialized at $CACHE_DIR"
}

# Cache Rust build artifacts
cache_rust() {
    log_info "Caching Rust build artifacts..."

    # Cache compiled dependencies
    if [[ -d "target" ]]; then
        rsync -a --delete target/debug/deps/ "$RUST_CACHE_DIR/deps/" 2>/dev/null || true
        rsync -a --delete target/release/deps/ "$RUST_CACHE_DIR/release-deps/" 2>/dev/null || true
    fi

    log_success "Rust artifacts cached"
}

# Restore Rust cache
restore_rust() {
    log_info "Restoring Rust build cache..."

    mkdir -p target/debug/deps target/release/deps

    if [[ -d "$RUST_CACHE_DIR/deps" ]]; then
        rsync -a "$RUST_CACHE_DIR/deps/" target/debug/deps/ 2>/dev/null || true
    fi

    if [[ -d "$RUST_CACHE_DIR/release-deps" ]]; then
        rsync -a "$RUST_CACHE_DIR/release-deps/" target/release/deps/ 2>/dev/null || true
    fi

    log_success "Rust cache restored"
}

# Cache Hoon compilation results
cache_hoon() {
    log_info "Caching Hoon compilation results..."

    # Cache compiled assets
    if [[ -d "assets" ]]; then
        rsync -a --delete assets/ "$HOON_CACHE_DIR/assets/" 2>/dev/null || true
    fi

    # Cache hoonc build timestamp
    if [[ -f ".hoonc_built" ]]; then
        cp .hoonc_built "$HOON_CACHE_DIR/"
    fi

    log_success "Hoon artifacts cached"
}

# Restore Hoon cache
restore_hoon() {
    log_info "Restoring Hoon build cache..."

    mkdir -p assets

    if [[ -d "$HOON_CACHE_DIR/assets" ]]; then
        rsync -a "$HOON_CACHE_DIR/assets/" assets/ 2>/dev/null || true
    fi

    if [[ -f "$HOON_CACHE_DIR/.hoonc_built" ]]; then
        cp "$HOON_CACHE_DIR/.hoonc_built" . 2>/dev/null || true
    fi

    log_success "Hoon cache restored"
}

# Clean cache
clean_cache() {
    log_info "Cleaning build cache..."
    rm -rf "$CACHE_DIR"
    log_success "Build cache cleaned"
}

# Show cache status
status() {
    if [[ ! -d "$CACHE_DIR" ]]; then
        echo "Cache not initialized"
        return
    fi

    echo "Cache status:"
    echo "  Location: $CACHE_DIR"
    echo "  Size: $(du -sh "$CACHE_DIR" 2>/dev/null | cut -f1)"
    echo "  Rust cache: $(if [[ -d "$RUST_CACHE_DIR" ]]; then echo "✓"; else echo "✗"; fi)"
    echo "  Hoon cache: $(if [[ -d "$HOON_CACHE_DIR" ]]; then echo "✓"; else echo "✗"; fi)"

    if [[ -f "$CACHE_DIR/metadata.json" ]]; then
        echo "  Created: $(grep '"created"' "$CACHE_DIR/metadata.json" | cut -d'"' -f4 2>/dev/null || echo "unknown")"
    fi
}

# Main function
case "${1:-status}" in
    init)
        init_cache
        ;;
    cache)
        cache_rust
        cache_hoon
        ;;
    restore)
        restore_rust
        restore_hoon
        ;;
    clean)
        clean_cache
        ;;
    status)
        status
        ;;
    *)
        echo "Usage: $0 {init|cache|restore|clean|status}"
        echo ""
        echo "Commands:"
        echo "  init     Initialize cache directories"
        echo "  cache    Cache current build artifacts"
        echo "  restore  Restore cached build artifacts"
        echo "  clean    Clean all cached data"
        echo "  status   Show cache status"
        exit 1
        ;;
esac