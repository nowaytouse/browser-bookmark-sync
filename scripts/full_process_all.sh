#!/bin/bash
# 全量书签处理脚本 - 导出 + 整理 + 死链检查 + 清理
# Full bookmark processing: export → organize → dead link check → cleanup

set -e

# 配置
PROXY="http://127.0.0.1:6152"
OUTPUT_DIR="$HOME/Desktop/bookmark_full_$(date +%Y%m%d_%H%M%S)"
BROWSERS="brave-nightly"
# 桌面上的额外书签文件
EXTRA_HTML="$HOME/Desktop/waterfox"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📚 全量书签处理 (Full Bookmark Processing)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "输出目录: $OUTPUT_DIR"
echo "浏览器: $BROWSERS"
echo "额外文件: $EXTRA_HTML"
echo "代理: $PROXY"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

mkdir -p "$OUTPUT_DIR"

# Step 1: 导出 (flatten + dedupe + clean + wrap默认开启)
# 结构: 📁镜像文件夹 + 👀临时 (顶层仅两个文件夹)
echo ""
echo "📤 Step 1: 导出书签 (Export with flatten + dedupe + clean + wrap)..."
./target/release/browser-bookmark-sync export \
    -o "$OUTPUT_DIR/01_exported.html" \
    -b "$BROWSERS" \
    --include "$EXTRA_HTML" \
    --flat \
    -d \
    --clean \
    -v

# Step 2: 整理分类 (organize + temp folder extraction)
echo ""
echo "🧠 Step 2: 智能整理分类 (Organize + classify)..."
./target/release/browser-bookmark-sync organize \
    --file "$OUTPUT_DIR/01_exported.html" \
    --output "$OUTPUT_DIR/02_organized.html" \
    --stats \
    -V

# Step 3: 全量死链检查 (dual-network: proxy + direct)
echo ""
echo "🔍 Step 3: 全量死链检查 (Full dead link check - NO LIMIT)..."
echo "⚠️  警告: 全量检查可能需要2小时以上!"
./target/release/browser-bookmark-sync check \
    -b "$BROWSERS" \
    --proxy "$PROXY" \
    --limit 0 \
    --concurrency 5 \
    --timeout 15 \
    --export-dir "$OUTPUT_DIR/03_check_results" \
    --dry-run \
    -v

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 全量处理完成!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "输出文件:"
echo "  📄 $OUTPUT_DIR/01_exported.html (导出+去重+扁平化+镜像文件夹包装)"
echo "  📄 $OUTPUT_DIR/02_organized.html (智能分类)"
echo "  📁 $OUTPUT_DIR/03_check_results/ (死链检查结果)"
echo "     - valid.html (有效)"
echo "     - invalid.html (无效)"
echo "     - uncertain.html (不确定)"
echo "     - skipped.html (跳过)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
