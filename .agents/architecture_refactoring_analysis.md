# browser-bookmark-sync 架构分析与主重构设计文档

> **文档性质**：对现有 `nowaytouse/browser-bookmark-sync` 仓库的全面架构审计与重构设计说明  
> **核心目标**：实现高保真、只读优先、可审计的全浏览器与全 Profile 书签采集、精确去重与侧写审计

---

## 1. 现状架构分析与缺陷审计

基于对当前代码库（`src/`）与新计划（`.agents/plan.md`、`.agents/code.md`）的深度对比，现有实现主要在**数据建模血统**、**浏览器发现与快照隔离**、**功能参数解耦**和**审计可追溯性**四个维度存在明显的重构缺口。

### 1.1 核心数据模型血统丢失 (Data Lineage Deficit)

- **问题现状**：
  - `src/browsers.rs` 中的 `Bookmark` 结构体主要由简单基本类型组成 (`id`, `title`, `url`, `folder`, `children`, `date_added`, `date_modified`)。
  - **无来源元数据**：当通过 `read_all_chromium_bookmarks` 或 `read_all_firefox_bookmarks` 枚举多个 Profile 时，程序仅仅使用 `all_bookmarks.extend(bookmarks)` 将各个 Profile 的书签直接拼合，书签节点上不携带任何 `SourceDescriptor`（浏览器名称、Release Channel、Profile ID、显示名称及配置文件路径）。
  - **无根结构语义**：缺乏 `BookmarkRootKind` 枚举来区分浏览器专属的顶级根节点（如 `BookmarkBar` 书签栏、`OtherBookmarks` 其他书签、`MobileBookmarks` 移动设备书签、`ReadingList` 阅读清单等）。
  - **实体与统计混合**：未能区分“原始出现记录 (`BookmarkOccurrence`)”与“规范主书签 (`CanonicalBookmark`)”，去重操作只能通过原地从 `Vec<Bookmark>` 中剔除元素来实现。
- **架构影响**：
  - 数据一旦合并，下游模块便不可逆地丢失了**来源追溯能力**。
  - 无法同时支持“保留原浏览器/Profile层级结构”和“全量书签去重”。

### 1.2 浏览器发现硬编码与只读快照隔离缺失 (Discovery & Snapshot Isolation)

- **问题现状**：
  - **硬编码的 `BrowserType` 枚举**：当前 `src/browsers.rs` 中的 `BrowserType` 仅写死了 `Waterfox`, `Safari`, `Brave`, `BraveNightly`, `Chrome`, `FirefoxNightly`, `LibreWolf` 7 个成员。不支持广义 Chromium 家族（如 Edge, Arc, Chrome Beta/Dev/Canary 等）及广义 Firefox 家族（如标准版 Firefox, Developer Edition, Floorp 等）。
  - **Profile 发现原始且简陋**：`detect_all_profiles()` 仅以目录路径扫描形式返回 `Vec<PathBuf>`，未解析 Chrome `Local State` 或 Firefox `profiles.ini` / `installs.ini` 中的用户可读显示名称。
  - **非一致性读取与强制杀进程**：当前代码大量直接读取实际使用的浏览器数据库（部分情况采用 SQLite `backup`）；此外 `src/browser_utils.rs` 还存在 `close_browsers()` 强行关闭浏览器进程的行为，严重违反“高保真、只读优先、绝不修改或干扰原始环境”的架构原则。

### 1.3 CLI 命令与引擎职责过度融合 (CLI & Engine Conflation)

- **问题现状**：
  - **`--merge` 选项承担了三重语义**：同时表达“将多个浏览器的顶级根合并为单一树”、“扁平化不同浏览器子目录”以及“触发全局书签匹配”，这使得用户无法导出**多浏览器未去重的原生分层结构 HTML**。
  - **无法输出可审计审计链路**：现有的 `deduplicate_bookmarks_global()` 方法是**破坏性合并**（Destructive Deduplication），没有向外输出 `inventory.json`、`duplicates.json`、`duplicates.csv` 与 `run-manifest.json` 等对于合规运维与对账至关重要的侧写文件。

### 1.4 URL 验证模块判定颗粒度粗糙 (Validation Classification Deficiency)

- **问题现状**：
  - 现存 `src/url_checker.rs` 将 4xx/5xx 及网络超时等响应一律视为死链（Dead Links）。
  - 在实际场景中，现代站点大量使用 Cloudflare / Bot Protection (403/429) 或存在服务器维护 (502/504)，应当建立分层的响应判定策略（永久死链 vs 临时网络失效 vs 软拒绝）。

---

## 2. 目标重构分层架构设计 (Target Layered Architecture)

为了满足高保真和无损审计原则，系统必须拆分为 **5 个边界清晰的抽象层**：

```
+--------------------------------------------------------------------------------+
|                        1. CLI & 调度层 (CLI & Dispatcher)                       |
|   bsync inventory  |  bsync collect  |  bsync dedupe  |  bsync pipeline        |
+--------------------------------------------------------------------------------+
                                       |
                                       v
+--------------------------------------------------------------------------------+
|               2. 浏览器发现与快照隔离层 (Discovery & Snapshot Layer)             |
|   BrowserInstallation (Chrome/Firefox/Safari...) -> Profile Enumeration        |
|   Snapshot Engine (Copy-on-Read / SQLite WAL Safe Copy / Zero DB Mutation)     |
+--------------------------------------------------------------------------------+
                                       |
                                       v
+--------------------------------------------------------------------------------+
|                 3. 统一内部数据模型层 (Unified Data Model Layer)                 |
|   SourceDescriptor | BookmarkRootKind | BookmarkNode (Folder/Bookmark/Sep)     |
+--------------------------------------------------------------------------------+
                                       |
                                       v
+--------------------------------------------------------------------------------+
|               4. 聚合与去重引擎层 (Aggregation & Deduplication)                 |
|   Exact Match Normalizer -> Deterministic Priority Selector (CanonicalBookmark)|
+--------------------------------------------------------------------------------+
                                       |
                                       v
+--------------------------------------------------------------------------------+
|              5. 多形态输出与审计报告层 (Multi-Format Export & Audit)             |
|   bookmarks-original.html | bookmarks-deduplicated.html | inventory.json       |
|   duplicates.json | duplicates.csv | run-manifest.json                         |
+--------------------------------------------------------------------------------+
```

### 2.1 架构核心设计方针

1. **树形完整性与全局去重二元分离原则**：  
   Netscape HTML 的标准 DOM 无法通过“一个 HTML 节点”同时完整描述多个不同浏览器及不同文件夹中的同一 URL。因此，输出引擎必须分离为两个独立可产生的产物：
   - **`bookmarks-original.html`**：100% 原始高保真、不去重，保留全量浏览器 -> Profile -> 根目录 -> 文件夹层级结构。
   - **`bookmarks-deduplicated.html`**：全局精确 URL 去重，根据确定的选取策略指定最权威的“主书签”，其它来源作为重复项以审计报告形式提供。
2. **读写分离与只读快照机制**：
   - 所有数据读取必须经过 `snapshot_profile()` 层，优先读取 SQLite WAL 或进行临时隔离复制，**禁止直接对用户运行时浏览器目录施加锁定或杀进程**。

---

## 3. 核心接口与数据模型说明 (Core Traits & Data Models)

### 3.1 统一内部抽象树模型 (`src/data_types/models.rs`)

应当废弃或改写原有平面结构，采用带有来源与描述的复合模型：

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 浏览器基础类型分类
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrowserKind {
    Chromium(String), // 例如 Chrome, Brave, Edge, Arc
    Firefox(String),  // 例如 Firefox, Waterfox, LibreWolf, Floorp
    Safari,
}

/// 唯一的书签原始来源标识
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDescriptor {
    pub browser: BrowserKind,
    pub browser_channel: Option<String>,
    pub profile_id: String,
    pub profile_name: String,
    pub source_path: PathBuf,
}

/// 标准书签顶级根类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BookmarkRootKind {
    BookmarkBar,
    OtherBookmarks,
    MobileBookmarks,
    BookmarkMenu,
    Toolbar,
    Unfiled,
    ReadingList,
    Favorites,
    Custom(String),
}

/// 树节点枚举
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BookmarkNode {
    Folder(FolderNode),
    Bookmark(BookmarkOccurrence),
    Separator(SeparatorNode),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderNode {
    pub id: String,
    pub title: String,
    pub children: Vec<BookmarkNode>,
    pub source: SourceDescriptor,
    pub root_kind: BookmarkRootKind,
    pub original_position: usize,
    pub date_added: Option<DateTime<Utc>>,
    pub date_modified: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkOccurrence {
    pub id: String,
    pub title: String,
    pub raw_url: String,
    pub source: SourceDescriptor,
    pub root_kind: BookmarkRootKind,
    pub folder_path: Vec<String>,
    pub original_position: usize,
    pub date_added: Option<DateTime<Utc>>,
    pub date_modified: Option<DateTime<Utc>>,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeparatorNode {
    pub id: String,
    pub source: SourceDescriptor,
    pub original_position: usize,
}
```

### 3.2 规范主书签与去重审计模型 (`src/data_types/canonical.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SelectionReason {
    PreferredSource,
    PreferredRootKind,
    DeeperFolderPath,
    EarliestDateAdded,
    DeterministicFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalBookmark {
    pub canonical_id: String,
    pub exact_url: String,
    pub primary: BookmarkOccurrence,
    pub duplicates: Vec<BookmarkOccurrence>,
    pub selection_reason: Vec<SelectionReason>,
}
```

### 3.3 浏览器适配器标准接口 (`src/browsers/traits.rs`)

```rust
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct BrowserInstallation {
    pub name: String,
    pub kind: BrowserKind,
    pub channel: Option<String>,
    pub root_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct BrowserProfile {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ProfileSnapshot {
    pub original_profile: BrowserProfile,
    pub snapshot_path: PathBuf,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub trait BrowserAdapter: Send + Sync {
    /// 自动发现本机安装的所有具体发布版本与变体
    fn discover_installations(&self) -> Result<Vec<BrowserInstallation>>;

    /// 枚举该安装分支下的全量用户配置文件 (通过 Local State / profiles.ini)
    fn discover_profiles(&self, installation: &BrowserInstallation) -> Result<Vec<BrowserProfile>>;

    /// 建立临时隔离只读目录或快照，绝不写入源目录
    fn snapshot_profile(&self, profile: &BrowserProfile, destination: &Path) -> Result<ProfileSnapshot>;

    /// 从已生成的安全快照读取书签树
    fn read_bookmarks(&self, snapshot: &ProfileSnapshot) -> Result<Vec<BookmarkNode>>;
}
```

---

## 4. 主重构设计：精确去重与主书签选择策略

去重引擎核心将实现自 `src/sync/dedupe.rs`。必须遵循**确定性（Deterministic）**与**最小改动匹配（Exact Only）**原则：

### 4.1 匹配模式策略

- 当前阶段**默认仅执行精确匹配 (`exact`)**：
  - 保留 URL 协议（不自动跨 `http` / `https` 合并）。
  - 保留 Query Params 与 Hash Fragments（不支持自动裁切 `utm_*` 或其他参数）。
  - 如需后续扩展，应设计为单独的命令行 Mode 标签 (`--mode exact|normalized|redirect|similar`)。

### 4.2 主书签选择规则优先度 (Tie-Breaking Strategy)

当不同 Profile 或文件夹下出现同一精确 URL 时，严格遵守以下选优规则逐级决策：

1. **用户配置文件指定优先级 (Source Priority)**：通过配置文件指定默认浏览器权威度（如 `Brave Nightly` > `Safari` > `Chrome` > `Firefox`）。
2. **Profile 层级**：默认配置文件（如 `Default` / `default-release`）优先级高于临时自定义 Profile。
3. **书签根节点类型 (`BookmarkRootKind`)**：
   - `BookmarkBar` > `BookmarkMenu` / `Toolbar` > `OtherBookmarks` > `Unfiled`。
4. **目录深度及路径语义**：
   - 处于具有明确层级定义的非空自定义目录下（例如 `开发/Rust/架构`）优先于根目录或未分类目录。
5. **最早创建时间 (`date_added`)**：优先保留最早添加的书签节点。
6. **确定性字典序终评 (Deterministic ID Fallback)**：比较节点唯一 UUID 或通过路径哈希保证多次执行时所选择主节点绝对不变。

---

## 5. CLI 命令行体系重构建议

在 `src/main.rs` 及 `src/sync_flags.rs` 中重构 Clap CLI，实现标准工程职责分离：

```text
bsync inventory  --all-browsers --all-profiles
bsync collect    --all-browsers --all-profiles --preserve-source-tree --output-dir ./run
bsync dedupe     --input ./run/inventory.json --mode exact --output ./run/dedup.html
bsync pipeline   --all-browsers --all-profiles --dedupe exact --output-dir ./run
bsync check      --input ./run/dedup.html --retry 2 --classify
```

### 5.1 命令与标志行为对齐表

| 命令 / 标志              | 参数职责说明                                      | 重构改造策略                                                             |
| ------------------------ | ------------------------------------------------- | ------------------------------------------------------------------------ |
| `bsync inventory`        | 列举系统已发现的 Browser、Profile 清单及书签数量  | 新增命令；以只读快照扫描，生成静态概况信息，不出 HTML                    |
| `bsync collect`          | 高保真提取所有 Browser / Profile 的原始结构并导出 | 新增命令；输出 `bookmarks-original.html` 及 `inventory.json`             |
| `bsync dedupe`           | 读取 inventory 或内部树模型进行精确匹配去重       | 新增命令；输出 `bookmarks-deduplicated.html` 与 `duplicates.json/csv`    |
| `bsync pipeline`         | 聚合全自动化工作流                                | 新增命令；顺序完成 collect -> dedupe -> report 并输出完整构建包          |
| `--preserve-source-tree` | `collect` 及 `pipeline` 的默认输出控制            | 替代原有的隐式层级打平行为，保持顶级以 `Browser -> Profile -> Root` 分层 |
| `--flatten`              | 可选控制                                          | 显式声明移除上一级的包围层，用于需导入不支持深度嵌套结构的浏览器         |
| `--merge-folders`        | 可选控制                                          | 显式合并相同名称的子目录                                                 |
| `--deduplicate`          | 明确为独立转换工序                                | 仅控制是否生成去重处理后的衍生 HTML                                      |

---

## 6. URL 有效性验证分类规划 (`src/url_checker.rs`)

将原有简单直接标记死链的逻辑修改为**三维响应健康状态分类机**：

```
                              HTTP / TCP 请求
                                    |
            +-----------------------+-----------------------+
            |                       |                       |
            v                       v                       v
     [ 永久死链/无效 ]        [ 瞬时错误/网络不通 ]      [ 软性拦截/限流 ]
     • 404 Not Found         • DNS 临时解析失败       • 403 Forbidden
     • 410 Gone              • TCP Connect Timeout    • 429 Too Many Req
     • NXDOMAIN              • 500/502/503/504        • Cloudflare Intercept
            |                       |                       |
            v                       v                       v
    建议：可进入待清理        建议：保留并重试           建议：保留并标记
```

---

## 7. 分阶段实施路线与代码修改索引 (Implementation Roadmap)

| 阶段        | 主干任务                                | 对应修改或创建的核心文件                                                        |
| ----------- | --------------------------------------- | ------------------------------------------------------------------------------- |
| **Phase 0** | 仓库结构与用例行为基线测试审计          | `tests/integration_test.rs`, `proptest-regressions/`                            |
| **Phase 1** | 统一内部数据模型抽象建立                | `src/data_types/models.rs`, `src/data_types/canonical.rs`                       |
| **Phase 2** | 浏览器与 Profile 发现引擎及安全快照     | `src/browsers/*.rs`, `src/browser_utils.rs`（移除 `close_browsers` 暴力操作）   |
| **Phase 3** | 精确去重引擎与双结构渲染导出器          | `src/sync/dedupe.rs`, `src/sync/export_original.rs`, `src/sync/export_dedup.rs` |
| **Phase 4** | 侧写审计（JSON/CSV）及 `inventory` 生成 | `src/sync/audit.rs`, `src/cleanup.rs`                                           |
| **Phase 5** | CLI 子命令（Clap Subcommand 体系改造）  | `src/main.rs`, `src/sync_flags.rs`                                              |
| **Phase 6** | 分层 URL 状态检查引擎升级               | `src/url_checker.rs`                                                            |
| **Phase 7** | 向下兼容适配与发布验收测试              | `CHANGELOG.md`, `README.md`, E2E 测试补充                                       |

---

## 8. 验收标准清单 (Acceptance Criteria)

- [ ] 所有安装有对标的 Chromium 系与 Firefox 系变种浏览器及其全量 Profiles 能被零干预自动发现。
- [ ] 导出 `bookmarks-original.html` 时，原有层级（Browser -> Profile -> Root -> Folders）及书签顺序和标题 100% 完整保留。
- [ ] 导出的 `bookmarks-deduplicated.html` 中任何完全等同的 URL 最多仅存在一行数据条目，主书签选定符合确定的优选策略。
- [ ] 产生完整的附加侧写文件包 (`inventory.json`, `duplicates.json`, `duplicates.csv`, `run-manifest.json`) 且格式符合 Schema 规范。
- [ ] 没有任何对浏览器本地生产 SQLite 数据库的原地修改或加锁冲突。
