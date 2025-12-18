#!/bin/bash
# 完整的 Waterfox 书签清理流程
# 1. 导出原始书签
# 2. 整理分类
# 3. 检查死链并删除
# 4. 导出最终结果

set -e

DESKTOP=~/Desktop
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BSYNC="./target/release/browser-bookmark-sync"

echo "🚀 开始 Waterfox 书签完整清理流程..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Step 1: 导出原始书签
echo ""
echo "📤 Step 1: 导出原始书签..."
$BSYNC export -b waterfox -o "$DESKTOP/waterfox_raw_$TIMESTAMP.html"

# Step 2: 整理分类
echo ""
echo "🧠 Step 2: 整理分类..."
$BSYNC organize -b waterfox --stats

# Step 3: 检查死链并删除 (这一步会很慢)
echo ""
echo "🔍 Step 3: 检查死链..."
echo "⚠️  这一步可能需要 30-60 分钟，请耐心等待..."
$BSYNC check -b waterfox --delete --export-dir "$DESKTOP/check_results_$TIMESTAMP" -c 20

# Step 4: 导出最终结果
echo ""
echo "📤 Step 4: 导出最终清理后的书签..."
$BSYNC export -b waterfox -o "$DESKTOP/waterfox_cleaned_$TIMESTAMP.html"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 完成！文件已保存到桌面:"
echo "   - waterfox_raw_$TIMESTAMP.html (原始)"
echo "   - waterfox_cleaned_$TIMESTAMP.html (清理后)"
echo "   - check_results_$TIMESTAMP/ (检查结果)"
