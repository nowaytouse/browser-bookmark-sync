#!/bin/bash
# ============================================================
# 🔄 Browser Sync - 一键同步脚本
# 双击即可运行，同步 Brave Nightly ↔ Waterfox
# ============================================================

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 获取脚本所在目录
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/browser-bookmark-sync"
BACKUP_DIR="$HOME/Desktop/browser_backup_$(date +%Y%m%d_%H%M%S)"

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║${NC}        🔄 ${GREEN}Browser Bookmark Sync${NC} - 一键同步工具           ${BLUE}║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# 检查二进制文件
if [ ! -f "$BINARY" ]; then
    echo -e "${YELLOW}⚠️  未找到编译好的程序，正在编译...${NC}"
    cd "$SCRIPT_DIR"
    cargo build --release
    echo -e "${GREEN}✅ 编译完成${NC}"
fi

# 检查浏览器是否运行
check_browser_running() {
    if pgrep -x "$1" > /dev/null 2>&1; then
        return 0
    fi
    return 1
}

echo -e "${YELLOW}🔍 检查浏览器状态...${NC}"

BROWSERS_RUNNING=""
if check_browser_running "Brave Browser"; then
    BROWSERS_RUNNING="$BROWSERS_RUNNING Brave"
fi
if check_browser_running "Google Chrome"; then
    BROWSERS_RUNNING="$BROWSERS_RUNNING Chrome"
fi
if check_browser_running "Waterfox"; then
    BROWSERS_RUNNING="$BROWSERS_RUNNING Waterfox"
fi
if check_browser_running "Safari"; then
    BROWSERS_RUNNING="$BROWSERS_RUNNING Safari"
fi

if [ -n "$BROWSERS_RUNNING" ]; then
    echo -e "${YELLOW}⚠️  以下浏览器正在运行:${BROWSERS_RUNNING}${NC}"
    echo -e "${YELLOW}   建议关闭浏览器后再同步，否则可能无法读取部分数据${NC}"
    echo ""
    read -p "是否继续? (y/N) " -n 1 -r
    echo ""
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo -e "${RED}❌ 已取消${NC}"
        exit 1
    fi
fi

# 创建备份目录
echo ""
echo -e "${BLUE}💾 创建备份...${NC}"
mkdir -p "$BACKUP_DIR"

# 备份中枢浏览器数据
if [ -f "$HOME/Library/Application Support/BraveSoftware/Brave-Browser-Nightly/Default/Bookmarks" ]; then
    cp "$HOME/Library/Application Support/BraveSoftware/Brave-Browser-Nightly/Default/Bookmarks" "$BACKUP_DIR/BraveNightly_Bookmarks.json"
    echo -e "  ${GREEN}✅${NC} Brave Nightly 书签已备份"
fi

if [ -f "$HOME/Library/Application Support/Waterfox/Profiles/"*".default-release/places.sqlite" ]; then
    cp "$HOME/Library/Application Support/Waterfox/Profiles/"*".default-release/places.sqlite" "$BACKUP_DIR/Waterfox_places.sqlite" 2>/dev/null || true
    echo -e "  ${GREEN}✅${NC} Waterfox 数据已备份"
fi

if [ -f "$HOME/Library/Safari/Bookmarks.plist" ]; then
    cp "$HOME/Library/Safari/Bookmarks.plist" "$BACKUP_DIR/Safari_Bookmarks.plist" 2>/dev/null || true
    echo -e "  ${GREEN}✅${NC} Safari 书签已备份"
fi

echo -e "  📁 备份位置: $BACKUP_DIR"

# 执行同步
echo ""
echo -e "${BLUE}🔄 开始同步...${NC}"
echo ""

# Step 1: 验证当前状态
echo -e "${YELLOW}📊 Step 1: 验证当前状态${NC}"
"$BINARY" validate 2>&1 | grep -E "(Bookmarks Read|URLs|folders|✅|❌)" | head -20

# Step 2: 设置中枢浏览器并同步
echo ""
echo -e "${YELLOW}🎯 Step 2: 同步到中枢浏览器 (Waterfox + Brave Nightly)${NC}"
"$BINARY" set-hubs \
    --browsers "waterfox,brave-nightly" \
    --sync-history \
    --sync-reading-list \
    --clear-others \
    2>&1 | grep -E "(Hub|Merged|URLs|folders|items|CLEARED|✅|❌|📊|📚|📜|🎯|🗑️)" | head -30

# Step 3: 最终验证
echo ""
echo -e "${YELLOW}🔍 Step 3: 验证同步结果${NC}"
"$BINARY" validate 2>&1 | grep -E "(Bookmarks Read|URLs|folders|✅|❌|Summary)" | head -15

# 完成
echo ""
echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║${NC}                    ✅ ${GREEN}同步完成!${NC}                           ${GREEN}║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  📁 备份位置: ${BLUE}$BACKUP_DIR${NC}"
echo -e "  🎯 中枢浏览器: ${GREEN}Waterfox${NC} + ${GREEN}Brave Nightly${NC}"
echo ""
echo -e "${YELLOW}提示: 重启浏览器以查看同步后的书签${NC}"
echo ""

# 保持窗口打开
read -p "按 Enter 键关闭..."
