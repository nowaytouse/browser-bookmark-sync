#!/bin/bash
# 全量书签处理脚本 v2
set -e
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
DESKTOP=~/Desktop
BIN="./target/release/browser-bookmark-sync"
PROXY="http://127.0.0.1:6152"

INPUT1="${DESKTOP}/waterfox"
INPUT2="${DESKTOP}/FINAL_ORGANIZED_BOOKMARKS.html"
STEP1="${DESKTOP}/step1_${TIMESTAMP}.html"
MERGED="${DESKTOP}/merged_${TIMESTAMP}.html"
ORGANIZED="${DESKTOP}/organized_${TIMESTAMP}.html"
FINAL="${DESKTOP}/FINAL_CLEAN_${TIMESTAMP}.html"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📚 书签全量处理 - ${TIMESTAMP}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Step 1: 导出Brave + waterfox
echo ""
echo "📥 Step 1a: 导出Brave Nightly + waterfox..."
$BIN export --browsers brave-nightly --include "$INPUT1" --deduplicate --output "$STEP1"

# Step 1b: 合并第二个文件
echo "📥 Step 1b: 合并FINAL_ORGANIZED..."
$BIN export --browsers none --include "$INPUT2" --update "$STEP1" --output "$MERGED"

MERGE_COUNT=$(grep -c '<DT><A' "$MERGED" 2>/dev/null || echo "0")
echo "   合并后书签数: $MERGE_COUNT"

# Step 2: 智能整理
echo ""
echo "📂 Step 2: 智能整理分类..."
$BIN organize --file "$MERGED" --output "$ORGANIZED"

ORG_COUNT=$(grep -c '<DT><A' "$ORGANIZED" 2>/dev/null || echo "0")
echo "   整理后书签数: $ORG_COUNT"

# Step 3: 小规模死链测试
echo ""
echo "🧪 Step 3: 小规模死链测试 (100个)..."
$BIN check \
  --file "$ORGANIZED" \
  --output "${DESKTOP}/test_check_${TIMESTAMP}.html" \
  --proxy "$PROXY" \
  --timeout 15 \
  --concurrency 10 \
  --limit 100 \
  --export-dir "${DESKTOP}/test_results_${TIMESTAMP}"

echo ""
echo "✅ 小规模测试完成！"
echo "全量: $BIN check --file \"$ORGANIZED\" --output \"$FINAL\" --proxy \"$PROXY\" --timeout 15 --concurrency 10 --limit 0 --delete"
