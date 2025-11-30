#!/bin/bash
# ============================================================
# 🔄 Browser Sync - One-Click Migration Script
# Double-click to run: Migrate all data to Waterfox + Brave Nightly
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
echo -e "${BLUE}║${NC}        🔄 ${GREEN}Browser Data Migration${NC} - One-Click Tool          ${BLUE}║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Build if needed
if [ ! -f "$BINARY" ]; then
    echo -e "${YELLOW}⚠️  Binary not found, building...${NC}"
    cd "$SCRIPT_DIR"
    cargo build --release
    echo -e "${GREEN}✅ Build complete${NC}"
fi

# Check running browsers
echo -e "${YELLOW}🔍 Checking browser status...${NC}"
BROWSERS_RUNNING=""

check_browser() {
    if pgrep -x "$1" > /dev/null 2>&1; then
        BROWSERS_RUNNING="$BROWSERS_RUNNING $2"
    fi
}

check_browser "Brave Browser Nightly" "Brave-Nightly"
check_browser "Brave Browser" "Brave"
check_browser "Google Chrome" "Chrome"
check_browser "Waterfox" "Waterfox"
check_browser "Safari" "Safari"
check_browser "firefox" "Firefox"

if [ -n "$BROWSERS_RUNNING" ]; then
    echo -e "${YELLOW}⚠️  Running browsers:${BROWSERS_RUNNING}${NC}"
    echo -e "${YELLOW}   Close them for best results, or continue anyway.${NC}"
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

# Backup hub browsers
if [ -f "$HOME/Library/Application Support/BraveSoftware/Brave-Browser-Nightly/Default/Bookmarks" ]; then
    cp "$HOME/Library/Application Support/BraveSoftware/Brave-Browser-Nightly/Default/Bookmarks" "$BACKUP_DIR/BraveNightly_Bookmarks.json" 2>/dev/null
    echo -e "  ${GREEN}✅${NC} Brave Nightly bookmarks backed up"
fi

for profile in "$HOME/Library/Application Support/Waterfox/Profiles/"*".default-release"; do
    if [ -f "$profile/places.sqlite" ]; then
        cp "$profile/places.sqlite" "$BACKUP_DIR/Waterfox_places.sqlite" 2>/dev/null
        echo -e "  ${GREEN}✅${NC} Waterfox data backed up"
        break
    fi
done

echo -e "  📁 Backup: $BACKUP_DIR"

# Run migration
echo ""
echo -e "${BLUE}🔄 Starting migration...${NC}"
echo ""

# Step 1: Validate
echo -e "${YELLOW}📊 Step 1: Validating current state${NC}"
"$BINARY" validate 2>&1 | grep -E "(✅|❌|URLs|folders|bookmarks)" | head -10

# Step 2: Migrate
echo ""
echo -e "${YELLOW}🎯 Step 2: Migrating to hub browsers${NC}"
"$BINARY" migrate \
    --browsers "waterfox,brave-nightly" \
    --history \
    --clear-others \
    2>&1 | grep -E "(Hub|Non-hub|Merged|URLs|folders|items|cleared|✅|❌|📊|📚|📜|🎯|🗑️)" | head -25

# Step 3: Verify
echo ""
echo -e "${YELLOW}🔍 Step 3: Verifying results${NC}"
"$BINARY" validate 2>&1 | grep -E "(✅|❌|URLs|folders|Summary)" | head -10

# Done
echo ""
echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║${NC}                    ✅ ${GREEN}Migration Complete!${NC}                    ${GREEN}║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  📁 Backup: ${BLUE}$BACKUP_DIR${NC}"
echo -e "  🎯 Hub browsers: ${GREEN}Waterfox${NC} + ${GREEN}Brave Nightly${NC}"
echo ""
echo -e "${YELLOW}Tip: Restart browsers to see migrated data${NC}"
echo ""

read -p "Press Enter to close..."
