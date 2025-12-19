#!/bin/bash
# Full bookmark processing script
# 完整书签处理脚本：导出 -> 整理 -> 检查死链 -> 清理

set -e

# Configuration
PROXY="http://127.0.0.1:6152"
OUTPUT_DIR="$HOME/Desktop/bookmark_process_$(date +%Y%m%d_%H%M%S)"
BROWSERS="waterfox,brave-nightly"
CHECK_LIMIT="${1:-500}"  # Default 500, can override with argument

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📚 Full Bookmark Processing"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Output: $OUTPUT_DIR"
echo "Browsers: $BROWSERS"
echo "Check limit: $CHECK_LIMIT URLs"
echo "Proxy: $PROXY"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

mkdir -p "$OUTPUT_DIR"

# Step 1: Export with flatten + dedupe + clean
echo ""
echo "📤 Step 1: Exporting bookmarks..."
cargo run --release -- export \
    -o "$OUTPUT_DIR/01_exported.html" \
    -b "$BROWSERS" \
    --flat \
    -d \
    --clean

# Step 2: Organize (classify + temp folder extraction)
echo ""
echo "🧠 Step 2: Organizing bookmarks..."
cargo run --release -- organize \
    --file "$OUTPUT_DIR/01_exported.html" \
    --output "$OUTPUT_DIR/02_organized.html" \
    --stats

# Step 3: Check dead links (with export)
echo ""
echo "🔍 Step 3: Checking dead links (limit: $CHECK_LIMIT)..."
cargo run --release -- check \
    -b "$BROWSERS" \
    --proxy "$PROXY" \
    --limit "$CHECK_LIMIT" \
    --export-dir "$OUTPUT_DIR/03_check_results" \
    --dry-run

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ Processing complete!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Output files:"
echo "  📄 $OUTPUT_DIR/01_exported.html"
echo "  📄 $OUTPUT_DIR/02_organized.html"
echo "  📁 $OUTPUT_DIR/03_check_results/"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
