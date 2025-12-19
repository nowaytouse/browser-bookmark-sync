#!/bin/bash
# Test check command preserves folder structure in export
set -e

BINARY="$(dirname "$0")/../target/release/browser-bookmark-sync"
TEST_DIR="/tmp/check_structure_test_$$"
mkdir -p "$TEST_DIR"

echo "🧪 测试check命令文件夹结构保留"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 创建测试HTML
cat > "$TEST_DIR/input.html" << 'EOF'
<!DOCTYPE NETSCAPE-Bookmark-file-1>
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3>📁 技术</H3>
    <DL><p>
        <DT><A HREF="https://github.com">GitHub</A>
        <DT><H3>🔧 工具</H3>
        <DL><p>
            <DT><A HREF="https://google.com">Google</A>
        </DL><p>
    </DL><p>
    <DT><H3>🎮 娱乐</H3>
    <DL><p>
        <DT><A HREF="https://youtube.com">YouTube</A>
    </DL><p>
</DL><p>
EOF

echo "📄 输入: 3文件夹, 3书签"
echo ""

# 运行check命令，限制1个URL，导出到目录
"$BINARY" check \
    --file "$TEST_DIR/input.html" \
    --output "$TEST_DIR/output.html" \
    --export-dir "$TEST_DIR/results" \
    --limit 3 \
    --timeout 5 \
    2>&1 | grep -E "📖|文件夹|书签|解析" || true

echo ""
echo "📤 检查导出结果..."

for f in valid.html invalid.html uncertain.html skipped.html; do
    if [ -f "$TEST_DIR/results/$f" ]; then
        FOLDERS=$(grep -ci "<H3" "$TEST_DIR/results/$f" 2>/dev/null || echo "0")
        BOOKMARKS=$(grep -ci "<DT><A" "$TEST_DIR/results/$f" 2>/dev/null || echo "0")
        echo "   $f: $FOLDERS 文件夹, $BOOKMARKS 书签"
    fi
done

echo ""
echo "📋 valid.html 内容预览:"
if [ -f "$TEST_DIR/results/valid.html" ]; then
    cat "$TEST_DIR/results/valid.html"
else
    echo "   (文件不存在)"
fi

rm -rf "$TEST_DIR"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
