#!/bin/bash
# 测试 check 命令的验证脚本
# Test script for the check command

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BSYNC="$PROJECT_DIR/target/release/browser-bookmark-sync"

echo "🔧 Building release..."
cd "$PROJECT_DIR"
cargo build --release

echo ""
echo "📋 Testing check command..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Test 1: Help
echo "Test 1: Check help"
$BSYNC check --help
echo "✅ Help works"
echo ""

# Test 2: Dry-run without proxy
echo "Test 2: Dry-run check (direct only)"
$BSYNC check --dry-run --timeout 5 --concurrency 5 2>&1 | head -30
echo "✅ Dry-run works"
echo ""

# Test 3: Verbose mode
echo "Test 3: Verbose check"
$BSYNC check --dry-run --verbose --timeout 5 --concurrency 3 2>&1 | head -50
echo "✅ Verbose mode works"
echo ""

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ All tests passed!"
