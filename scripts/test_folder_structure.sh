#!/bin/bash
# Test folder structure preservation in HTML import/export
# 测试HTML导入导出时文件夹结构是否正确保留

set -e

SCRIPT_DIR="$(dirname "$0")"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BINARY="$PROJECT_ROOT/target/release/browser-bookmark-sync"
TEST_DIR="/tmp/folder_structure_test_$$"
mkdir -p "$TEST_DIR"

echo "🧪 测试文件夹结构保留"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 创建测试HTML文件（带文件夹结构）
cat > "$TEST_DIR/input.html" << 'EOF'
<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3 ADD_DATE="1734567890">📁 技术文档</H3>
    <DL><p>
        <DT><A HREF="https://rust-lang.org" ADD_DATE="1734567891">Rust官网</A>
        <DT><A HREF="https://doc.rust-lang.org" ADD_DATE="1734567892">Rust文档</A>
        <DT><H3 ADD_DATE="1734567893">🔧 工具</H3>
        <DL><p>
            <DT><A HREF="https://crates.io" ADD_DATE="1734567894">Crates.io</A>
            <DT><A HREF="https://docs.rs" ADD_DATE="1734567895">Docs.rs</A>
        </DL><p>
    </DL><p>
    <DT><H3 ADD_DATE="1734567896">🎮 娱乐</H3>
    <DL><p>
        <DT><A HREF="https://youtube.com" ADD_DATE="1734567897">YouTube</A>
        <DT><A HREF="https://bilibili.com" ADD_DATE="1734567898">Bilibili</A>
    </DL><p>
    <DT><A HREF="https://github.com" ADD_DATE="1734567899">GitHub (根级)</A>
</DL><p>
EOF

echo "📄 输入文件结构:"
echo "   📁 技术文档"
echo "      - Rust官网"
echo "      - Rust文档"
echo "      📁 工具"
echo "         - Crates.io"
echo "         - Docs.rs"
echo "   📁 娱乐"
echo "      - YouTube"
echo "      - Bilibili"
echo "   - GitHub (根级)"
echo ""

# 运行organize命令（会触发import和export）
echo "🔄 执行导入导出测试..."
"$BINARY" organize \
    --file "$TEST_DIR/input.html" \
    --output "$TEST_DIR/output.html" \
    2>&1 | grep -E "📖|📁|书签|文件夹|解析" || true

echo ""
echo "📤 检查输出文件..."

# 检查输出文件中是否包含文件夹标签
if [ -f "$TEST_DIR/output.html" ]; then
    echo "✅ 输出文件已生成"
    
    # 统计文件夹数量
    FOLDER_COUNT=$(grep -c "<H3" "$TEST_DIR/output.html" 2>/dev/null || echo "0")
    BOOKMARK_COUNT=$(grep -c "<DT><A" "$TEST_DIR/output.html" 2>/dev/null || echo "0")
    DL_COUNT=$(grep -c "<DL>" "$TEST_DIR/output.html" 2>/dev/null || echo "0")
    
    echo "   文件夹数量: $FOLDER_COUNT (期望: 3)"
    echo "   书签数量: $BOOKMARK_COUNT (期望: 7)"
    echo "   DL标签数量: $DL_COUNT (期望: 4+)"
    
    # 检查关键文件夹是否存在
    if grep -q "技术文档" "$TEST_DIR/output.html"; then
        echo "   ✅ 找到文件夹: 技术文档"
    else
        echo "   ❌ 缺失文件夹: 技术文档"
    fi
    
    if grep -q "工具" "$TEST_DIR/output.html"; then
        echo "   ✅ 找到文件夹: 工具"
    else
        echo "   ❌ 缺失文件夹: 工具"
    fi
    
    if grep -q "娱乐" "$TEST_DIR/output.html"; then
        echo "   ✅ 找到文件夹: 娱乐"
    else
        echo "   ❌ 缺失文件夹: 娱乐"
    fi
    
    echo ""
    echo "📋 输出文件内容预览:"
    head -40 "$TEST_DIR/output.html"
else
    echo "❌ 输出文件未生成"
fi

# 清理
rm -rf "$TEST_DIR"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 测试完成"
