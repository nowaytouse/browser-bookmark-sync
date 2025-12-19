#!/bin/bash
# Quick Temp Folder Process - 一键处理临时文件夹
# 用法: ./quick_temp.sh [额外HTML文件...]
# 从所有浏览器的👀临时文件夹提取 + 额外HTML → 整理 → 死链检查 → 合并输出
set -e

SCRIPT_DIR="$(dirname "$0")"
BINARY="$SCRIPT_DIR/../target/release/browser-bookmark-sync"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
OUTPUT="$HOME/Desktop/temp_processed_${TIMESTAMP}.html"
WORK_DIR="/tmp/bsync_temp_$$"

export https_proxy=http://127.0.0.1:6152
export http_proxy=http://127.0.0.1:6152

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🚀 临时文件夹快速处理"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

mkdir -p "$WORK_DIR"

# Step 1: 从所有浏览器提取临时文件夹
echo "📥 从所有浏览器提取 👀临时..."
"$BINARY" export -o "$WORK_DIR/temp.html" -b "all" -f "👀临时" --no-wrap -d 2>&1 | grep -E "✅|书签" || true

# 合并额外HTML（变体浏览器导出）
for EXTRA in "$@"; do
    if [ -f "$EXTRA" ]; then
        echo "   📄 合并: $(basename "$EXTRA")"
        "$BINARY" export -o "$WORK_DIR/temp.html" -b "none" --include "$EXTRA" --update "$WORK_DIR/temp.html" --no-wrap -d 2>&1 | grep "new" || true
    fi
done

COUNT=$(grep -c '<DT><A' "$WORK_DIR/temp.html" 2>/dev/null || echo 0)
echo "   总计: $COUNT 书签"

[ "$COUNT" -eq 0 ] && echo "⚠️ 无书签" && rm -rf "$WORK_DIR" && exit 0

# Step 2: 整理
echo "🧠 整理..."
"$BINARY" organize --file "$WORK_DIR/temp.html" --output "$WORK_DIR/organized.html" 2>&1 | grep "📁" | head -10 || true

# Step 3: 死链检查
echo "🔍 检查死链..."
mkdir -p "$WORK_DIR/results"
"$BINARY" check --file "$WORK_DIR/organized.html" --proxy "http://127.0.0.1:6152" --timeout 15 --concurrency 10 --limit 0 --export-dir "$WORK_DIR/results" 2>&1 | grep -E "✅|❌|❓" || true

# Step 4: 合并
echo "📦 合并..."
"$SCRIPT_DIR/merge_results.sh" "$WORK_DIR/results" "$OUTPUT" 2>&1 | tail -3

rm -rf "$WORK_DIR"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 完成: $OUTPUT"
