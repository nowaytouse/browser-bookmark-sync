# 🔖 跨浏览器书签同步工具 (bsync)

快速、跨浏览器的书签管理工具，支持 macOS。合并、去重、导出多个浏览器的书签。

## ✨ 特色功能

- **多浏览器支持**: Safari、Chrome、Brave、Brave Nightly、Waterfox、Firefox
- **HTML 导出**: 标准 Netscape 格式（所有浏览器可导入）
- **智能去重**: 跨所有来源移除重复书签
- **自动分类**: 48 条内置规则整理书签
- **Safari 阅读列表**: 将阅读列表导出为书签
- **历史记录导出**: 导出浏览历史（可设置天数限制）
- **Cookie 导出**: 导出 Cookie 数据（⚠️ 影响会话）
- **数据库安全**: 复制-验证-替换机制，防止数据损坏
- **默认安全**: 仅导出，不修改浏览器数据

## 🚀 快速开始

```bash
# 编译
cargo build --release
cp target/release/browser-bookmark-sync /usr/local/bin/bsync

# 基本用法
bsync list                              # 列出浏览器
bsync export -d --merge                 # 导出全部，去重合并
bsync export -b safari -r               # Safari + 阅读列表
bsync analyze                           # 检查问题
```

## 📖 命令列表

| 命令 | 别名 | 说明 |
|------|------|------|
| `list` | `l` | 列出检测到的浏览器 |
| `export` | `e` | 导出书签到 HTML |
| `analyze` | `a` | 分析书签 |
| `organize` | `o` | 智能整理 |
| `validate` | `v` | 验证完整性 |
| `history` | `hist` | 同步浏览历史 |
| `rules` | - | 显示分类规则 |
| `backup` | - | 创建完整备份 |

## 📤 导出命令

书签管理的主要命令：

```bash
bsync export [选项]
```

### 选项

| 参数 | 简写 | 说明 |
|------|------|------|
| `--output <文件>` | `-o` | 输出路径（默认: ~/Desktop/bookmarks.html）|
| `--browsers <列表>` | `-b` | 来源浏览器（默认: all）|
| `--deduplicate` | `-d` | 移除重复 |
| `--merge` | `-m` | 扁平结构（不按浏览器分文件夹）|
| `--clean` | - | 移除空文件夹 |
| `--reading-list` | `-r` | 包含 Safari 阅读列表 |
| `--history` | - | 包含浏览历史 |
| `--history-days <天数>` | - | 历史记录天数（默认: 30，0=全部）|
| `--cookies` | - | 包含 Cookie (⚠️  影响会话) |
| `--passwords` | - | 🔴 导出加密密码元数据（⚠️ 无法解密）|
| `--extensions` | - | ⚠️ 导出扩展程序元数据（仅列表，无法迁移）|
| `--include <文件>` | - | 导入已有 HTML |
| `--folder <名称>` | `-f` | 仅导出指定名称的文件夹内容（如 "👀临时"）|
| `--clear-after` | - | 导出后清空来源（⚠️ 危险，需要 --unsafe-write）|
| `--unsafe-write` | - | 启用不安全数据库写入（需要确认）|
| `--verbose` | `-v` | 详细输出 |

### 示例

```bash
# 导出全部浏览器，去重合并
bsync export -d -m -o ~/bookmarks.html

# 仅 Safari，包含阅读列表
bsync export -b safari -r -d

# 合并多个来源
bsync export -b "safari,brave" -d -m --include old_backup.html

# 完整清理导出
bsync export -d -m --clean

# 仅导出指定文件夹（如 "👀临时" 或 "Temp"）
bsync export -f "👀临时" -d -o ~/Desktop/temp_bookmarks.html

# 从所有浏览器导出 emoji 文件夹
bsync export -f "👀" -d --merge
```

## 🧠 智能整理

自动将书签分类到 48 个类别：

```bash
# 预览（安全）
bsync organize --dry-run --stats

# 应用到指定浏览器
bsync organize -b safari

# 自定义规则
bsync organize -r my-rules.json
```

### 分类

- 🎬 流媒体、视频
- 🎮 游戏
- 💻 开发、GitHub
- 📱 社交媒体
- 🛒 购物
- 📰 新闻、博客
- 还有 40+ 更多...

## 🔍 分析

检查书签问题：

```bash
bsync analyze
bsync analyze -b safari
```

检测内容：
- 重复 URL
- 空文件夹
- NSFW 内容（仅统计）

## 🌐 支持的浏览器

| 浏览器 | 书签 | 历史 | 阅读列表 | Cookie | 密码 | 扩展 |
|--------|------|------|----------|--------|------|------|
| Safari | ✅ | ✅ | ✅ | ✅ | - | - |
| Chrome | ✅ | ✅ | - | ✅ | 🔒* | ✅ |
| Brave | ✅ | ✅ | - | ✅ | 🔒* | ✅ |
| Brave Nightly | ✅ | ✅ | - | ✅ | 🔒* | ✅ |
| Waterfox | ✅ | ✅ | - | ✅ | - | - |
| Firefox | ✅ | ✅ | - | ✅ | - | - |

*🔒 = 仅加密元数据，无法解密实际密码

## ⚠️ 重要提示

1. **操作前关闭浏览器** - 避免数据库锁定
2. **导出是安全的** - 不修改浏览器数据，内置数据库安全机制
3. **--clear-after 是破坏性的** - 谨慎使用，必须配合 --unsafe-write 标志
4. **浏览器同步冲突** - 如果启用了同步，手动导入更安全
5. **Cookie 导出警告** - 导出 Cookie 会影响活动会话，谨慎处理
6. **密码导出** - 🔴 仅导出加密的密码元数据（URL、用户名、时间戳），实际密码由操作系统加密保护，无法解密。如需迁移密码，请使用浏览器内置导出功能或密码管理器
7. **扩展程序导出** - ⚠️ 仅导出扩展程序列表（名称、版本、权限），无法自动安装或迁移设置。用于手动重新安装参考
8. **数据库安全** - 所有写操作使用"复制-验证-替换"机制，确保原始数据库完整性

## 📊 输出示例

```
📤 Exporting bookmarks to HTML
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Output: ~/Desktop/bookmarks.html
Source: all
  ✓ Deduplicate
  ✓ Merge (flat)
  ✓ Include Safari reading list
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
🎯 Target browsers:
  - Safari
  - Brave Nightly
  - Waterfox
📖 Reading Safari reading list...
   42 items found
📊 Collection complete: 178326 bookmarks
🧹 Deduplicating...
  ✅ Removed 154805 duplicate bookmarks

✅ Exported 23521 bookmarks to ~/Desktop/bookmarks.html
```

## 📄 许可证

MIT License
