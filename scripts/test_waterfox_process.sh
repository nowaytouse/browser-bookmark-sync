#!/bin/bash
# 测试从waterfox HTML文件处理书签
# 小规模测试 - 验证wrap和临时文件夹合并功能

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BSYNC="$PROJECT_DIR/target/release/browser-bookmark-sync"

# 如果不存在，尝试编译
if [ ! -f "$BSYNC" ]; then
    echo "🔨 编译中..."
    (cd "$PROJECT_DIR" && cargo build --release 2>&1 | tail -5)
fi

# 输入文件
INPUT_FILE="$HOME/Desktop/waterfox"
OUTPUT_DIR="$HOME/Desktop/bookmark_test_$(date +%Y%m%d_%H%M%S)"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📁 Waterfox书签处理测试"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "输入: $INPUT_FILE"
echo "输出目录: $OUTPUT_DIR"
echo ""

# 检查输入文件
if [ ! -f "$INPUT_FILE" ]; then
    echo "❌ 错误: 输入文件不存在: $INPUT_FILE"
    exit 1
fi

# 创建输出目录
mkdir -p "$OUTPUT_DIR"

# 统计输入文件
INPUT_COUNT=$(grep -c "HREF=" "$INPUT_FILE" 2>/dev/null || echo "0")
echo "📊 输入文件书签数: $INPUT_COUNT"
echo ""

# Step 1: 整理书签 (organize)
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧠 Step 1: 整理书签..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ORGANIZED_FILE="$OUTPUT_DIR/01_organized.html"
"$BSYNC" organize --file "$INPUT_FILE" --output "$ORGANIZED_FILE" --stats -V

# 统计整理后
ORGANIZED_COUNT=$(grep -c "HREF=" "$ORGANIZED_FILE" 2>/dev/null || echo "0")
echo ""
echo "📊 整理后书签数: $ORGANIZED_COUNT"
echo ""

# Step 2: 验证输出结构
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🔍 Step 2: 验证输出结构..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 检查顶层文件夹
echo "顶层文件夹:"
grep -E "^    <DT><H3" "$ORGANIZED_FILE" | head -10 || echo "(无)"

# 检查是否有📁镜像文件夹
if grep -q "📁镜像文件夹" "$ORGANIZED_FILE"; then
    echo "✅ 找到 📁镜像文件夹"
else
    echo "❌ 未找到 📁镜像文件夹"
fi

# 检查是否有👀临时
TEMP_COUNT=$(grep -c "👀临时" "$ORGANIZED_FILE" 2>/dev/null || echo "0")
echo "👀临时文件夹数量: $TEMP_COUNT"
if [ "$TEMP_COUNT" -gt 1 ]; then
    echo "⚠️  警告: 存在多个临时文件夹，应该合并为1个!"
elif [ "$TEMP_COUNT" -eq 1 ]; then
    echo "✅ 临时文件夹已正确合并"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 测试完成"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "输出文件: $ORGANIZED_FILE"
echo ""
echo "下一步: 检查输出文件结构是否符合预期"
echo "  - 顶层应只有: 📁镜像文件夹 和 👀临时"
echo "  - 所有分类文件夹应在 📁镜像文件夹 内"
