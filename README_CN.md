# 🔄 浏览器书签同步工具

一款功能强大的跨浏览器书签、历史记录和 Cookie 同步工具。具备智能**规则引擎**自动分类书签，以及**中心浏览器架构**进行有序的数据管理。

[English](./README.md)

## ✨ 核心功能

### 🧠 智能整理 (规则引擎)
- **18条内置分类规则** - 根据URL模式自动分类书签
- **自定义规则支持** - 从JSON文件加载自定义规则
- **多维度匹配** - URL、域名、路径和标题模式匹配
- **优先级处理** - 高优先级规则优先匹配

### 🎯 中心浏览器架构
- **指定主浏览器** - 在中心浏览器之间同步，可选清理其他浏览器
- **全数据同步** - 书签、历史记录、阅读列表和Cookie一键同步
- **保留结构** - 完整保留文件夹层级，不会扁平化

### 🔄 数据管理
- **全局去重** - 智能移除整个书签树中的重复URL
- **空文件夹清理** - 自动移除空书签文件夹
- **安全备份** - 每次操作前自动备份

## 🖥️ 支持的浏览器

| 浏览器 | 书签 | 历史记录 | Cookie | 阅读列表 |
|--------|------|----------|--------|----------|
| **Waterfox** | ✅ | ✅ | ✅ | - |
| **Brave Nightly** | ✅ | ✅ | ✅ | - |
| **Brave** | ✅ | ✅ | ✅ | - |
| **Chrome** | ✅ | ✅ | ✅ | - |
| **Safari** | ✅ | ✅ | - | ✅ |
| **Firefox Nightly** | ✅ | ✅ | ✅ | - |

## 🚀 快速开始

### 基本同步

```bash
# 中心浏览器间全量同步（书签+历史+Cookie）
browser-bookmark-sync sync

# 预览变更，不实际执行
browser-bookmark-sync sync --dry-run

# 自定义中心浏览器
browser-bookmark-sync sync --browsers "chrome,brave"
```

### 智能整理

```bash
# 使用规则引擎自动分类所有书签
browser-bookmark-sync smart-organize

# 预览分类结果
browser-bookmark-sync smart-organize --dry-run --show-stats

# 只整理未分类的书签（根目录的）
browser-bookmark-sync smart-organize --uncategorized-only

# 使用自定义规则
browser-bookmark-sync smart-organize --rules-file my-rules.json

# 查看所有可用规则
browser-bookmark-sync list-rules
```

### 清理与维护

```bash
# 移除重复书签
browser-bookmark-sync cleanup --remove-duplicates

# 移除空文件夹
browser-bookmark-sync cleanup --remove-empty-folders

# 完整清理
browser-bookmark-sync cleanup --remove-duplicates --remove-empty-folders
```

## 🧠 规则引擎

智能分类引擎根据URL模式、域名、路径和标题自动将书签整理到对应分类。

### 内置分类

| 优先级 | 分类 | 文件夹名称 | 描述 |
|--------|------|------------|------|
| 100 | **登录** | 登录入口 | 登录页面、SSO、OAuth端点 |
| 90 | **社交** | 社交媒体 | Twitter、Facebook、Instagram等 |
| 85 | **视频** | 视频流媒体 | YouTube、Netflix、B站等 |
| 80 | **开发** | 开发工具 | GitHub、StackOverflow、npm等 |
| 75 | **购物** | 购物网站 | 亚马逊、淘宝、eBay等 |
| 70 | **新闻** | 新闻资讯 | CNN、BBC、路透社等 |
| 65 | **文档** | 文档参考 | 维基百科、ReadTheDocs等 |
| 60 | **云存储** | 云存储 | Google Drive、Dropbox等 |
| 55 | **邮箱** | 邮箱通讯 | Gmail、Outlook等 |
| 50 | **金融** | 金融理财 | PayPal、银行、投资网站 |
| 45 | **AI** | AI工具 | ChatGPT、Claude、Midjourney等 |
| 40 | **设计** | 设计创意 | Figma、Canva、Dribbble等 |
| 35 | **教育** | 教育学习 | Coursera、Udemy等 |
| 30 | **音乐** | 音乐音频 | Spotify、Apple Music等 |
| 25 | **游戏** | 游戏娱乐 | Steam、Epic Games等 |
| 20 | **论坛** | 论坛社区 | Reddit、Quora、V2EX等 |
| 15 | **管理** | 管理后台 | 管理面板、仪表盘 |
| 10 | **API** | API服务 | API端点、Web服务 |

### 自定义规则

创建JSON文件定义自定义规则：

```json
[
  {
    "name": "work-tools",
    "folder_name": "工作工具",
    "folder_name_en": "Work Tools",
    "url_patterns": ["jira", "confluence", "slack"],
    "domain_patterns": ["atlassian.com", "slack.com"],
    "path_patterns": ["/projects", "/workspace"],
    "title_patterns": ["project", "工作"],
    "priority": 95,
    "description": "工作相关的工具和平台"
  }
]
```

使用方法：

```bash
browser-bookmark-sync smart-organize --rules-file work-rules.json
```

### 规则匹配逻辑

每条规则可通过四种方式匹配书签：

1. **URL模式** - 在完整URL中匹配
   - 示例：`login` 匹配 `https://example.com/login`
   
2. **域名模式** - 在域名部分匹配
   - 示例：`github.com` 匹配 `https://github.com/user/repo`
   
3. **路径模式** - 在URL路径中匹配
   - 示例：`/admin` 匹配 `https://example.com/admin/dashboard`
   
4. **标题模式** - 在书签标题中匹配
   - 示例：`文档` 匹配 "API 文档"

规则按优先级处理（从高到低），第一个匹配的规则生效。

## 📐 架构设计

### 中心浏览器模型

```
┌─────────────────────────────────────────────────────┐
│                   中心浏览器                          │
│         Waterfox  ←──────→  Brave Nightly           │
│                                                      │
│   📚 书签         📜 历史记录    🍪 Cookie          │
│   (全量同步)      (全量同步)     (全量同步)          │
└─────────────────────────────────────────────────────┘
                         ↑
              可选: --clear-others
                         ↑
┌─────────────────────────────────────────────────────┐
│                   非中心浏览器                        │
│        Chrome | Brave | Safari | Firefox            │
│              (数据迁移后清理)                         │
└─────────────────────────────────────────────────────┘
```

### 智能去重

去重引擎使用智能规则：

1. **深度优先** - 优先保留文件夹层级更深的书签
2. **时间优先** - 相同深度时，优先保留较新的书签
3. **URL规范化** - 比较时移除尾部斜杠和片段标识符

```
处理前: https://example.com (根目录) + https://example.com (在工作文件夹中)
处理后: https://example.com (仅保留在工作文件夹中)
```

## 📊 命令参考

### 同步命令

| 命令 | 描述 |
|------|------|
| `sync` | 中心浏览器间全量同步（书签+历史+Cookie） |
| `sync --clear-others` | 全量同步 + 清理非中心浏览器数据 |
| `sync-history` | 仅同步全部历史记录 |
| `sync-cookies` | 仅同步Cookie |
| `sync-reading-list` | 同步阅读列表 |
| `sync-scenario` | 跨浏览器同步特定文件夹 |
| `set-hubs` | 配置并同步中心浏览器 |

### 整理命令

| 命令 | 描述 |
|------|------|
| `smart-organize` | **使用规则引擎自动分类书签** |
| `smart-organize --show-stats` | 显示分类统计 |
| `organize` | 将主页书签移到专用文件夹 |
| `list-rules` | 显示所有可用的分类规则 |

### 维护命令

| 命令 | 描述 |
|------|------|
| `cleanup --remove-duplicates` | 移除重复书签 |
| `cleanup --remove-empty-folders` | 移除空书签文件夹 |
| `validate` | 检查所有浏览器的数据完整性 |
| `list` | 显示检测到的浏览器和路径 |

### 通用选项

```bash
# 大多数命令的通用选项
-b, --browsers <BROWSERS>    目标浏览器（逗号分隔）
-d, --dry-run                预览模式，不实际修改
-v, --verbose                详细输出

# smart-organize 特有选项
-r, --rules-file <FILE>      从JSON文件加载自定义规则
    --uncategorized-only     仅整理根目录书签
    --show-stats             显示分类统计
```

## 📊 测试结果

```
测试套件: 48个测试 (40单元测试 + 8集成测试) ✅

同步统计:
├── 书签: 41,661 URLs, 1,936 文件夹
├── 历史记录: 30,301 条去重后
├── Cookie: 925 个去重后
├── 规则引擎: 18条内置分类规则
└── 性能: ~1.1秒 (release构建)
```

## 🔧 安装

```bash
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync
cargo build --release

# 运行测试
cargo test

# 安装（可选）
cp target/release/browser-bookmark-sync /usr/local/bin/
```

## ⚠️ 注意事项

1. **同步前关闭浏览器** - 运行中的浏览器会覆盖更改
2. **自动备份** - 保存到 `~/Desktop/browser_backup_*`
3. **默认中心浏览器** - Waterfox + Brave Nightly（可用 `--browsers` 自定义）
4. **受保护文件夹** - 已存在的分类文件夹不会被重新整理

## 📁 项目结构

```
browser-bookmark-sync/
├── src/
│   ├── main.rs          # CLI命令和入口
│   ├── sync.rs          # 同步引擎和规则引擎
│   ├── browsers.rs      # 浏览器适配器 (Chromium/Firefox/Safari)
│   ├── validator.rs     # 数据验证
│   └── scheduler.rs     # 定时同步调度器
├── tests/
│   └── integration_test.rs
├── examples/
│   └── custom-rules.json
└── Cargo.toml
```

## 📜 许可证

MIT License
