#!/bin/bash

# 🚨 Firefox Sync冲突修复脚本
# 用途：恢复被Firefox Sync覆盖的书签数据

set -e

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🚨 Firefox Sync冲突修复工具"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# 检测Waterfox配置文件路径
WATERFOX_PROFILE="$HOME/Library/Application Support/Waterfox/Profiles/ll4fbmm0.default-release"

if [ ! -d "$WATERFOX_PROFILE" ]; then
    echo "❌ 错误：找不到Waterfox配置文件"
    echo "   路径：$WATERFOX_PROFILE"
    exit 1
fi

echo "✅ 找到Waterfox配置文件"
echo ""

# 检查Firefox Sync状态
echo "🔍 检查Firefox Sync状态..."
if grep -q "services.sync.username" "$WATERFOX_PROFILE/prefs.js"; then
    SYNC_USER=$(grep "services.sync.username" "$WATERFOX_PROFILE/prefs.js" | sed 's/.*"\(.*\)".*/\1/')
    echo "⚠️  Firefox Sync已启用"
    echo "   账号：$SYNC_USER"
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "⚠️  重要警告"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "Firefox Sync会从云端恢复旧数据，覆盖我们的修改。"
    echo ""
    echo "请在继续之前，手动禁用Firefox Sync："
    echo "  1. 打开Waterfox"
    echo "  2. 点击右上角账户图标"
    echo "  3. 选择'管理账户'"
    echo "  4. 取消勾选'书签'同步"
    echo "  5. 或者完全断开Firefox账户"
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    read -p "已禁用Firefox Sync？(y/N): " -n 1 -r
    echo ""
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "❌ 取消操作"
        exit 1
    fi
else
    echo "✅ Firefox Sync未启用"
fi

echo ""

# 检查Waterfox是否正在运行
echo "🔍 检查Waterfox运行状态..."
if pgrep -x "waterfox-bin" > /dev/null; then
    echo "⚠️  Waterfox正在运行"
    echo ""
    read -p "需要关闭Waterfox，继续？(y/N): " -n 1 -r
    echo ""
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "❌ 取消操作"
        exit 1
    fi
    
    echo "🛑 关闭Waterfox..."
    killall waterfox-bin 2>/dev/null || true
    sleep 2
    echo "✅ Waterfox已关闭"
else
    echo "✅ Waterfox未运行"
fi

echo ""

# 检查备份文件
BACKUP_FILE="$WATERFOX_PROFILE/places.sqlite.backup"
if [ ! -f "$BACKUP_FILE" ]; then
    echo "❌ 错误：找不到备份文件"
    echo "   路径：$BACKUP_FILE"
    exit 1
fi

echo "✅ 找到备份文件"
echo ""

# 显示当前状态
echo "📊 当前书签数量："
CURRENT_COUNT=$(sqlite3 "$WATERFOX_PROFILE/places.sqlite" "SELECT COUNT(*) FROM moz_bookmarks WHERE type = 1;" 2>/dev/null || echo "无法读取")
echo "   当前：$CURRENT_COUNT"

BACKUP_COUNT=$(sqlite3 "$BACKUP_FILE" "SELECT COUNT(*) FROM moz_bookmarks WHERE type = 1;" 2>/dev/null || echo "无法读取")
echo "   备份：$BACKUP_COUNT"
echo ""

# 确认恢复
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "⚠️  准备恢复数据"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "将从备份恢复书签数据："
echo "  从：$CURRENT_COUNT 个书签"
echo "  到：$BACKUP_COUNT 个书签"
echo ""
read -p "确认恢复？(y/N): " -n 1 -r
echo ""
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "❌ 取消操作"
    exit 1
fi

# 备份当前数据（以防万一）
echo ""
echo "💾 备份当前数据..."
cp "$WATERFOX_PROFILE/places.sqlite" "$WATERFOX_PROFILE/places.sqlite.firefox_sync_backup"
echo "✅ 当前数据已备份到：places.sqlite.firefox_sync_backup"
echo ""

# 恢复备份
echo "🔄 恢复备份数据..."
cp "$BACKUP_FILE" "$WATERFOX_PROFILE/places.sqlite"
echo "✅ 备份已恢复"
echo ""

# 验证
echo "🔍 验证恢复结果..."
NEW_COUNT=$(sqlite3 "$WATERFOX_PROFILE/places.sqlite" "SELECT COUNT(*) FROM moz_bookmarks WHERE type = 1;" 2>/dev/null || echo "无法读取")
echo "   恢复后：$NEW_COUNT 个书签"

if [ "$NEW_COUNT" = "$BACKUP_COUNT" ]; then
    echo "✅ 验证成功"
else
    echo "⚠️  警告：数量不匹配"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 恢复完成"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "📝 后续步骤："
echo "  1. 确认已禁用Firefox Sync的书签同步"
echo "  2. 打开Waterfox验证书签数据"
echo "  3. 如果需要，重新运行同步工具："
echo "     ./target/release/browser-bookmark-sync sync"
echo "     ./target/release/browser-bookmark-sync smart-organize"
echo ""
echo "⚠️  重要提醒："
echo "  如果不禁用Firefox Sync，数据会再次被覆盖！"
echo ""
