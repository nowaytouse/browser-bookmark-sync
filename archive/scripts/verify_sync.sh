#!/bin/bash
echo "🔍 验证同步结果"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# 从日志中提取关键信息
echo "📊 同步统计："
grep "Bookmarks:" sync_output.log | tail -1
grep "History:" sync_output.log | tail -1
grep "Cookies:" sync_output.log | tail -1
echo ""

echo "📁 智能分类统计："
grep "📁" organize_output.log | head -18
echo ""

echo "❓ 未分类书签："
grep "❓ Unclassified" organize_output.log | head -1
echo ""

echo "✅ 处理的浏览器："
grep "✅ Organization complete" organize_output.log
echo ""

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
