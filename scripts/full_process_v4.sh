#!/bin/bash
# Full Bookmark Processing Script v4
# 全量书签处理脚本 - 合并、整理、去重、死链检查
set -e

SCRIPT_DIR="$(dirname "$0")"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BINARY="$PROJECT_ROOT/target/release/browser-bookmark-sync"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
OUTPUT_DIR="$HOME/Desktop/bookmark_process_${TIMESTAMP}"

# 代理配置
export https_proxy=http://127.0.0.1:6152
export http_proxy=http://127.0.0.1:6152
export all_proxy=socks5://127.0.0.1:6153

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📚 全量书签处理 v4 - $TIMESTAMP"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

mkdir -p "$OUTPUT_DIR"

# ============================================================
# Step 1: 提取 Brave Nightly 👀临时 文件夹
# ============================================================
echo ""
echo "📥 Step 1: 提取 Brave Nightly 👀临时 文件夹..."
TEMP_BRAVE="$OUTPUT_DIR/01_brave_temp.html"

"$BINARY" export \
    --output "$TEMP_BRAVE" \
    --browsers "brave-nightly" \
    --folder "👀临时" \
    --no-wrap \
    2>&1 | grep -E "📤|✅|书签|folder" || true

if [ -f "$TEMP_BRAVE" ]; then
    COUNT=$(grep -c "<DT><A" "$TEMP_BRAVE" 2>/dev/null || echo "0")
    echo "   ✅ 提取了 $COUNT 个书签"
else
    echo "   ⚠️  未找到临时文件夹或为空"
    touch "$TEMP_BRAVE"
fi

# ============================================================
# Step 2: 合并所有书签来源
# ============================================================
echo ""
echo "📥 Step 2: 合并所有书签来源..."
MERGED="$OUTPUT_DIR/02_merged.html"

# 检查桌面文件
WATERFOX_FILE="$HOME/Desktop/waterfox"
FINAL_FILE="$HOME/Desktop/FINAL_ORGANIZED_BOOKMARKS.html"

INCLUDE_ARGS=""
if [ -f "$WATERFOX_FILE" ]; then
    INCLUDE_ARGS="--include $WATERFOX_FILE"
    echo "   📄 包含: waterfox"
fi

# 合并: Brave临时 + 桌面文件
"$BINARY" export \
    --output "$MERGED" \
    --browsers "none" \
    --include "$TEMP_BRAVE" \
    --no-wrap \
    -d \
    2>&1 | grep -E "📤|✅|import|书签" || true

# 如果有FINAL文件，再合并一次
if [ -f "$FINAL_FILE" ]; then
    echo "   📄 合并: FINAL_ORGANIZED_BOOKMARKS.html"
    "$BINARY" export \
        --output "$MERGED" \
        --browsers "none" \
        --include "$FINAL_FILE" \
        --update "$MERGED" \
        --no-wrap \
        -d \
        2>&1 | grep -E "📤|✅|import|书签|new" || true
fi

MERGED_COUNT=$(grep -c "<DT><A" "$MERGED" 2>/dev/null || echo "0")
echo "   ✅ 合并后: $MERGED_COUNT 个书签"

# ============================================================
# Step 3: 智能整理分类
# ============================================================
echo ""
echo "🧠 Step 3: 智能整理分类..."
ORGANIZED="$OUTPUT_DIR/03_organized.html"

"$BINARY" organize \
    --file "$MERGED" \
    --output "$ORGANIZED" \
    --stats \
    2>&1 | grep -E "📁|✅|分类|书签|classify" || true

ORG_COUNT=$(grep -c "<DT><A" "$ORGANIZED" 2>/dev/null || echo "0")
FOLDER_COUNT=$(grep -c "<H3" "$ORGANIZED" 2>/dev/null || echo "0")
echo "   ✅ 整理后: $ORG_COUNT 书签, $FOLDER_COUNT 文件夹"

# ============================================================
# Step 4: 小规模死链测试 (100个)
# ============================================================
echo ""
echo "🧪 Step 4: 小规模死链测试 (100个)..."
TEST_RESULT="$OUTPUT_DIR/04_test_check"
mkdir -p "$TEST_RESULT"

"$BINARY" check \
    --file "$ORGANIZED" \
    --proxy "http://127.0.0.1:6152" \
    --timeout 10 \
    --concurrency 5 \
    --limit 100 \
    --export-dir "$TEST_RESULT" \
    2>&1 | grep -E "✅|❌|❓|有效|无效|不确定" || true

echo "   测试结果:"
for f in valid.html invalid.html uncertain.html skipped.html; do
    if [ -f "$TEST_RESULT/$f" ]; then
        C=$(grep -c "<DT><A" "$TEST_RESULT/$f" 2>/dev/null || echo "0")
        echo "      $f: $C"
    fi
done

# ============================================================
# Step 5: 全量死链检查
# ============================================================
echo ""
echo "🔍 Step 5: 全量死链检查 (可能需要较长时间)..."
FULL_RESULT="$OUTPUT_DIR/05_full_check"
mkdir -p "$FULL_RESULT"

"$BINARY" check \
    --file "$ORGANIZED" \
    --output "$OUTPUT_DIR/FINAL_VALID.html" \
    --proxy "http://127.0.0.1:6152" \
    --timeout 15 \
    --concurrency 10 \
    --limit 0 \
    --export-dir "$FULL_RESULT" \
    2>&1 | tail -20

# ============================================================
# 最终统计
# ============================================================
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 处理完成 - 最终统计"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "输出目录: $OUTPUT_DIR"
echo ""

for f in "$FULL_RESULT"/*.html; do
    if [ -f "$f" ]; then
        NAME=$(basename "$f")
        BOOKMARKS=$(grep -c "<DT><A" "$f" 2>/dev/null || echo "0")
        FOLDERS=$(grep -c "<H3" "$f" 2>/dev/null || echo "0")
        SIZE=$(ls -lh "$f" | awk '{print $5}')
        echo "   $NAME: $BOOKMARKS 书签, $FOLDERS 文件夹 ($SIZE)"
    fi
done

if [ -f "$OUTPUT_DIR/FINAL_VALID.html" ]; then
    FINAL_B=$(grep -c "<DT><A" "$OUTPUT_DIR/FINAL_VALID.html" 2>/dev/null || echo "0")
    FINAL_F=$(grep -c "<H3" "$OUTPUT_DIR/FINAL_VALID.html" 2>/dev/null || echo "0")
    echo ""
    echo "🎯 最终有效书签: $FINAL_B 书签, $FINAL_F 文件夹"
fi

echo ""
echo "✅ 全部完成!"
