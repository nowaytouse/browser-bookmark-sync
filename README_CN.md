# 🔄 浏览器书签同步工具

可靠的跨浏览器书签、历史记录、阅读列表同步工具。采用**中枢浏览器架构**，避免数据重复和混乱。

[English](./README.md)

## ✨ 特性

- 🎯 **中枢浏览器模式** - 指定主力浏览器，其他浏览器自动清理
- 📚 **书签同步** - 保留完整文件夹结构，无扁平化
- 📜 **历史记录同步** - 跨浏览器合并浏览历史，自动去重
- 📖 **阅读列表迁移** - Safari 阅读列表 → 中枢浏览器书签
- 🍪 **Cookies 同步** - 跨浏览器 Cookie 迁移
- ⏰ **定时同步** - 支持 Cron 表达式自动同步
- 🔒 **安全备份** - 每次操作前自动备份
- 🧪 **测试验证** - 包含完整集成测试套件

## 🖥️ 支持的浏览器

| 浏览器 | 书签 | 历史 | 阅读列表 | Cookies |
|--------|------|------|----------|---------|
| **Brave Nightly** | ✅ | ✅ | - | ✅ |
| **Waterfox** | ✅ | ✅ | - | ✅ |
| **Brave** | ✅ | ✅ | - | ✅ |
| **Chrome** | ✅ | ✅ | - | ✅ |
| **Safari** | ✅ | ✅ | ✅ | - |
| **Firefox** | ✅ | ✅ | - | ✅ |
| **LibreWolf** | ✅ | ✅ | - | ✅ |

## 🚀 快速开始

### 一键同步（推荐）

在 macOS 上双击运行 `sync-now.command`：

```bash
# 或在终端运行
./sync-now.command
```

这将自动：
1. 备份当前数据到桌面
2. 同步 Brave Nightly ↔ Waterfox 书签和历史
3. 迁移 Safari 阅读列表到中枢浏览器
4. 清理非中枢浏览器的重复数据

### 命令行使用

```bash
# 查看所有检测到的浏览器
browser-bookmark-sync list

# 验证书签完整性
browser-bookmark-sync validate

# 设置中枢浏览器并同步（推荐）
browser-bookmark-sync set-hubs \
  --browsers "waterfox,brave-nightly" \
  --sync-history \
  --clear-others

# 预览更改（不实际执行）
browser-bookmark-sync set-hubs --dry-run

# 仅同步书签（所有浏览器）
browser-bookmark-sync sync

# 同步历史记录（最近30天）
browser-bookmark-sync sync-history --days 30

# 定时同步（每30分钟）
browser-bookmark-sync schedule --cron "0 */30 * * * *"
```

## 📐 同步架构

### 中枢浏览器模型

```
┌─────────────────────────────────────────────────────┐
│                    中枢浏览器                        │
│         Waterfox  ←→  Brave Nightly                │
│         (完整数据)     (完整数据)                    │
└─────────────────────────────────────────────────────┘
                         ↑
                    迁移后清空
                         ↑
┌─────────────────────────────────────────────────────┐
│                   非中枢浏览器                       │
│     Chrome | Brave | Safari | LibreWolf            │
│     (清空)   (清空)  (清空)    (清空)               │
└─────────────────────────────────────────────────────┘
```

### 同步规则

1. **书签同步**
   - 选择文件夹结构最完整的浏览器作为基准
   - 保留完整的树形结构（无扁平化）
   - URL 去重（相同 URL 只保留一份）

2. **历史记录同步**
   - 合并所有浏览器的历史记录
   - 按 URL 去重
   - 按最后访问时间排序

3. **Profile 处理**
   - 仅同步 Default Profile
   - 清理其他 Profile 的重复数据

## 📊 验证测试结果

```
测试套件: 6/6 通过 ✅

数据统计:
├── Waterfox: 24,361 URLs, 1,252 文件夹
├── Brave Nightly: 41,661 URLs, 1,936 文件夹
├── 历史记录: 30,301 条（合并去重后）
└── 节省空间: 156MB (减少 92%)
```

## 🔧 安装

```bash
# 克隆仓库
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync

# 编译
cargo build --release

# 运行测试
cargo test --test integration_test

# 安装到系统（可选）
cp target/release/browser-bookmark-sync /usr/local/bin/
```

## 🧪 测试

运行集成测试套件：

```bash
cargo test --test integration_test
```

测试覆盖：
- ✅ 浏览器检测 (`list`)
- ✅ 数据验证 (`validate`)
- ✅ 书签同步 (`sync`)
- ✅ 历史同步 (`sync-history`)
- ✅ 中枢配置 (`set-hubs`)
- ✅ 帮助命令

## ⚠️ 已知限制

1. **浏览器运行时** - 同步前请关闭浏览器，避免数据库锁定
2. **Safari 阅读列表写入** - 仅支持读取（迁移到书签文件夹）
3. **多 Profile** - 仅同步 Default Profile，避免重复

## 📁 项目结构

```
browser-sync/
├── src/
│   ├── main.rs          # CLI 入口
│   ├── browsers.rs      # 浏览器适配器
│   ├── sync.rs          # 同步引擎
│   ├── scheduler.rs     # 定时任务
│   └── validator.rs     # 数据验证
├── tests/
│   └── integration_test.rs  # 测试套件
├── sync-now.command     # 一键同步脚本 (macOS)
├── empty_bookmarks.json # 空书签模板
└── README.md
```

## 📜 许可证

MIT License
