#!/bin/bash
# ============================================================
# 🔄 Browser Sync - One-Click Full Sync
# Double-click to run: syncs Brave Nightly ↔ Waterfox
# Includes: Bookmarks + History + Cookies
# ============================================================

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/browser-bookmark-sync"
BACKUP_DIR="$HOME/Desktop/browser_backup_$(date +%Y%m%d_%H%M%S)"

echo ""
echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║${NC}     🔄 ${GREEN}Browser Full Sync${NC} - Bookmarks + History + Cookies   ${BLUE}║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Check binary
if [ ! -f "$BINARY" ]; then
    echo -e "${YELLOW}⚠️  Binary not found, compiling...${NC}"
    cd "$SCRIPT_DIR"
    cargo build --release
    echo -e "${GREEN}✅ Compilation complete${NC}"
fi

# Check running browsers
echo -e "${YELLOW}🔍 Checking browser status...${NC}"
RUNNING=""
pgrep -x "Brave Browser Nightly" > /dev/null 2>&1 && RUNNING="$RUNNING Brave-Nightly"
pgrep -x "Brave Browser" > /dev/null 2>&1 && RUNNING="$RUNNING Brave"
pgrep -x "Google Chrome" > /dev/null 2>&1 && RUNNING="$RUNNING Chrome"
pgrep -x "Waterfox" > /dev/null 2>&1 && RUNNING="$RUNNING Waterfox"
pgrep -x "Safari" > /dev/null 2>&1 && RUNNING="$RUNNING Safari"

if [ -n "$RUNNING" ]; then
    echo -e "${YELLOW}⚠️  Running browsers:${RUNNING}${NC}"
    echo -e "${YELLOW}   Close them for best results, or continue anyway${NC}"
    echo ""
    read -p "Continue? (y/N) " -n 1 -r
    echo ""
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo -e "${RED}❌ Cancelled${NC}"
        exit 1
    fi
else
    echo -e "${GREEN}✅ No browsers running${NC}"
fi

# Create backup
echo ""
echo -e "${BLUE}💾 Creating backup...${NC}"
mkdir -p "$BACKUP_DIR"

cp "$HOME/Library/Application Support/BraveSoftware/Brave-Browser-Nightly/Default/Bookmarks" "$BACKUP_DIR/BraveNightly_Bookmarks.json" 2>/dev/null && echo -e "  ${GREEN}✅${NC} Brave Nightly bookmarks" || true
cp "$HOME/Library/Application Support/Waterfox/Profiles/"*".default-release/places.sqlite" "$BACKUP_DIR/Waterfox_places.sqlite" 2>/dev/null && echo -e "  ${GREEN}✅${NC} Waterfox data" || true

echo -e "  📁 Backup: $BACKUP_DIR"

# Run full sync
echo ""
echo -e "${BLUE}🔄 Starting full sync...${NC}"
echo -e "   ${YELLOW}Syncing: Bookmarks + History + Cookies${NC}"
echo -e "   ${YELLOW}Hub browsers: Waterfox ↔ Brave Nightly${NC}"
echo ""

"$BINARY" sync --verbose 2>&1 | grep -E "(Hub|Merged|URLs|folders|items|cookies|✅|❌|📊|📚|📜|🍪|🎯)" | head -30

# Verify
echo ""
echo -e "${BLUE}🔍 Verifying...${NC}"
"$BINARY" validate 2>&1 | grep -E "(Bookmarks Read|✅|Summary)" | head -10

# Done
echo ""
echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║${NC}                    ✅ ${GREEN}Sync Complete!${NC}                        ${GREEN}║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  📁 Backup: ${BLUE}$BACKUP_DIR${NC}"
echo -e "  🎯 Hub browsers: ${GREEN}Waterfox${NC} ↔ ${GREEN}Brave Nightly${NC}"
echo -e "  📚 Synced: Bookmarks + History + Cookies"
echo ""
echo -e "${YELLOW}Tip: Restart browsers to see synced data${NC}"
echo ""

read -p "Press Enter to close..."
