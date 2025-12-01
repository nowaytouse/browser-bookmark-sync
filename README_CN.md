# 🔄 Browser Sync (macOS) - 浏览器同步工具

一款强大的、仅适用于 macOS 的命令行工具，用于在多个浏览器之间同步和管理书签、历史记录和阅读列表。它采用先进的 **“基准合并” (Base & Merge)** 同步策略、智能的 **规则引擎整理器** 以及用于处理云端同步冲突的高级工具。

[English Document](./README.md)

## ✨ 核心功能

- **macOS 原生:** 与 macOS 上的浏览器数据文件（SQLite, Plist）进行深度集成。
- **高级同步逻辑:** 采用 **“基准合并”** 策略，将书签结构最规整的浏览器作为模板，同步到其他浏览器。
- **智能去重:** 根据书签所在文件夹的深度和添加时间，自动解决重复书签的冲突。
- **智能整理器:** 一个强大的、基于优先级的引擎，内置 **75 条双语 (中/英) 规则**，可自动将书签分类到不同文件夹。支持通过自定义 JSON 文件进行扩展。
- **Firefox Sync 集成:** 提供两种策略（`api` 或 `local`）来安全地与 Firefox Sync 同步，防止云端数据覆盖本地更改。
- **完整数据管理:** 提供一整套丰富的工具，用于 `sync` (同步), `cleanup` (清理), `organize` (整理), `validate` (校验), `backup` (备份), `restore` (恢复) 和 `schedule` (定时任务)。
- **云端重置向导:** 一个引导式的流程 (`cloud-reset`)，用于解决复杂的 Firefox Sync 数据冲突。
- **Cron 定时任务:** 基于 Cron 表达式，在后台自动执行同步操作。

## 🖥️ 支持的平台

**本工具仅适用于 macOS。** 它需要直接访问特定于浏览器的数据库和属性列表文件。

| 浏览器 | 书签 | 历史记录 | 阅读列表 | 备注 |
| :--- | :---: | :---: | :---: | :--- |
| **Waterfox** | ✅ | ✅ | - | 可作为核心浏览器 |
| **Brave Nightly** | ✅ | ✅ | - | 可作为核心浏览器 |
| **Brave** | ✅ | ✅ | - | |
| **Chrome** | ✅ | ✅ | - | |
| **Safari** | ✅ | ✅ | ✅ | |
| **Firefox Nightly**| ✅ | ✅ | - | |

---

## 🔬 工作原理

### “基准合并” (Base & Merge) 同步策略

本工具**并非执行简单的双向合并**。为确保结果清晰有序，它采用 “基准合并” 策略：

1.  **分析 (Analyze):** 读取**所有**指定浏览器中的书签数据。
2.  **评分 (Score):** 对每个浏览器的文件夹结构进行评分。拥有更多文件夹和书签的浏览器会获得更高的分数，这优先考虑了“组织性”。
3.  **选择基准 (Select Base):** 得分**最高**的浏览器被选为 **“基准” (base)**。其结构将成为本次同步的唯一真实来源。
4.  **合并与去重 (Merge & Deduplicate):** 将其他浏览器的书签合并到基准结构中。重复项将通过智能去重逻辑解决。
5.  **覆盖 (Overwrite):** 最终合并完成的书签集将被**写回到所有核心浏览器 (hub browsers)**，覆盖它们之前的书签数据。

> ⚠️ **重要提示**: 这是一个单向过程。如果一个书签存在于得分较低的浏览器中，但不存在于“基准”浏览器中，那么在同步后它将**被删除**。此设计旨在优先保证单一、清晰的结构，而不是保留分散各处的书签。

### 智能去重逻辑

当发现重复的 URL 时，工具会使用以下两条规则来解决冲突：
1.  **深度优先:** 保留位于更深层文件夹结构中的书签。
2.  **时间优先:** 如果文件夹深度相同，则保留最近添加的书签。

### Firefox Sync: 双重策略

为防止与 Mozilla 服务器发生冲突，本工具在本地合并后，提供两种方式来处理 Firefox Sync：

-   `--firefox-sync=api`: (默认) 工具将作为**直接的 API 客户端**。它会使用您的 Firefox 账户进行身份验证，并将新合并的书签集上传到 Mozilla 服务器，使其成为云端新的唯一真实来源。
-   `--firefox-sync=local`: 工具会**触发浏览器自身的内部同步机制**。这是一种较为间接的方式，由浏览器自己执行同步。

---

## 🚀 安装

1.  **环境准备:** 确保您已安装 Rust 和 Cargo。
2.  **克隆并构建:**

    ```bash
    git clone https://github.com/your-username/browser-bookmark-sync.git
    cd browser-bookmark-sync
    cargo build --release
    ```
3.  **安装 (可选):** 将可执行文件复制到您的 PATH 路径下。
    ```bash
    cp target/release/browser-bookmark-sync /usr/local/bin/
    ```

## 📖 命令用法 (Commands)

所有命令都通过 `browser-bookmark-sync <COMMAND>` 运行。

### 核心命令

| 命令 | 描述 | 示例 |
| :--- | :--- | :--- |
| `sync` | 主命令。使用“基准合并”策略同步核心浏览器之间的书签、历史记录和阅读列表。 | `browser-bookmark-sync sync` |
| `smart-organize`| **使用规则引擎自动分类所有书签**。 | `browser-bookmark-sync smart-organize` |
| `cleanup` | 在不进行完全同步的情况下，删除重复书签和/或空文件夹。 | `browser-bookmark-sync cleanup --remove-duplicates` |
| `schedule` | 启动守护进程，按 cron 计划执行同步任务。 | `browser-bookmark-sync schedule --cron "0 * * * *"` |
| `validate` | 检查数据完整性，寻找重复或格式错误的数据。 | `browser-bookmark-sync validate --detailed` |
| `cloud-reset` | 启动一个**引导式向导**来解决 Firefox Sync 服务器数据问题。 | `browser-bookmark-sync cloud-reset` |
| `list` | 列出所有检测到的浏览器及其数据路径。 | `browser-bookmark-sync list` |
| `list-rules` | 显示所有 75 条内置的分类规则。 | `browser-bookmark-sync list-rules` |

### 通用选项

-   `--dry-run`: 预览更改，但不会修改任何文件。**强烈建议首次使用时开启此项。**
-   `--browsers "brave,safari"`: 指定要操作的浏览器。
-   `--firefox-sync <api|local>`: (用于 `sync`) 选择 Firefox Sync 同步策略。
-   `-v, --verbose`: 启用详细的日志输出。

### 工作流示例

#### 首次同步 (预览)
```bash
# 查看在不更改任何内容的情况下，同步会执行哪些操作。
browser-bookmark-sync sync --dry-run -v
```

#### 日常同步
```bash
# 在默认的核心浏览器 (Waterfox, Brave Nightly) 之间同步。
browser-bookmark-sync sync
```

#### 完全重新整理
```bash
# 使用规则引擎将每个书签归类。
browser-bookmark-sync smart-organize --show-stats
```

#### 每小时同步一次
```bash
# 在后台运行调度器。(建议使用如 launchd 等进程管理器以确保持久运行)
browser-bookmark-sync schedule --cron "0 * * * *" &
```

---

## 🧠 用于整理的规则引擎

`smart-organize` 命令使用强大的引擎来自动分类您的书签。

-   **基于优先级:** 优先级更高的规则会首先被检查。第一个匹配成功的规则将决定书签的分类。
-   **多维匹配:** 规则可以根据书签的 URL、域名、路径或标题进行匹配。
-   **双语支持:** 所有 75 条内置规则都包含中文和英文的文件夹名称。
-   **可扩展:** 您可以通过 JSON 文件提供自己的规则。

#### 自定义规则

创建一个 `my-rules.json` 文件:
```json
[
  {
    "name": "work-tools",
    "folder_name": "工作工具",
    "folder_name_en": "Work Tools",
    "url_patterns": ["jira", "confluence"],
    "domain_patterns": ["atlassian.com"],
    "priority": 110,
    "description": "用于工作的 Atlassian 工具栈。"
  }
]
```

然后使用它:
```bash
# 使用您的自定义规则运行整理，它将拥有更高优先级。
browser-bookmark-sync smart-organize --rules-file my-rules.json
```
---

## ⚠️ 重要说明

1.  **关闭您的浏览器:** 在运行任何同步或清理操作之前，必须完全关闭浏览器。本工具会直接修改数据库文件，如果浏览器正在运行，这些更改将被覆盖。
2.  **自动备份:** 在执行任何破坏性操作之前，工具会自动将您的浏览器配置文件备份到 `~/Desktop/browser_backup_*`。
3.  **默认核心浏览器:** 默认用于同步的“核心”浏览器是 Waterfox 和 Brave Nightly。您可以使用 `--browsers` 标志进行更改。

## 📜 许可证

本项目基于 MIT 许可证授权。