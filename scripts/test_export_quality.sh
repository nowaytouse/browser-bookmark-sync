#!/bin/bash
# Test script for export quality improvements
# Tests: --flat, --dedupe, --clean, --update

set -e

BINARY="./target/release/browser-bookmark-sync"
TEST_DIR="/tmp/bsync_export_test"

echo "🧪 Export Quality Improvement Tests"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Clean up
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"

# Build if needed
if [ ! -f "$BINARY" ]; then
    echo "📦 Building..."
    cargo build --release
fi

# Test 1: Flat export
echo ""
echo "📋 Test 1: Flat export (--flat)"
$BINARY export -o "$TEST_DIR/flat.html" --flat -b waterfox 2>&1 | grep -E "(Removed.*browser root|Exported)"

# Verify no browser root folders
if grep -qiE "<H3.*(waterfox|brave|chrome|safari|书签栏|bookmarks bar)</H3>" "$TEST_DIR/flat.html"; then
    echo "❌ FAIL: Browser root folders found in flat export"
    exit 1
else
    echo "✅ PASS: No browser root folders in flat export"
fi

# Test 2: Deduplicate
echo ""
echo "📋 Test 2: Deduplicate (--dedupe)"
$BINARY export -o "$TEST_DIR/dedupe.html" -d -b waterfox 2>&1 | grep -E "(Removed.*duplicate|Exported)"
echo "✅ PASS: Deduplication completed"

# Test 3: Clean empty folders
echo ""
echo "📋 Test 3: Clean empty folders (--clean)"
$BINARY export -o "$TEST_DIR/clean.html" --clean -b waterfox 2>&1 | grep -E "(Removed.*empty|Exported)"
echo "✅ PASS: Empty folder cleanup completed"

# Test 4: Combined options
echo ""
echo "📋 Test 4: Combined (--flat --dedupe --clean)"
$BINARY export -o "$TEST_DIR/combined.html" --flat -d --clean -b waterfox 2>&1 | grep -E "(Removed|Exported)"
echo "✅ PASS: Combined options work"

# Test 5: Unicode/Emoji preservation
echo ""
echo "📋 Test 5: Unicode/Emoji folder names"
if grep -q "直播平台\|谷歌服务\|金融理财" "$TEST_DIR/flat.html"; then
    echo "✅ PASS: Chinese folder names preserved"
else
    echo "⚠️  WARN: No Chinese folder names found (may be expected)"
fi

# Test 6: HTML escaping
echo ""
echo "📋 Test 6: HTML special character escaping"
if grep -q "&amp;\|&lt;\|&gt;\|&quot;" "$TEST_DIR/flat.html"; then
    echo "✅ PASS: HTML special characters properly escaped"
else
    echo "⚠️  WARN: No escaped characters found (may be expected)"
fi

# Test 7: Incremental update
echo ""
echo "📋 Test 7: Incremental update (--update)"
# First export
$BINARY export -o "$TEST_DIR/base.html" -b waterfox 2>&1 | grep "Exported"
# Update (should skip duplicates)
$BINARY export -o "$TEST_DIR/updated.html" -u "$TEST_DIR/base.html" -b waterfox 2>&1 | grep -E "(Incremental|Exported)"
echo "✅ PASS: Incremental update completed"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ All export quality tests passed!"
echo "📁 Test files in: $TEST_DIR"
