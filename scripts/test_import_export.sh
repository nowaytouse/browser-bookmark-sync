#!/bin/bash
# Test HTML import/export preserves folder structure
# 测试HTML导入导出是否保留文件夹结构

set -e

SCRIPT_DIR="$(dirname "$0")"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BINARY="$PROJECT_ROOT/target/release/browser-bookmark-sync"
TEST_DIR="/tmp/import_export_test_$$"
mkdir -p "$TEST_DIR"

echo "🧪 测试HTML导入导出文件夹结构保留"
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
        </DL><p>
    </DL><p>
    <DT><H3 ADD_DATE="1734567896">🎮 娱乐</H3>
    <DL><p>
        <DT><A HREF="https://youtube.com" ADD_DATE="1734567897">YouTube</A>
    </DL><p>
    <DT><A HREF="https://github.com" ADD_DATE="1734567899">GitHub根级</A>
</DL><p>
EOF

echo "📄 输入文件结构:"
echo "   📁 技术文档"
echo "      - Rust官网"
echo "      - Rust文档"
echo "      📁 工具"
echo "         - Crates.io"
echo "   📁 娱乐"
echo "      - YouTube"
echo "   - GitHub根级"
echo ""

# 使用export命令导入并导出（不进行organize）
echo "🔄 执行导入导出测试 (export --include)..."
"$BINARY" export \
    --output "$TEST_DIR/output.html" \
    --include "$TEST_DIR/input.html" \
    --browsers none \
    --no-wrap \
    2>&1 | grep -E "📖|📁|书签|文件夹|解析|import" || true

echo ""
echo "📤 检查输出文件..."

if [ -f "$TEST_DIR/output.html" ]; then
    echo "✅ 输出文件已生成"
    
    # 统计
    FOLDER_COUNT=$(grep -ci "<H3" "$TEST_DIR/output.html" 2>/dev/null || echo "0")
    BOOKMARK_COUNT=$(grep -ci "<DT><A" "$TEST_DIR/output.html" 2>/dev/null || echo "0")
    
    echo "   文件夹数量: $FOLDER_COUNT (期望: 3)"
    echo "   书签数量: $BOOKMARK_COUNT (期望: 5)"
    
    # 检查关键文件夹
    echo ""
    echo "📋 文件夹检查:"
    for folder in "技术文档" "工具" "娱乐"; do
        if grep -q "$folder" "$TEST_DIR/output.html"; then
            echo "   ✅ 找到: $folder"
        else
            echo "   ❌ 缺失: $folder"
        fi
    done
    
    echo ""
    echo "📋 输出文件内容:"
    cat "$TEST_DIR/output.html"
else
    echo "❌ 输出文件未生成"
fi

# 清理
rm -rf "$TEST_DIR"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
