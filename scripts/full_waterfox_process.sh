#!/bin/bash
# 完整处理 Waterfox 书签文件
# 功能: 整理 + 去重 + 死链检查 + 输出到桌面
# 
# 使用方法:
#   ./scripts/full_waterfox_process.sh [--full]
#   
#   默认: 小规模测试 (100个URL)
#   --full: 全量处理 (所有URL)

set -e

# 配置
INPUT_FILE="/Users/nyamiiko/Desktop/waterfox"
OUTPUT_DIR="/Users/nyamiiko/Desktop/waterfox_processed"
PROXY="http://127.0.0.1:6152"
TIMEOUT=15
CONCURRENCY=8

# 解析参数
FULL_MODE=false
LIMIT=100
if [ "$1" = "--full" ]; then
    FULL_MODE=true
    LIMIT=0
    echo "⚠️  全量处理模式 - 这可能需要很长时间!"
fi

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}🔧 Waterfox 书签完整处理流程${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${CYAN}输入文件: $INPUT_FILE${NC}"
echo -e "${CYAN}输出目录: $OUTPUT_DIR${NC}"
echo -e "${CYAN}代理: $PROXY${NC}"
if [ "$FULL_MODE" = true ]; then
    echo -e "${YELLOW}模式: 全量处理${NC}"
else
    echo -e "${CYAN}模式: 小规模测试 (限制 $LIMIT 个URL)${NC}"
fi
echo ""

# 检查输入文件
if [ ! -f "$INPUT_FILE" ]; then
    echo -e "${RED}❌ 输入文件不存在: $INPUT_FILE${NC}"
    exit 1
fi

# 创建输出目录
mkdir -p "$OUTPUT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# 获取原始书签数量
ORIGINAL_COUNT=$(grep -c '<DT><A' "$INPUT_FILE" || echo "0")
echo -e "${GREEN}📖 原始书签数量: $ORIGINAL_COUNT${NC}"
echo ""

# ============================================================
# Step 1: 智能整理书签
# ============================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}📁 Step 1: 智能整理书签${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

ORGANIZED_FILE="$OUTPUT_DIR/01_organized_$TIMESTAMP.html"
./target/release/browser-bookmark-sync organize \
    --file "$INPUT_FILE" \
    --output "$ORGANIZED_FILE" \
    --stats

ORGANIZED_COUNT=$(grep -c '<DT><A' "$ORGANIZED_FILE" || echo "0")
echo -e "${GREEN}✅ 整理后书签数量: $ORGANIZED_COUNT${NC}"
echo ""

# ============================================================
# Step 2: 死链检查
# ============================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}🔍 Step 2: 死链检查 (代理+直连双重验证)${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

CHECK_OUTPUT_DIR="$OUTPUT_DIR/check_results_$TIMESTAMP"
VALID_FILE="$OUTPUT_DIR/02_valid_$TIMESTAMP.html"

# 设置代理环境变量
export https_proxy=$PROXY
export http_proxy=$PROXY
export all_proxy=socks5://127.0.0.1:6153

./target/release/browser-bookmark-sync check \
    --file "$ORGANIZED_FILE" \
    --output "$VALID_FILE" \
    --proxy "$PROXY" \
    --timeout $TIMEOUT \
    --concurrency $CONCURRENCY \
    --limit $LIMIT \
    --export-dir "$CHECK_OUTPUT_DIR" \
    -v

VALID_COUNT=$(grep -c '<DT><A' "$VALID_FILE" 2>/dev/null || echo "0")
echo -e "${GREEN}✅ 有效书签数量: $VALID_COUNT${NC}"
echo ""

# ============================================================
# Step 3: 最终输出
# ============================================================
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}📤 Step 3: 最终输出${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# 复制最终文件到桌面
FINAL_FILE="/Users/nyamiiko/Desktop/waterfox_clean_$TIMESTAMP.html"
cp "$VALID_FILE" "$FINAL_FILE"

echo -e "${GREEN}✅ 最终文件: $FINAL_FILE${NC}"
echo ""

# ============================================================
# 统计摘要
# ============================================================
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}📊 处理完成摘要${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "  原始书签:     ${CYAN}$ORIGINAL_COUNT${NC}"
echo -e "  整理后:       ${CYAN}$ORGANIZED_COUNT${NC}"
echo -e "  有效书签:     ${GREEN}$VALID_COUNT${NC}"
echo ""
echo -e "  输出文件:"
echo -e "    📁 整理后:    ${CYAN}$ORGANIZED_FILE${NC}"
echo -e "    ✅ 有效书签:  ${GREEN}$VALID_FILE${NC}"
echo -e "    📊 检查结果:  ${CYAN}$CHECK_OUTPUT_DIR/${NC}"
echo -e "    🎯 最终文件:  ${GREEN}$FINAL_FILE${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

if [ "$FULL_MODE" = false ]; then
    echo ""
    echo -e "${YELLOW}💡 提示: 这是小规模测试。要进行全量处理，请运行:${NC}"
    echo -e "${CYAN}   ./scripts/full_waterfox_process.sh --full${NC}"
fi
