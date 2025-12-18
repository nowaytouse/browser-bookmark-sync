#!/bin/bash
# 测试文件夹结构保持功能
# 使用 Waterfox 书签文件进行测试

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BINARY="$PROJECT_DIR/target/release/browser-bookmark-sync"
TEST_FILE="/Users/nyamiiko/Desktop/waterfox"
OUTPUT_DIR="/tmp/bookmark_test_$(date +%Y%m%d_%H%M%S)"

echo "🧪 测试文件夹结构保持功能"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "测试文件: $TEST_FILE"
echo "输出目录: $OUTPUT_DIR"
echo ""

# 检查测试文件是否存在
if [ ! -f "$TEST_FILE" ]; then
    echo "❌ 测试文件不存在: $TEST_FILE"
    exit 1
fi

# 检查二进制文件
if [ ! -f "$BINARY" ]; then
    echo "⚠️  编译 release 版本..."
    cargo build --release -p browser-bookmark-sync
fi

mkdir -p "$OUTPUT_DIR"

# 复制测试文件
cp "$TEST_FILE" "$OUTPUT_DIR/original.html"
echo "✅ 复制原始文件到: $OUTPUT_DIR/original.html"

# 统计原始文件的文件夹结构
echo ""
echo "📊 原始文件统计:"
echo "   文件夹数量: $(grep -c '<DT><H3' "$TEST_FILE" || echo 0)"
echo "   书签数量: $(grep -c '<DT><A' "$TEST_FILE" || echo 0)"

# 测试1: Dry-run 模式检查
echo ""
echo "🔍 测试1: Dry-run 模式检查 (限制10个URL)"
$BINARY check --dry-run --limit 10 --export-dir "$OUTPUT_DIR/dry_run" 2>&1 | head -30 || true

# 检查导出的文件是否保持了文件夹结构
if [ -d "$OUTPUT_DIR/dry_run" ]; then
    echo ""
    echo "📁 导出文件检查:"
    for f in "$OUTPUT_DIR/dry_run"/*.html; do
        if [ -f "$f" ]; then
            name=$(basename "$f")
            folders=$(grep -c '<DT><H3' "$f" 2>/dev/null || echo 0)
            bookmarks=$(grep -c '<DT><A' "$f" 2>/dev/null || echo 0)
            echo "   $name: $folders 个文件夹, $bookmarks 个书签"
        fi
    done
fi

echo ""
echo "✅ 测试完成"
echo "输出目录: $OUTPUT_DIR"
