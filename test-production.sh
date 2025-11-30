#!/bin/bash
# 书签同步工具实战测试脚本
# 用途: 安全地测试新功能（场景同步和清理）

set -e  # 遇到错误立即退出

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
BACKUP_DIR="$HOME/Desktop/bookmark_deep_backup_$(date +%Y%m%d_%H%M%S)"
BINARY="$SCRIPT_DIR/target/release/browser-bookmark-sync"

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}   书签同步工具 - 实战测试脚本${NC}"
echo -e "${BLUE}════════════════════════════════════════════════════════${NC}"
echo ""

# 检查二进制文件
if [ ! -f "$BINARY" ]; then
    echo -e "${RED}错误: 未找到编译后的二进制文件${NC}"
    echo -e "${YELLOW}请先运行: cargo build --release${NC}"
    exit 1
fi

echo -e "${GREEN}✓${NC} 找到二进制文件: $BINARY"
echo ""

# 第一步: 创建深度备份
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}步骤 1/5: 创建深度备份${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

mkdir -p "$BACKUP_DIR"
echo "备份目录: $BACKUP_DIR"
echo ""

# 备份各浏览器配置
BROWSERS=(
    "BraveSoftware/Brave-Browser"
    "BraveSoftware/Brave-Browser-Nightly"
    "Google/Chrome"
    "Waterfox"
)

for browser in "${BROWSERS[@]}"; do
    SRC="$HOME/Library/Application Support/$browser"
    if [ -d "$SRC" ]; then
        echo -e "${GREEN}✓${NC} 备份: $browser"
        cp -R "$SRC" "$BACKUP_DIR/" 2>/dev/null || echo -e "${YELLOW}⚠${NC}  无法备份 $browser (可能需要关闭浏览器)"
    else
        echo -e "${YELLOW}⊗${NC} 跳过: $browser (未安装)"
    fi
done

# 备份 Safari
SAFARI_SRC="$HOME/Library/Safari"
if [ -d "$SAFARI_SRC" ]; then
    echo -e "${GREEN}✓${NC} 备份: Safari"
    cp -R "$SAFARI_SRC" "$BACKUP_DIR/" 2>/dev/null || echo -e "${YELLOW}⚠${NC}  无法备份 Safari"
fi

echo ""
echo -e "${GREEN}✓ 备份完成!${NC}"
echo -e "${YELLOW}备份位置: $BACKUP_DIR${NC}"
echo ""

# 第二步: 列出检测到的浏览器
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}步骤 2/5: 检测浏览器${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

"$BINARY" list
echo ""

# 第三步: Dry-run 清理测试
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}步骤 3/5: Dry-Run 清理预览${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

echo "预览所有浏览器的清理结果..."
echo ""

"$BINARY" cleanup \
    --remove-duplicates \
    --remove-empty-folders \
    --dry-run

echo ""

# 询问是否继续
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}是否继续执行实际清理? (这将修改书签数据)${NC}"
echo -e "${YELLOW}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo ""
echo -e "${GREEN}备份已创建在: $BACKUP_DIR${NC}"
echo ""
read -p "输入 YES 继续，其他任何键取消: " -r
echo ""

if [[ ! $REPLY =~ ^YES$ ]]; then
    echo -e "${YELLOW}已取消。备份已保留在: $BACKUP_DIR${NC}"
    exit 0
fi

# 第四步: 小范围测试 - Chrome
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}步骤 4/5: 小范围测试 (Chrome)${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

echo "仅清理 Chrome 浏览器..."
echo ""

"$BINARY" cleanup \
    --browsers "chrome" \
    --remove-duplicates \
    --remove-empty-folders

echo ""
echo -e "${GREEN}✓ Chrome 清理完成${NC}"
echo ""

# 第五步: 验证
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${YELLOW}步骤 5/5: 验证结果${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

"$BINARY" validate --detailed

echo ""
echo -e "${GREEN}════════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}   测试完成!${NC}"
echo -e "${GREEN}════════════════════════════════════════════════════════${NC}"
echo ""
echo -e "备份位置: ${YELLOW}$BACKUP_DIR${NC}"
echo ""
echo -e "${BLUE}下一步建议:${NC}"
echo ""
echo -e "1. 打开 Chrome 浏览器，检查书签是否正常"
echo -e "2. 如果一切正常，可以对其他浏览器执行清理:"
echo -e "   ${YELLOW}$BINARY cleanup --remove-duplicates --remove-empty-folders${NC}"
echo ""
echo -e "3. 如果需要恢复，可以从备份恢复:"
echo -e "   ${YELLOW}cp -R $BACKUP_DIR/* ~/Library/Application\\ Support/${NC}"
echo ""
