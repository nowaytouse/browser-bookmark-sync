#!/bin/bash
# 全量书签处理脚本 v3
set -e
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
DESKTOP=~/Desktop
BIN="./target/release/browser-bookmark-sync"
PROXY="http://127.0.0.1:6152"

INPUT1="${DESKTOP}/waterfox"
INPUT2="${DESKTOP}/FINAL_ORGANIZED_BOOKMARKS.html"

TEMP_EXTRACT="${DESKTOP}/brave_temp_${TIMESTAMP}.html"
MERGED="${DESKTOP}/merged_${TIMESTAMP}.html"
ORGANIZED="${DESKTOP}/organized_${TIMESTAMP}.html"
FINAL="${DESKTOP}/FINAL_CLEAN_${TIMESTAMP}.html"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📚 书签全量处理 v3 - ${TIMESTAMP}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Step 1: 从 Brave Nightly 提取临时文件夹
echo ""
echo "📥 Step 1: 从 Brave Nightly 提取临时文件夹(👀临时)..."
$BIN export --browsers brave-nightly --folder "👀临时" --output "$TEMP_EXTRACT" --no-wrap

TEMP_COUNT=$(grep -c '<DT><A' "$TEMP_EXTRACT" 2>/dev/null || echo "0")
echo "   提取临时书签数: $TEMP_COUNT"

# Step 2: 合并所有来源 (使用 brave 作为占位浏览器)
echo ""
echo "📥 Step 2: 合并所有书签来源..."
$BIN export --browsers brave --include "$INPUT1" --deduplicate --output "${DESKTOP}/tmp1_${TIMESTAMP}.html"
$BIN export --browsers brave --include "$INPUT2" --update "${DESKTOP}/tmp1_${TIMESTAMP}.html" --output "${DESKTOP}/tmp2_${TIMESTAMP}.html"
$BIN export --browsers brave --include "$TEMP_EXTRACT" --update "${DESKTOP}/tmp2_${TIMESTAMP}.html" --output "$MERGED"

rm -f "${DESKTOP}/tmp1_${TIMESTAMP}.html" "${DESKTOP}/tmp2_${TIMESTAMP}.html"

MERGE_COUNT=$(grep -c '<DT><A' "$MERGED" 2>/dev/null || echo "0")
echo "   合并后书签数: $MERGE_COUNT"

# Step 3: 智能整理
echo ""
echo "📂 Step 3: 智能整理分类..."
$BIN organize --file "$MERGED" --output "$ORGANIZED"

ORG_COUNT=$(grep -c '<DT><A' "$ORGANIZED" 2>/dev/null || echo "0")
echo "   整理后书签数: $ORG_COUNT"

# Step 4: 小规模死链测试
echo ""
echo "🧪 Step 4: 小规模死链测试 (100个)..."
$BIN check \
  --file "$ORGANIZED" \
  --output "${DESKTOP}/test_check_${TIMESTAMP}.html" \
  --proxy "$PROXY" \
  --timeout 15 \
  --concurrency 10 \
  --limit 100 \
  --export-dir "${DESKTOP}/test_results_${TIMESTAMP}"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 小规模测试完成！"
echo "   临时文件夹: $TEMP_COUNT 个"
echo "   合并结果: $MERGE_COUNT 个"
echo "   整理结果: $ORG_COUNT 个"
echo ""
echo "⚠️  Brave Nightly 中的 👀临时 文件夹需手动删除"
echo ""
echo "🚀 全量命令:"
echo "$BIN check --file \"$ORGANIZED\" --output \"$FINAL\" --proxy \"$PROXY\" --timeout 15 --concurrency 10 --limit 0 --delete"
