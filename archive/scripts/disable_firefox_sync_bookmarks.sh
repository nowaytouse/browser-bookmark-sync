#!/bin/bash

# 🔧 自动禁用Firefox Sync书签同步
# 这是唯一真实有效的解决方案

set -e

WATERFOX_PROFILE="$HOME/Library/Application Support/Waterfox/Profiles/ll4fbmm0.default-release"
PREFS_FILE="$WATERFOX_PROFILE/prefs.js"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🔧 禁用Firefox Sync书签同步"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# 检查Waterfox是否运行
if pgrep -x "waterfox-bin" > /dev/null; then
    echo "⚠️  Waterfox正在运行，必须关闭"
    read -p "关闭Waterfox？(y/N): " -n 1 -r
    echo ""
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        killall waterfox-bin 2>/dev/null || true
        sleep 2
        echo "✅ Waterfox已关闭"
    else
        echo "❌ 取消操作"
        exit 1
    fi
fi

echo ""

# 备份prefs.js
echo "💾 备份prefs.js..."
cp "$PREFS_FILE" "$PREFS_FILE.backup_$(date +%Y%m%d_%H%M%S)"
echo "✅ 备份完成"
echo ""

# 禁用书签同步
echo "🔧 修改Firefox Sync配置..."

# 方法：添加或修改 services.sync.engine.bookmarks 为 false
if grep -q "services.sync.engine.bookmarks" "$PREFS_FILE"; then
    # 已存在，修改为false
    sed -i '' 's/user_pref("services.sync.engine.bookmarks", true);/user_pref("services.sync.engine.bookmarks", false);/' "$PREFS_FILE"
    echo "✅ 已禁用书签同步"
else
    # 不存在，添加
    echo 'user_pref("services.sync.engine.bookmarks", false);' >> "$PREFS_FILE"
    echo "✅ 已添加书签同步禁用配置"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 完成"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "📝 下一步："
echo "  1. 启动Waterfox"
echo "  2. 进入 设置 → Firefox账户 → 同步"
echo "  3. 确认'书签'已取消勾选"
echo "  4. 运行同步工具："
echo "     ./target/release/browser-bookmark-sync sync"
echo ""
echo "⚠️  重要："
echo "  - 其他数据（历史、密码等）仍会同步"
echo "  - 只有书签由我们的工具管理"
echo "  - 这是唯一有效的解决方案"
echo ""
