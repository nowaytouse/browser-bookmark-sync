# 🔄 浏览器书签同步工具

可靠的跨浏览器同步工具，支持书签、历史记录和 Cookies。采用**中枢浏览器架构**，避免数据重复和混乱。

[English](./README.md)

## ✨ 特性

- 🎯 **中枢浏览器架构** - 指定主力浏览器，在它们之间同步，可选清理其他浏览器
- 📚 **完整数据同步** - 书签、历史记录、阅读列表、Cookies 一条命令搞定
- 🌳 **保留结构** - 完整保留文件夹层级，无扁平化
- 🔄 **自动去重** - 自动删除重复的 URL 和条目
- 🔒 **安全备份** - 每次操作前自动备份
- 🧪 **测试验证** - 包含完整集成测试套件

## 🖥️ 支持的浏览器

| 浏览器 | 书签 | 历史 | Cookies |
|--------|------|------|---------|
| **Brave Nightly** | ✅ | ✅ | ✅ |
| **Waterfox** | ✅ | ✅ | ✅ |
| **Brave** | ✅ | ✅ | ✅ |
| **Chrome** | ✅ | ✅ | ✅ |
| **Safari** | ✅ | ✅ | - |
| **Firefox** | ✅ | ✅ | ✅ |
| **LibreWolf** | ✅ | ✅ | ✅ |

## 🚀 快速开始

### 一键同步 (macOS)

双击 `sync-now.command`：

```bash
./sync-now.command
```

### 命令行

```bash
# 完整同步中枢浏览器（书签 + 历史 + Cookies）
browser-bookmark-sync sync

# 预览更改（不实际执行）
browser-bookmark-sync sync --dry-run

# 同步并清理非中枢浏览器
browser-bookmark-sync sync --clear-others

# 自定义中枢浏览器
browser-bookmark-sync sync --browsers "chrome,firefox"

# 列出检测到的浏览器
browser-bookmark-sync list

# 验证数据完整性
browser-bookmark-sync validate
```

## 📐 架构

### 中枢浏览器模型

```
┌─────────────────────────────────────────────────────┐
│                    中枢浏览器                        │
│         Waterfox  ←──────→  Brave Nightly           │
│                                                      │
│   📚 书签         📜 历史记录    🍪 Cookies         │
│   (完整同步)      (完整同步)     (完整同步)          │
└─────────────────────────────────────────────────────┘
                         ↑
              可选: --clear-others
                         ↑
┌─────────────────────────────────────────────────────┐
│                   非中枢浏览器                       │
│        Chrome | Brave | Safari | LibreWolf          │
│              (数据迁移后清空)                        │
└─────────────────────────────────────────────────────┘
```

### 同步内容

| 数据类型 | 同步方式 |
|----------|----------|
| **书签** | 选择文件夹结构最完整的浏览器作为基准，保留层级 |
| **历史记录** | 合并所有浏览器的全部历史，按 URL 去重 |
| **Cookies** | 合并 Cookies，按 host+name+path 去重 |
| **阅读列表** | Safari 阅读列表 → 中枢浏览器书签文件夹 |

## 📊 命令参考

| 命令 | 说明 |
|------|------|
| `sync` | **完整同步** - 中枢浏览器之间同步书签 + 历史 + Cookies |
| `sync --clear-others` | 完整同步 + 清空非中枢浏览器数据 |
| `sync-history` | 仅同步全部历史记录 |
| `sync-cookies` | 仅同步 Cookies |
| `validate` | 检查所有浏览器数据完整性 |
| `list` | 显示检测到的浏览器和路径 |
| `schedule` | 启动自动定时同步 |

### 同步选项

```bash
browser-bookmark-sync sync [选项]

选项:
  -b, --browsers <浏览器>    中枢浏览器 [默认: waterfox,brave-nightly]
      --clear-others         清空非中枢浏览器数据
  -d, --dry-run              预览模式，不实际执行
  -v, --verbose              详细输出
```

### 性能优化

仅读取每个浏览器的 **Default 配置文件**，确保最佳性能：

```bash
# 完整同步（含全局去重）
browser-bookmark-sync sync
# 23,513 书签（去重后）约 1.7 秒
```

## 📊 验证结果

```
测试套件: 8/8 通过 ✅

同步统计:
├── 书签: 41,661 URLs, 1,936 文件夹
├── 历史: 30,301 条
├── Cookies: 925 条
└── 性能: ~1.1 秒 (release 构建)
```

## 🔧 安装

```bash
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync
cargo build --release

# 运行测试
cargo test --test integration_test

# 安装（可选）
cp target/release/browser-bookmark-sync /usr/local/bin/
```

## ⚠️ 注意事项

1. **同步前关闭浏览器** - 避免数据库锁定错误
2. **自动备份** - 保存到 `~/Desktop/browser_backup_*`
3. **默认中枢** - Waterfox + Brave Nightly（可通过 `--browsers` 自定义）

## 📜 许可证

MIT License
