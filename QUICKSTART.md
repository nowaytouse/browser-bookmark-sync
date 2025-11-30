# 快速开始

## 一键同步

```bash
# 编译（首次使用）
cargo build --release

# 执行同步
./target/release/browser-bookmark-sync sync
```

就这么简单！✅

## 自动定时同步

```bash
# 每30分钟自动同步
./target/release/browser-bookmark-sync schedule
```

## 注意事项

### macOS 权限
首次使用需要授予**完全磁盘访问权限**：
1. 系统偏好设置 → 安全性与隐私 → 隐私
2. 完全磁盘访问权限
3. 添加你的终端应用（Terminal 或 iTerm）

### 支持的浏览器
- ✅ Waterfox
- ✅ Safari  
- ✅ Brave
- ✅ Firefox Nightly（如果已安装）

## 工作原理

1. **读取** - 从所有浏览器读取书签
2. **合并** - 智能去重，保留唯一书签
3. **备份** - 自动创建 .backup 文件
4. **写入** - 同步到所有浏览器
5. **验证** - 确认同步成功

## 常用命令

```bash
# 查看检测到的浏览器
./target/release/browser-bookmark-sync list

# 验证书签完整性
./target/release/browser-bookmark-sync validate

# 预览同步（不实际修改）
./target/release/browser-bookmark-sync sync --dry-run

# 执行同步
./target/release/browser-bookmark-sync sync

# 定时同步（每小时）
./target/release/browser-bookmark-sync schedule --cron "0 0 * * * *"
```

## 恢复备份

如果需要恢复：

```bash
# Safari
cp ~/Library/Safari/Bookmarks.plist.backup ~/Library/Safari/Bookmarks.plist

# Brave
cp ~/Library/Application\ Support/BraveSoftware/Brave-Browser/Default/Bookmarks.json.backup \
   ~/Library/Application\ Support/BraveSoftware/Brave-Browser/Default/Bookmarks

# Waterfox
cp ~/Library/Application\ Support/Waterfox/Profiles/*/places.sqlite.backup \
   ~/Library/Application\ Support/Waterfox/Profiles/*/places.sqlite
```

## 完成！

现在你的所有浏览器书签将保持同步 🎉
