#!/bin/bash

# Browser Sync Test Script
# Tests incremental and full sync with validation

set -e

echo "🧪 Browser Sync Test Suite"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Build the project
echo ""
echo "📦 Building project..."
cargo build --release

BINARY="./target/release/browser-bookmark-sync"

# Test 1: List browsers
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 1: List detected browsers"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY list

# Test 2: Validate current state
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 2: Validate current bookmark state"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY validate --detailed

# Test 3: Dry run - Incremental sync
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 3: Dry run - Incremental sync"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY sync --browsers "waterfox,brave-nightly" --mode incremental --dry-run --verbose

# Test 4: Dry run - Full sync
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 4: Dry run - Full sync"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY sync --browsers "waterfox,brave-nightly" --mode full --dry-run --verbose

# Test 5: Cleanup test (dry run)
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 5: Cleanup test (dry run)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY cleanup --browsers "waterfox,brave-nightly" --remove-duplicates --remove-empty-folders --dry-run --verbose

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ All tests completed successfully!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "💡 To perform actual sync (not dry run):"
echo "   $BINARY sync --browsers \"waterfox,brave-nightly\" --mode incremental"
echo ""
echo "💡 To perform full sync:"
echo "   $BINARY sync --browsers \"waterfox,brave-nightly\" --mode full"
echo ""
