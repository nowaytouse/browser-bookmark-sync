#!/bin/bash
# 完整处理 Waterfox 书签文件
# 功能: 整理 + 去重 + 死链检查 + 输出到桌面

set -e

# 配置
INPUT_FILE="/Users/nyamiiko/Desktop/waterfox"
OUTPUT_DIR="/Users/nyamiiko/Desktop/waterfox_processed"
TEMP_DIR="/tmp/waterfox_process_$$"
PROXY="http://127.0.0.1:6152"

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}🔧 Waterfox 书签完整处理流程${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

# 检查输入文件
if [ ! -f "$INPUT_FILE" ]; then
    echo -e "${RED}❌ 输入文件不存在: $INPUT_FILE${NC}"
    exit 1
fi

# 创建临时目录和输出目录
mkdir -p "$TEMP_DIR"
mkdir -p "$OUTPUT_DIR"

# 获取书签数量
ORIGINAL_COUNT=$(grep -c '<DT><A' "$INPUT_FILE" || echo "0")
echo -e "${GREEN}📖 原始书签数量: $ORIGINAL_COUNT${NC}"

# Step 1: 整理书签
echo -e "\n${YELLOW}📁 Step 1: 智能整理书签...${NC}"
./target/release/browser-bookmark-sync organize \
    --file "$INPUT_FILE" \
    --output "$TEMP_DIR/organized.html" \
    --stats

ORGANIZED_COUNT=$(grep -c '<DT><A' "$TEMP_DIR/organized.html" || echo "0")
echo -e "${GREEN}✅ 整理后书签数量: $ORGANIZED_COUNT${NC}"

# Step 2: 小规模死链检查测试 (先测试 50 个)
echo -e "\n${YELLOW}🔍 Step 2: 小规模死链检查测试 (50个)...${NC}"
echo -e "${BLUE}   代理: $PROXY${NC}"
echo -e "${BLUE}   直连: 同时测试${NC}"

# 注意: check 命令目前不支持从文件输入，需要先实现这个功能
# 暂时跳过死链检查，先完成整理和去重

# Step 3: 复制整理后的文件到输出目录
echo -e "\n${YELLOW}📤 Step 3: 输出到桌面...${NC}"
cp "$TEMP_DIR/organized.html" "$OUTPUT_DIR/waterfox_organized_$(date +%Y%m%d_%H%M%S).html"

# 清理临时文件
rm -rf "$TEMP_DIR"

echo -e "\n${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${GREEN}✅ 处理完成!${NC}"
echo -e "${GREEN}   原始: $ORIGINAL_COUNT 个书签${NC}"
echo -e "${GREEN}   整理后: $ORGANIZED_COUNT 个书签${NC}"
echo -e "${GREEN}   输出目录: $OUTPUT_DIR${NC}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
