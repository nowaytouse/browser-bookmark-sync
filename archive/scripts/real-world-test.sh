#!/bin/bash

# Real-world test with Waterfox and Brave Nightly
# This script performs actual sync operations with validation

set -e

BINARY="./target/release/browser-bookmark-sync"
BROWSERS="waterfox,brave-nightly"

echo "🚀 Real-world Browser Sync Test"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Target browsers: $BROWSERS"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Step 1: Pre-sync validation
echo "📋 Step 1: Pre-sync validation"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY validate --detailed
echo ""

# Step 2: List browsers
echo "🌐 Step 2: Detected browsers"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY list
echo ""

# Step 3: Dry run to preview changes
echo "🔍 Step 3: Dry run preview"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY sync --browsers "$BROWSERS" --mode incremental --dry-run --verbose
echo ""

# Step 4: Ask for confirmation
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
read -p "⚠️  Proceed with actual sync? (yes/no): " confirm
echo ""

if [ "$confirm" != "yes" ]; then
    echo "❌ Sync cancelled by user"
    exit 0
fi

# Step 5: Perform incremental sync
echo "🔄 Step 4: Performing incremental sync"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY sync --browsers "$BROWSERS" --mode incremental --verbose
echo ""

# Step 6: Post-sync validation
echo "✅ Step 5: Post-sync validation"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY validate --detailed
echo ""

# Step 7: Cleanup duplicates
echo "🧹 Step 6: Cleanup duplicates (dry run)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY cleanup --browsers "$BROWSERS" --remove-duplicates --dry-run --verbose
echo ""

read -p "⚠️  Proceed with cleanup? (yes/no): " cleanup_confirm
echo ""

if [ "$cleanup_confirm" = "yes" ]; then
    echo "🧹 Performing cleanup"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    $BINARY cleanup --browsers "$BROWSERS" --remove-duplicates --verbose
    echo ""
fi

# Step 8: Final validation
echo "🎯 Step 7: Final validation"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY validate --detailed
echo ""

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ Real-world test completed successfully!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "📊 Summary:"
echo "  - Pre-sync validation: ✅"
echo "  - Incremental sync: ✅"
echo "  - Post-sync validation: ✅"
echo "  - Cleanup: $([ "$cleanup_confirm" = "yes" ] && echo "✅" || echo "⏭️  Skipped")"
echo "  - Final validation: ✅"
echo ""
echo "💡 Next steps:"
echo "  - Check your browsers to verify bookmarks are synced"
echo "  - Run 'browser-bookmark-sync validate' anytime to check integrity"
echo "  - Use 'browser-bookmark-sync sync --mode full' for full sync"
echo ""
