#!/bin/bash
# Merge check results into single HTML
set -e

RESULT_DIR="$1"
OUTPUT="$2"

if [ -z "$RESULT_DIR" ] || [ -z "$OUTPUT" ]; then
    echo "Usage: $0 <result_dir> <output.html>"
    exit 1
fi

echo "📦 合并检查结果到: $OUTPUT"

# 提取内容，跳过外层📁镜像文件夹包装
# 源文件结构: 9行头 + <H3>📁镜像文件夹 + <DL><p> + 内容 + </DL><p> + </DL><p>
extract_inner() {
    local file="$1"
    if [ -f "$file" ]; then
        local total=$(wc -l < "$file" | tr -d ' ')
        # 跳过前10行(头+📁镜像文件夹+<DL><p>)，去掉最后2行(</DL><p></DL><p>)
        local content_lines=$((total - 10 - 2))
        if [ $content_lines -gt 0 ]; then
            tail -n +11 "$file" | head -n $content_lines
        fi
    fi
}

# HTML头
cat > "$OUTPUT" << 'EOF'
<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3>📁镜像文件夹</H3>
    <DL><p>
EOF

# 有效
if [ -f "$RESULT_DIR/valid.html" ]; then
    C=$(grep -c '<DT><A' "$RESULT_DIR/valid.html" 2>/dev/null || echo 0)
    echo "   ✅ 有效: $C"
    echo '        <DT><H3>✅ 有效</H3>' >> "$OUTPUT"
    echo '        <DL><p>' >> "$OUTPUT"
    extract_inner "$RESULT_DIR/valid.html" >> "$OUTPUT"
    echo '        </DL><p>' >> "$OUTPUT"
fi

# 无效
if [ -f "$RESULT_DIR/invalid.html" ]; then
    C=$(grep -c '<DT><A' "$RESULT_DIR/invalid.html" 2>/dev/null || echo 0)
    echo "   ❌ 无效: $C"
    echo '        <DT><H3>❌ 无效</H3>' >> "$OUTPUT"
    echo '        <DL><p>' >> "$OUTPUT"
    extract_inner "$RESULT_DIR/invalid.html" >> "$OUTPUT"
    echo '        </DL><p>' >> "$OUTPUT"
fi

# 不确定
if [ -f "$RESULT_DIR/uncertain.html" ]; then
    C=$(grep -c '<DT><A' "$RESULT_DIR/uncertain.html" 2>/dev/null || echo 0)
    echo "   ❓ 不确定: $C"
    echo '        <DT><H3>❓ 不确定</H3>' >> "$OUTPUT"
    echo '        <DL><p>' >> "$OUTPUT"
    extract_inner "$RESULT_DIR/uncertain.html" >> "$OUTPUT"
    echo '        </DL><p>' >> "$OUTPUT"
fi

# 跳过
if [ -f "$RESULT_DIR/skipped.html" ]; then
    C=$(grep -c '<DT><A' "$RESULT_DIR/skipped.html" 2>/dev/null || echo 0)
    echo "   ⏭️ 跳过: $C"
    echo '        <DT><H3>⏭️ 跳过</H3>' >> "$OUTPUT"
    echo '        <DL><p>' >> "$OUTPUT"
    extract_inner "$RESULT_DIR/skipped.html" >> "$OUTPUT"
    echo '        </DL><p>' >> "$OUTPUT"
fi

# 临时文件夹 (放一个占位书签，防止浏览器删除空文件夹)
echo "   👀 临时: (占位)"
echo '        <DT><H3>👀 临时</H3>' >> "$OUTPUT"
echo '        <DL><p>' >> "$OUTPUT"
echo '            <DT><A HREF="about:blank">📌 临时书签放这里</A>' >> "$OUTPUT"
echo '        </DL><p>' >> "$OUTPUT"

# HTML尾
cat >> "$OUTPUT" << 'EOF'
    </DL><p>
</DL><p>
EOF

TOTAL=$(grep -c '<DT><A' "$OUTPUT" 2>/dev/null || echo 0)
echo "✅ 合并完成: $TOTAL 书签"
