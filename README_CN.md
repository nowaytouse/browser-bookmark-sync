# 🔖 跨浏览器书签同步工具

一款强大的 macOS 跨浏览器书签管理工具。合并、去重、导出多个浏览器的书签到单一 HTML 文件。

## ✨ 特色功能

- **🌐 多浏览器支持**: Safari、Chrome、Brave、Brave Nightly、Waterfox、Firefox
- **📤 HTML 导出**: 导出为标准 Netscape HTML 格式（所有浏览器都可导入）
- **🧹 智能去重**: 跨所有来源移除重复书签
- **🧠 自动分类**: 48 条内置规则，按类别整理书签
- **🔍 异常检测**: 检测批量导入、历史污染、NSFW 内容
- **💾 备份恢复**: 完整的备份和恢复功能

## 🚀 快速开始

### 安装

```bash
# 克隆并编译
git clone https://github.com/user/browser-sync.git
cd browser-sync
cargo build --release

# 添加到 PATH（可选）
cp target/release/browser-bookmark-sync /usr/local/bin/
```

### 基本用法

```bash
# 列出检测到的浏览器
browser-bookmark-sync list

# 导出所有书签到 HTML（推荐）
browser-bookmark-sync export-html -o ~/Desktop/my_bookmarks.html -d

# 导出指定浏览器并去重
browser-bookmark-sync export-html -b "safari,brave-nightly" -d --merge

# 智能分类书签
browser-bookmark-sync smart-organize -b safari --dry-run --show-stats
```

## 📖 命令列表

| 命令 | 说明 |
|------|------|
| `list` | 列出所有检测到的浏览器及书签位置 |
| `export-html` | 导出书签到 HTML 文件（推荐） |
| `validate` | 验证书签完整性 |
| `cleanup` | 删除重复书签和空文件夹 |
| `smart-organize` | 按 URL 模式自动分类书签 |
| `list-rules` | 显示可用的分类规则 |
| `sync-history` | 同步浏览器历史记录 |
| `analyze` | 分析书签（NSFW检测） |
| `master-backup` | 创建综合备份 |
| `restore-backup` | 从备份恢复 |
| `clear-bookmarks` | 清空浏览器书签（仅调试用） |

## 📤 导出到 HTML（推荐工作流）

推荐的书签管理方式是导出到 HTML，然后手动导入到目标浏览器。这样可以避免同步冲突。

```bash
# 第一步：导出所有书签并去重
browser-bookmark-sync export-html \
  -b "safari,brave-nightly,waterfox" \
  -d --merge \
  -o ~/Desktop/all_bookmarks.html

# 第二步：手动将 HTML 文件导入到浏览器
# - Safari: 文件 → 导入自 → 书签 HTML 文件
# - Chrome/Brave: 书签 → 导入书签和设置
# - Firefox: 书签 → 管理书签 → 导入和备份
```

### 导出选项

```bash
-o, --output <文件>      输出 HTML 文件路径
-b, --browsers <列表>    来源浏览器（逗号分隔，默认: all）
-d, --deduplicate        移除重复书签
    --merge              合并为扁平结构（不按浏览器分文件夹）
    --clean-empty        导出前移除空文件夹
    --include-html <文件> 同时导入已有 HTML 备份
    --clear-after        导出后清空来源浏览器的书签
-v, --verbose            显示详细输出
```

### 导出后清空

`--clear-after` 选项会在成功导出后删除来源浏览器的所有书签：

```bash
# 导出并清空来源书签
browser-bookmark-sync export-html -d --merge --clear-after
```

⚠️ **警告**: 如果浏览器启用了同步功能（Firefox Sync、Chrome Sync、iCloud 等），删除可能无效或导致书签版本不可预测。建议在使用此选项前禁用同步功能。

## 🧠 智能分类

自动将书签分类到 48 个类别：

```bash
# 预览分类（dry-run）
browser-bookmark-sync smart-organize -b safari --dry-run --show-stats

# 应用分类
browser-bookmark-sync smart-organize -b safari

# 使用自定义规则
browser-bookmark-sync smart-organize -r custom-rules.json
```

### 内置分类

- 🎬 流媒体、视频平台
- 🎮 游戏、游戏商店
- 💻 开发、GitHub、Stack Overflow
- 📱 社交媒体、论坛
- 🛒 购物、电商
- 📰 新闻、博客
- 🎨 设计、创意工具
- 还有 40+ 更多分类...

## 🔄 历史记录同步

在Hub浏览器之间同步浏览历史：

```bash
# 同步最近30天的历史
browser-bookmark-sync sync-history -b "waterfox,brave-nightly"

# 同步最近7天
browser-bookmark-sync sync-history -b "waterfox,brave-nightly" -d 7

# 预览模式
browser-bookmark-sync sync-history --dry-run
```

## 🔍 书签分析

分析书签中的重复和NSFW内容：

```bash
browser-bookmark-sync analyze -b safari
```

检测内容：
- **重复URL**: 同一URL被多次收藏
- **空文件夹**: 没有书签的文件夹
- **NSFW内容**: 成人内容统计（仅信息）

## 💾 备份与恢复

```bash
# 创建主备份
browser-bookmark-sync master-backup -o ~/Desktop/BookmarkBackup

# 从备份恢复
browser-bookmark-sync restore-backup -b waterfox -f backup.sqlite
```

## 🌐 支持的浏览器

| 浏览器 | 书签 | 历史 | Cookies |
|--------|------|------|---------|
| Safari | ✅ | ✅ | ❌ |
| Chrome | ✅ | ✅ | ✅ |
| Brave | ✅ | ✅ | ✅ |
| Brave Nightly | ✅ | ✅ | ✅ |
| Waterfox | ✅ | ✅ | ✅ |
| Firefox | ✅ | ✅ | ✅ |

## ⚠️ 重要提示

1. **操作前关闭浏览器**: 某些浏览器会锁定数据库文件
2. **使用 HTML 导出**: 避免直接写入浏览器以防止同步冲突
3. **先备份**: 重大操作前务必创建备份
4. **手动导入**: 手动导入 HTML 文件效果最佳

## 📊 输出示例

```
📤 导出书签到HTML文件
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
📄 输出: ~/Desktop/bookmarks.html
🌐 来源: safari,brave-nightly
🔀 合并模式
🧹 去重复
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  ✅ Safari : 136054 书签
  ✅ Brave Nightly : 42272 书签
📊 收集完成: 178326 书签
  ✅ 移除 154805 重复书签
✅ 导出完成!
   📄 文件: ~/Desktop/bookmarks.html
   📊 书签数: 23521

🎉 导出完成! 23521 书签
💡 请手动导入到目标浏览器，避免被同步覆盖
```

## 🛠️ 开发

```bash
# 运行测试
cargo test

# 编译发布版
cargo build --release

# 带调试日志运行
RUST_LOG=debug browser-bookmark-sync list
```

## 📄 许可证

MIT License

## 🤝 贡献

欢迎贡献！请先阅读贡献指南。
