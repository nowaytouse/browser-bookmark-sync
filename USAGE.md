# 使用指南

## 快速开始

### 1. 编译项目

```bash
cargo build --release
```

### 2. 检查浏览器

首先检查工具能检测到哪些浏览器：

```bash
./target/release/browser-bookmark-sync list
```

输出示例：
```
🌐 Detected Browsers:
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  ✅ Safari
     Path: "/Users/username/Library/Safari/Bookmarks.plist"
  ✅ Brave
     Path: "/Users/username/Library/Application Support/BraveSoftware/Brave-Browser/Default/Bookmarks"
  ❌ Waterfox (not detected)
  ❌ Firefox Nightly (not detected)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### 3. 验证书签完整性

在同步前，建议先验证所有浏览器的书签：

```bash
./target/release/browser-bookmark-sync validate
```

详细验证：
```bash
./target/release/browser-bookmark-sync validate --detailed
```

### 4. 预览同步（干运行）

在实际同步前，先预览会发生什么：

```bash
./target/release/browser-bookmark-sync sync --dry-run
```

输出示例：
```
🔍 Phase 1: Pre-sync validation
✅ Pre-sync validation passed: 2 browsers detected

📖 Phase 2: Reading bookmarks from all browsers
✅ Read 150 bookmarks from Safari
✅ Read 200 bookmarks from Brave

🔄 Phase 3: Merging bookmarks
📊 Merged result: 280 unique bookmarks

🏃 Dry run mode - no changes will be made

📊 Sync Preview:
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Safari: 150 bookmarks
  Brave: 200 bookmarks
  ─────────────────────────────────────────
  Merged: 280 unique bookmarks
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### 5. 执行同步

确认预览无误后，执行实际同步：

```bash
./target/release/browser-bookmark-sync sync
```

带详细输出：
```bash
./target/release/browser-bookmark-sync sync --verbose
```

## 定时同步

### 基础用法

每30分钟自动同步一次（默认）：

```bash
./target/release/browser-bookmark-sync schedule
```

### 自定义时间间隔

使用 cron 表达式自定义同步频率：

```bash
# 每小时同步
./target/release/browser-bookmark-sync schedule --cron "0 0 * * * *"

# 每天凌晨2点同步
./target/release/browser-bookmark-sync schedule --cron "0 0 2 * * *"

# 每周一早上9点同步
./target/release/browser-bookmark-sync schedule --cron "0 0 9 * * MON"
```

### 后台运行

```bash
./target/release/browser-bookmark-sync schedule --daemon
```

## Cron 表达式格式

格式：`秒 分 时 日 月 星期`

| 字段 | 允许值 | 特殊字符 |
|------|--------|----------|
| 秒 | 0-59 | * , - / |
| 分 | 0-59 | * , - / |
| 时 | 0-23 | * , - / |
| 日 | 1-31 | * , - / ? |
| 月 | 1-12 或 JAN-DEC | * , - / |
| 星期 | 0-6 或 SUN-SAT | * , - / ? |

### 常用示例

```bash
# 每分钟
"0 * * * * *"

# 每5分钟
"0 */5 * * * *"

# 每15分钟
"0 */15 * * * *"

# 每30分钟
"0 */30 * * * *"

# 每小时
"0 0 * * * *"

# 每天中午12点
"0 0 12 * * *"

# 每天凌晨3点
"0 0 3 * * *"

# 工作日早上9点
"0 0 9 * * MON-FRI"

# 周末早上10点
"0 0 10 * * SAT,SUN"
```

## 历史记录同步

### 基础用法

同步所有浏览器的历史记录：

```bash
./target/release/browser-bookmark-sync sync-history
```

### 仅同步最近N天

只同步最近7天的历史记录：

```bash
./target/release/browser-bookmark-sync sync-history --days 7
```

只同步最近30天：

```bash
./target/release/browser-bookmark-sync sync-history --days 30
```

### 预览模式

先预览会同步什么：

```bash
./target/release/browser-bookmark-sync sync-history --days 7 --dry-run --verbose
```

输出示例：
```
📜 Starting history synchronization
📅 Syncing history from last 7 days
📖 Phase 1: Reading history from all browsers
✅ Read 396 history items from Waterfox
⚠️  Failed to read history from Brave: Brave history file not found
🔄 Phase 2: Merging history
📊 Merged result: 396 unique history items
🏃 Dry run mode - no changes will be made
✅ History synchronization complete!
```

### 支持的浏览器

历史记录同步目前支持：
- ✅ Waterfox（所有配置文件）
- ✅ Firefox Nightly
- ✅ Brave
- ✅ Chrome
- ✅ Safari（6155条历史记录）

## 阅读列表同步

### 基础用法

同步所有浏览器的阅读列表：

```bash
./target/release/browser-bookmark-sync sync-reading-list
```

### 预览模式

先预览会同步什么：

```bash
./target/release/browser-bookmark-sync sync-reading-list --dry-run --verbose
```

输出示例：
```
📚 Starting reading list synchronization
📖 Phase 1: Reading lists from all browsers
✅ Read 15 reading list items from Safari
🔄 Phase 2: Merging reading lists
📊 Merged result: 15 unique reading list items
🏃 Dry run mode - no changes will be made
✅ Reading list synchronization complete!
```

### 支持的浏览器

阅读列表同步目前支持：
- ✅ Safari（原生Reading List）
- ❌ 其他浏览器（暂不支持）

### 注意事项

1. **历史记录可能很大**：如果不指定天数，可能会同步数万条记录
2. **性能考虑**：建议使用 `--days` 参数限制同步范围
3. **隐私保护**：历史记录包含敏感信息，请谨慎使用
4. **Safari限制**：Safari的历史记录数据库格式特殊，暂不支持

## 高级功能

### 环境变量

控制日志级别：

```bash
# 详细调试信息
RUST_LOG=debug ./target/release/browser-bookmark-sync sync

# 仅显示警告和错误
RUST_LOG=warn ./target/release/browser-bookmark-sync sync

# 追踪级别（最详细）
RUST_LOG=trace ./target/release/browser-bookmark-sync sync
```

### 备份恢复

每次同步前会自动创建备份文件：

```bash
# Safari 备份位置
~/Library/Safari/Bookmarks.plist.backup

# Brave 备份位置
~/Library/Application Support/BraveSoftware/Brave-Browser/Default/Bookmarks.backup
```

恢复备份：
```bash
# Safari
cp ~/Library/Safari/Bookmarks.plist.backup ~/Library/Safari/Bookmarks.plist

# Brave
cp ~/Library/Application\ Support/BraveSoftware/Brave-Browser/Default/Bookmarks.backup \
   ~/Library/Application\ Support/BraveSoftware/Brave-Browser/Default/Bookmarks
```

## 故障排查

### 问题：找不到浏览器

**症状**：`list` 命令显示浏览器未检测到

**解决方案**：
1. 确认浏览器已安装
2. 确认浏览器至少运行过一次（生成书签文件）
3. 检查书签文件路径是否正确

### 问题：同步失败

**症状**：同步过程中出现错误

**解决方案**：
1. 运行验证命令检查书签完整性
   ```bash
   ./target/release/browser-bookmark-sync validate --detailed
   ```

2. 查看详细日志
   ```bash
   RUST_LOG=debug ./target/release/browser-bookmark-sync sync --verbose
   ```

3. 检查备份文件是否存在，必要时恢复

### 问题：权限错误

**症状**：无法读取或写入书签文件

**解决方案**：
1. 确保有足够的文件系统权限
2. 在 macOS 上，可能需要授予终端完全磁盘访问权限
   - 系统偏好设置 → 安全性与隐私 → 隐私 → 完全磁盘访问权限

### 问题：定时任务不工作

**症状**：schedule 命令启动但不执行同步

**解决方案**：
1. 检查 cron 表达式是否正确
2. 查看日志输出
3. 确保进程保持运行（使用 `--daemon` 标志）

## 最佳实践

### 1. 首次使用

```bash
# 1. 检查浏览器
./target/release/browser-bookmark-sync list

# 2. 验证书签
./target/release/browser-bookmark-sync validate --detailed

# 3. 干运行预览
./target/release/browser-bookmark-sync sync --dry-run

# 4. 执行同步
./target/release/browser-bookmark-sync sync
```

### 2. 日常使用

设置定时任务，让工具自动同步：

```bash
# 每30分钟自动同步
./target/release/browser-bookmark-sync schedule --daemon &
```

### 3. 定期验证

建议每周运行一次验证：

```bash
./target/release/browser-bookmark-sync validate --detailed
```

### 4. 备份管理

定期检查备份文件，确保可以恢复：

```bash
# 列出所有备份文件
find ~/Library -name "*.backup" -type f
```

## 安全建议

1. ✅ **始终先运行干运行模式**
2. ✅ **定期验证书签完整性**
3. ✅ **保留备份文件**
4. ✅ **使用版本控制管理配置**
5. ✅ **监控同步日志**

## 性能优化

### 大量书签

如果有数千个书签，可以：

1. 增加同步间隔（减少频率）
2. 使用 `--verbose` 监控性能
3. 考虑分批同步（未来功能）

### 资源使用

工具设计为轻量级：
- 内存使用：< 50MB
- CPU 使用：同步时短暂峰值，其余时间接近0
- 磁盘 I/O：仅在同步时发生

## 获取帮助

```bash
# 查看帮助
./target/release/browser-bookmark-sync --help

# 查看子命令帮助
./target/release/browser-bookmark-sync sync --help
./target/release/browser-bookmark-sync schedule --help
```
