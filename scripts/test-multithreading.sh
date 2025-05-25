#!/bin/bash

# Test script for multi-threading mining optimizations
# This script tests the new batch processing and kernel pool optimizations

set -e

echo "🧪 TESTING MULTI-THREADING MINING OPTIMIZATIONS"
echo "================================================"

# Get system info
NUM_CORES=$(nproc)
echo "System cores detected: $NUM_CORES"

# Set optimized environment variables
export RUST_BACKTRACE=1
export RUST_LOG=info,nockchain::mining=debug,nockchain::kernel_pool=info
export TOKIO_WORKER_THREADS=$NUM_CORES
export RAYON_NUM_THREADS=$NUM_CORES
export MINIMAL_LOG_FORMAT=false

echo "Environment configured for $NUM_CORES cores"

# Build with optimizations
echo "🔨 Building with optimizations..."
cd "$(dirname "$0")/.."
make build BUILD_MODE=release

# Create test directory
TEST_DIR="test-multithreading-$(date +%s)"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

echo "📊 Expected performance with $NUM_CORES cores:"
echo "   • Kernel pool size: $((NUM_CORES * 3 / 4)) kernels"
echo "   • Expected CPU usage: ~75%"
echo "   • Target mining rate: >$((NUM_CORES / 2)) attempts/sec"

echo ""
echo "🚀 Starting mining test (will run for 60 seconds)..."
echo "Watch for:"
echo "   ✅ 'BATCH PROCESSING: X mine effects simultaneously'"
echo "   ✅ 'Mining performance monitor started'"
echo "   ✅ 'MINING PERFORMANCE REPORT' every 10 seconds"
echo "   ⚠️  Any performance warnings"

# Start mining with timeout
timeout 60s ../target/release/nockchain \
    --npc-socket test.sock \
    --mining-pubkey 2qwq9dQRZfpFx8BDicghpMRnYGKZsZGxxhh9m362pzpM9aeo276pR1yHZPS41y3CW3vPKxeYM8p8fzZS8GXmDGzmNNCnVNekjrSYogqfEFMqwhHh5iCjaKPaDTwhupWqiXj6 \
    --mine \
    --peer /ip4/95.216.102.60/udp/3006/quic-v1 \
    --peer /ip4/65.108.123.225/udp/3006/quic-v1 \
    --peer /ip4/65.109.156.108/udp/3006/quic-v1 \
    --peer /ip4/65.21.67.175/udp/3006/quic-v1 \
    --peer /ip4/65.109.156.172/udp/3006/quic-v1 \
    --peer /ip4/34.174.22.166/udp/3006/quic-v1 \
    --peer /ip4/34.95.155.151/udp/30000/quic-v1 \
    --peer /ip4/34.18.98.38/udp/30000/quic-v1 \
    2>&1 | tee mining-test.log || echo "Test completed (timeout expected)"

echo ""
echo "📈 ANALYZING TEST RESULTS..."
echo "================================"

# Analyze logs for performance indicators
if grep -q "BATCH PROCESSING:" mining-test.log; then
    echo "✅ Batch processing detected"
    BATCH_COUNT=$(grep -c "BATCH PROCESSING:" mining-test.log)
    echo "   Found $BATCH_COUNT batch operations"
else
    echo "❌ No batch processing detected - multi-threading may not be working"
fi

if grep -q "Mining performance monitor started" mining-test.log; then
    echo "✅ Performance monitoring active"
else
    echo "❌ Performance monitoring not detected"
fi

if grep -q "MINING PERFORMANCE REPORT" mining-test.log; then
    echo "✅ Performance reports generated"
    echo "   Last performance report:"
    grep -A 10 "MINING PERFORMANCE REPORT" mining-test.log | tail -10
else
    echo "❌ No performance reports found"
fi

# Check for warnings
if grep -q "LOW MINING RATE\|LOW CPU UTILIZATION\|SLOW KERNEL CHECKOUT" mining-test.log; then
    echo "⚠️  Performance warnings detected:"
    grep "LOW MINING RATE\|LOW CPU UTILIZATION\|SLOW KERNEL CHECKOUT" mining-test.log
else
    echo "✅ No performance warnings"
fi

# Check for successful mines
MINE_COUNT=$(grep -c "SUCCESSFUL MINE" mining-test.log || echo "0")
echo "🎉 Successful mines: $MINE_COUNT"

echo ""
echo "🔍 RECOMMENDATIONS:"
if [ "$MINE_COUNT" -gt 0 ]; then
    echo "✅ Mining is working correctly"
else
    echo "⚠️  No successful mines - this is normal for short tests"
fi

echo "📊 To monitor ongoing performance:"
echo "   tail -f mining-test.log | grep 'MINING PERFORMANCE REPORT' -A 10"

echo ""
echo "Test completed. Check mining-test.log for detailed output."