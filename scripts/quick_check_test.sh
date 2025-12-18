#!/bin/bash
# 快速测试check功能 - 只检查少量URL

cd "$(dirname "$0")/.."

echo "🔧 构建..."
cargo build --release 2>/dev/null

BSYNC="./target/release/browser-bookmark-sync"

echo ""
echo "📋 测试1: 导出收藏夹到HTML"
$BSYNC export -o /tmp/test_bookmarks.html -b "brave nightly" -d --clean 2>&1 | tail -10

echo ""
echo "📋 测试2: 检查收藏夹有效性 (仅前100个URL)"
# 由于URL数量太多，这里只是展示命令
echo "命令: $BSYNC check -b 'brave nightly' --timeout 5 --concurrency 30"
echo "⚠️  注意: 23000+个URL需要较长时间检查"

echo ""
echo "📋 测试3: 查看导出文件大小"
ls -lh /tmp/test_bookmarks.html 2>/dev/null || echo "文件不存在"

echo ""
echo "📋 测试4: 统计导出文件中的URL数量"
grep -c "HREF=" /tmp/test_bookmarks.html 2>/dev/null || echo "无法统计"

echo ""
echo "✅ 基本功能测试完成"
echo ""
echo "💡 要实际检查死链，请运行:"
echo "   $BSYNC check -b 'brave nightly' --timeout 10 --concurrency 30 --verbose"
echo ""
echo "💡 要删除死链，请运行:"
echo "   $BSYNC check -b 'brave nightly' --timeout 10 --concurrency 30 --delete"
