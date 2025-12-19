#!/bin/bash
# Full Bookmark Processing v5 - 优化网络性能，减少不确定
set -e

BINARY="$(dirname "$0")/../target/release/browser-bookmark-sync"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
OUTPUT_DIR="$HOME/Desktop/bookmark_v5_${TIMESTAMP}"

export https_proxy=http://127.0.0.1:6152
export http_proxy=http://127.0.0.1:6152
export all_proxy=socks5://127.0.0.1:6153

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📚 全量书签处理 v5 (优化版) - $TIMESTAMP"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "优化: 403/503视为有效 (服务器在线)"
echo ""

mkdir -p "$OUTPUT_DIR"

# Step 1: 提取Brave临时文件夹
echo "📥 Step 1: 提取 Brave Nightly 👀临时..."
TEMP_BRAVE="$OUTPUT_DIR/01_brave_temp.html"
"$BINARY" export -o "$TEMP_BRAVE" -b "brave-nightly" -f "👀临时" --no-wrap 2>&1 | grep -E "✅|书签" || true

# Step 2: 合并所有来源
echo ""
echo "📥 Step 2: 合并书签来源..."
MERGED="$OUTPUT_DIR/02_merged.html"

# 先用Brave临时作为基础
cp "$TEMP_BRAVE" "$MERGED" 2>/dev/null || touch "$MERGED"

# 合并FINAL文件
if [ -f "$HOME/Desktop/FINAL_ORGANIZED_BOOKMARKS.html" ]; then
    echo "   合并: FINAL_ORGANIZED_BOOKMARKS.html"
    "$BINARY" export -o "$MERGED" -b "none" \
        --include "$HOME/Desktop/FINAL_ORGANIZED_BOOKMARKS.html" \
        --update "$MERGED" --no-wrap -d 2>&1 | grep -E "✅|new|书签" || true
fi

COUNT=$(grep -c "<DT><A" "$MERGED" 2>/dev/null || echo "0")
echo "   ✅ 合并后: $COUNT 书签"

# Step 3: 智能整理
echo ""
echo "🧠 Step 3: 智能整理..."
ORGANIZED="$OUTPUT_DIR/03_organized.html"
"$BINARY" organize --file "$MERGED" --output "$ORGANIZED" 2>&1 | grep -E "📁|✅" | head -20 || true

ORG_COUNT=$(grep -c "<DT><A" "$ORGANIZED" 2>/dev/null || echo "0")
echo "   ✅ 整理后: $ORG_COUNT 书签"

# Step 4: 全量死链检查 (优化参数)
echo ""
echo "🔍 Step 4: 全量死链检查 (优化: timeout=30, concurrency=20)..."
RESULT_DIR="$OUTPUT_DIR/check_results"
mkdir -p "$RESULT_DIR"

"$BINARY" check \
    --file "$ORGANIZED" \
    --output "$OUTPUT_DIR/FINAL_VALID.html" \
    --proxy "http://127.0.0.1:6152" \
    --timeout 15 \
    --concurrency 10 \
    --limit 0 \
    --export-dir "$RESULT_DIR" \
    2>&1 | tail -25

# 统计
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 处理完成"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "输出: $OUTPUT_DIR"
for f in "$RESULT_DIR"/*.html; do
    [ -f "$f" ] && echo "   $(basename $f): $(grep -c '<DT><A' "$f" 2>/dev/null || echo 0) 书签"
done
echo ""
echo "✅ 完成!"
