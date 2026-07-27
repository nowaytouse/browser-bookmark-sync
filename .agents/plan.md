# browser-bookmark-sync 高保真书签汇总与清理改造计划

## 一、目标

基于现有 `nowaytouse/browser-bookmark-sync` 项目继续开发，不从零重写。

本阶段目标是：

1. 自动发现 macOS 上所有受支持浏览器。
2. 自动发现每个浏览器的全部 Profile。
3. 以只读方式读取全部书签。
4. 保留浏览器、Profile、浏览器根目录、文件夹层级、节点顺序和书签顺序。
5. 输出一份不做任何去重的完整原始备份。
6. 对全部来源执行全局精确 URL 去重。
7. 每个精确 URL 在去重结果中只保留一个书签。
8. 保留所有重复项原始来源、标题、路径、时间和位置记录。
9. 输出可审计的 JSON、CSV 和 HTML。
10. 为下一阶段 URL 有效性验证提供稳定输入和接口。
11. 默认绝不修改浏览器原始数据库。

---

## 二、重要数据原则

### 1. 原始结构保留和全局唯一不能由同一份 HTML 同时完整表达

假设同一个 URL 分别存在于：

- Brave Nightly / Default / 书签栏 / 开发
- Firefox / default-release / 书签菜单 / 常用
- Safari / 收藏夹 / 临时

要求 URL 只保留一次以后，另外两个文件夹位置必然消失。

标准 Netscape Bookmark HTML 没有“同一书签节点同时引用多个文件夹”的能力。

因此必须输出两套结果：

### 完整原始版本

`bookmarks-original.html`

- 不去重；
- 完整保留来源；
- 完整保留文件夹结构；
- 用于恢复和人工检查。

### 全局去重版本

`bookmarks-deduplicated.html`

- 每个精确 URL 只保留一次；
- 使用确定性规则选择主要书签；
- 用于后续导入或 URL 有效性检查。

同时输出：

- `inventory.json`
- `duplicates.json`
- `duplicates.csv`
- `run-manifest.json`

这些 sidecar 文件保存 HTML 无法表达的来源与合并信息。

---

## 三、现有行为调整

现有项目中的以下功能应当保留：

- 浏览器自动检测；
- 浏览器适配器；
- Netscape HTML 导出；
- 数据库副本读取；
- dry-run；
- backup；
- analyze；
- validate；
- check；
- 代理与直连请求能力。

以下行为需要重新定义：

### `--merge`

当前 `--merge` 会扁平化多个浏览器的目录结构。

改造后应拆分为：

- `--preserve-source-tree`
- `--flatten`
- `--merge-folders`
- `--deduplicate`

默认使用 `--preserve-source-tree`。

不要让 `--merge` 同时承担“合并来源”“扁平目录”“去重”三种不同语义。

### `--flat`

保留作为明确的可选转换操作，但不得作为多浏览器汇总的默认行为。

### `--deduplicate`

默认仅执行精确 URL 去重。

不得默认执行：

- HTTP → HTTPS 合并；
- 移除 `www`；
- 删除 URL fragment；
- 删除查询参数；
- 删除 `utm_*`；
- 跟随重定向后合并；
- 标题相似度合并；
- 网页内容相似度合并。

这些以后应作为独立模式：

- `exact`
- `normalized`
- `redirect`
- `similar`

当前阶段只实现或默认启用 `exact`。

---

## 四、内部数据模型

不要在解析器中直接生成 HTML。

先建立统一的、与浏览器格式无关的内部树模型。

建议结构：

```rust
struct SourceDescriptor {
    browser: BrowserKind,
    browser_channel: Option<String>,
    profile_id: String,
    profile_name: String,
    source_path: PathBuf,
}

enum BookmarkRootKind {
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

enum BookmarkNode {
    Folder(FolderNode),
    Bookmark(BookmarkOccurrence),
    Separator(SeparatorNode),
}

struct FolderNode {
    id: NodeId,
    title: String,
    children: Vec<BookmarkNode>,
    source: SourceDescriptor,
    root_kind: BookmarkRootKind,
    original_position: usize,
    date_added: Option<DateTime<Utc>>,
    date_modified: Option<DateTime<Utc>>,
}

struct BookmarkOccurrence {
    id: NodeId,
    title: String,
    raw_url: String,
    source: SourceDescriptor,
    root_kind: BookmarkRootKind,
    folder_path: Vec<String>,
    original_position: usize,
    date_added: Option<DateTime<Utc>>,
    date_modified: Option<DateTime<Utc>>,
    metadata: BTreeMap<String, serde_json::Value>,
}
```

去重后增加：

```rust
struct CanonicalBookmark {
    canonical_id: String,
    exact_url: String,
    primary: BookmarkOccurrence,
    duplicates: Vec<BookmarkOccurrence>,
    selection_reason: SelectionReason,
}
```

注意：

- `BookmarkOccurrence` 表示一次真实出现。
- `CanonicalBookmark` 表示多个出现记录合并后的逻辑书签。
- 不要在采集阶段丢弃任何 occurrence。
- 不要只使用 URL 作为原始节点 ID。
- 节点 ID 应能稳定描述来源、Profile、路径与原始节点标识。

---

## 五、浏览器和 Profile 发现

建立统一 trait：

```rust
trait BrowserAdapter {
    fn discover_installations(&self) -> Result<Vec<BrowserInstallation>>;
    fn discover_profiles(
        &self,
        installation: &BrowserInstallation,
    ) -> Result<Vec<BrowserProfile>>;
    fn snapshot_profile(
        &self,
        profile: &BrowserProfile,
        destination: &Path,
    ) -> Result<ProfileSnapshot>;
    fn read_bookmarks(
        &self,
        snapshot: &ProfileSnapshot,
    ) -> Result<BookmarkTree>;
}
```

### Chromium 系

至少处理：

- Chrome
- Chrome Beta、Dev、Canary，如安装
- Chromium
- Brave
- Brave Beta、Nightly，如安装
- Edge
- Edge Beta、Dev、Canary，如安装
- Arc
- Vivaldi，如项目准备支持
- Opera，如项目准备支持

从浏览器 User Data 根目录枚举：

- `Default`
- `Profile 1`
- `Profile 2`
- 其他符合 Profile 规则的目录

不要仅解析当前活动 Profile。

读取每个 Profile 的：

- `Bookmarks`
- `Bookmarks.bak`，仅作为故障恢复来源
- `Local State`，用于获取 Profile 显示名称和元数据

保留 Chromium 根节点：

- `bookmark_bar`
- `other`
- `synced`
- 未来出现的未知根节点

未知节点不得静默丢弃。

### Firefox 系

至少处理：

- Firefox
- Firefox Developer Edition
- Firefox Nightly
- LibreWolf
- Waterfox
- Zen，如结构兼容

通过：

- `profiles.ini`
- `installs.ini`

枚举全部 Profile。

读取复制后的：

- `places.sqlite`
- 相关 WAL/SHM，或使用 SQLite backup API 生成一致性快照

重建：

- Toolbar
- Menu
- Unfiled
- Mobile
- Tags
- 自定义文件夹
- 节点位置

不要将同名但 parent 不同的文件夹合并。

### Safari

读取 Safari 书签时必须：

- 使用只读副本；
- 明确处理 macOS 权限错误；
- 保留 Favorites、Bookmarks Menu、Reading List 等根节点；
- 不把 Safari Profile 误当成 Chromium 式独立书签 Profile；
- 如果 Safari 的多个浏览配置共享书签，manifest 中必须说明这一事实。

---

## 六、安全快照

每次运行创建独立工作目录：

```text
runs/
└── 2026-07-27TxxxxxxZ/
    ├── snapshots/
    ├── output/
    ├── reports/
    └── run-manifest.json
```

`run-manifest.json` 记录：

- 运行版本；
- Git commit；
- 开始和结束时间；
- macOS 版本；
- 检测到的浏览器；
- Profile 列表；
- 原始文件路径；
- 快照路径；
- 文件大小；
- SHA-256；
- 读取警告；
- 解析警告；
- 导出数量；
- 去重前后数量。

要求：

- 所有解析只针对 snapshot。
- 不直接读取并长期占用浏览器活跃数据库。
- 不修改原始 `Bookmarks`、`places.sqlite` 或 Safari 数据。
- 浏览器运行中时，要么创建一致性快照，要么明确要求关闭浏览器。
- 失败时不得留下半成品冒充成功结果。
- 输出文件使用临时文件＋原子 rename。

---

## 七、完整原始导出

顶层建议结构：

```text
All Browser Bookmarks
├── Brave Nightly
│   ├── Default
│   │   ├── Bookmarks Bar
│   │   ├── Other Bookmarks
│   │   └── Mobile Bookmarks
│   └── Profile 1
├── Firefox
│   └── default-release
├── LibreWolf
│   └── default-default
└── Safari
    ├── Favorites
    ├── Bookmarks Menu
    └── Reading List
```

要求：

- 保留节点顺序；
- 保留目录顺序；
- 保留空文件夹，除非用户明确传入 `--clean-empty-folders`；
- 保留分隔符；
- 正确转义 HTML；
- 保留 Unicode 和 emoji；
- 不自动重命名同名文件夹；
- 不把来源目录折叠；
- 不执行 URL 标准化；
- 不执行去重。

---

## 八、精确 URL 去重

### 精确匹配键

默认只执行：

```rust
let exact_key = raw_url.trim();
```

不得改变 URL 的其他部分。

以下不应视为精确重复：

```text
http://example.com/a
https://example.com/a
```

```text
https://example.com/a
https://example.com/a/
```

```text
https://example.com/a#one
https://example.com/a#two
```

```text
https://example.com/a?utm_source=x
https://example.com/a
```

只有 trim 后完整字符串相同才属于精确重复。

### 确定性主书签选择

建立可配置优先级：

1. 用户显式配置的浏览器/Profile 优先级；
2. 路径更具体且不是临时目录的 occurrence；
3. 标题非空；
4. 标题不等于裸 URL；
5. 标题信息量更高；
6. 原始创建时间更早；
7. 来源排序稳定；
8. 文件夹路径字典序；
9. 原始位置；
10. 稳定节点 ID。

同样输入必须始终生成同样结果。

不要依赖 HashMap 随机遍历顺序。

### 默认来源优先级配置

新增配置文件：

```toml
[dedup]
mode = "exact"

[[dedup.source_priority]]
browser = "brave-nightly"
profile = "Default"
priority = 100

[[dedup.source_priority]]
browser = "librewolf"
priority = 80

[[dedup.source_priority]]
browser = "safari"
priority = 60
```

没有配置时使用稳定但中立的默认规则。

---

## 九、重复项报告

`duplicates.json` 至少包含：

```json
{
  "schema_version": 1,
  "duplicate_groups": [
    {
      "exact_url": "https://example.com/",
      "kept": {
        "browser": "Brave Nightly",
        "profile": "Default",
        "path": ["Bookmarks Bar", "Development"],
        "title": "Example"
      },
      "removed_occurrences": [
        {
          "browser": "Firefox",
          "profile": "default-release",
          "path": ["Bookmarks Menu", "Common"],
          "title": "Example Website"
        }
      ],
      "selection_reason": ["preferred_source", "more_specific_folder_path"]
    }
  ]
}
```

`duplicates.csv` 至少包含：

- group_id
- exact_url
- action
- browser
- profile
- root
- folder_path
- title
- date_added
- date_modified
- original_position
- selection_reason

要求：

- 报告中不能只写“删除了 N 项”；
- 必须能追溯每一项来自哪里；
- 必须能根据报告重新构建原始 occurrence；
- JSON schema 需要版本号。

---

## 十、CLI 重构建议

新增或调整命令：

```text
bsync inventory
bsync collect
bsync dedupe
bsync check
bsync report
bsync pipeline
```

### Inventory

```bash
bsync inventory --all-browsers --all-profiles
```

输出浏览器、Profile、数据文件和书签数量，不创建最终 HTML。

### Collect

```bash
bsync collect \
  --all-browsers \
  --all-profiles \
  --preserve-source-tree \
  --output-dir ./bookmark-run
```

只采集和导出原始结构，不去重。

### Dedupe

```bash
bsync dedupe \
  --input ./bookmark-run/inventory.json \
  --mode exact \
  --output ./bookmark-run/bookmarks-deduplicated.html
```

### Pipeline

```bash
bsync pipeline \
  --all-browsers \
  --all-profiles \
  --preserve-source-tree \
  --dedupe exact \
  --output-dir ./bookmark-run
```

生成：

- 原始 HTML；
- 去重 HTML；
- inventory；
- duplicates JSON/CSV；
- manifest。

### Check

```bash
bsync check \
  --input ./bookmark-run/bookmarks-deduplicated.html \
  --report ./bookmark-run/url-health.json
```

默认只生成报告，不删除。

---

## 十一、URL 有效性验证规划

去重和有效性验证必须分离。

不能在第一次采集时同时删除重复和失效书签。

### 状态分类

不要把所有 4xx/5xx 都判断成永久失效。

建议分类：

- `valid`
- `redirected`
- `authentication_required`
- `forbidden_or_bot_blocked`
- `rate_limited`
- `not_found`
- `gone`
- `server_error`
- `dns_error`
- `tls_error`
- `timeout`
- `connection_error`
- `unsupported_scheme`
- `uncertain`

具体规则：

- 2xx：有效；
- 3xx：有效但记录重定向链；
- 401：需要认证，不是死链；
- 403：可能反爬或登录限制，不自动删除；
- 404：疑似失效；
- 410：强失效证据；
- 429：限流，不是死链；
- 5xx：服务器错误，不自动删除；
- DNS NXDOMAIN：强失效证据，但需要复检；
- Timeout：不确定；
- TLS 错误：单独记录；
- `file:`、`about:`、`javascript:` 等不应按普通 HTTP 处理。

### 请求策略

- 优先 GET 或 HEAD→GET fallback；
- 可配置 User-Agent；
- 限制每域名并发；
- 全局并发限制；
- 指数退避；
- 429 遵守 Retry-After；
- 重定向链上限；
- 可选直连和代理双路径；
- 结果缓存；
- 支持暂停和恢复；
- 支持第二次复检；
- 输出 JSON、CSV 和 HTML 报告。

### 删除策略

本阶段不直接实现对浏览器数据库的删除。

即使保留已有 `--delete`：

- 也只能作用于导出工作集；
- 必须要求显式 `--apply`；
- 必须要求存在先前生成的检查报告；
- 必须校验报告哈希和输入文件哈希；
- 只允许删除经过多轮确认的状态；
- 401、403、429、5xx、timeout 永远不能自动删除。

可以添加可选 Lychee 集成，但内部数据模型和报告格式不应依赖 Lychee。

---

## 十二、分阶段实施

### Phase 0：仓库审计

先阅读全部源码、测试和现有命令。

输出：

- 现有模块图；
- 浏览器适配器列表；
- 当前 Profile 发现机制；
- 当前 Bookmark 类型；
- 当前 deduplicate 实现；
- 当前 merge/flat 实现；
- 当前 check 实现；
- 可复用模块；
- 必须重构模块。

此阶段不修改功能。

### Phase 1：统一数据模型

- 增加 SourceDescriptor；
- 区分 BookmarkOccurrence 和 CanonicalBookmark；
- 建立完整树模型；
- 保留未知元数据；
- 为输出 schema 增加版本。

### Phase 2：全 Profile 发现

- Chromium 全 Profile；
- Firefox 系 profiles.ini；
- 浏览器 channel 区分；
- Profile display name；
- inventory 命令；
- 单元测试和 fixture。

### Phase 3：一致性快照

- 数据文件复制；
- SQLite 一致性快照；
- manifest；
- SHA-256；
- 原子输出；
- 浏览器运行状态处理。

### Phase 4：高保真原始导出

- 保留来源树；
- 保留根节点；
- 保留排序和分隔符；
- 原始 HTML；
- inventory JSON；
- round-trip 测试。

### Phase 5：精确去重

- exact key；
- 确定性 winner；
- duplicates JSON/CSV；
- deduplicated HTML；
- source priority 配置；
- property-based tests。

### Phase 6：有效性验证

- 状态分类；
- 重试；
- 代理与直连；
- 缓存；
- 恢复；
- 报告；
- 安全删除门控。

### Phase 7：兼容与发布

- 保持原有 CLI 能力；
- 对旧参数给出迁移提示；
- README 更新；
- 中英文文档；
- CHANGELOG；
- 示例数据；
- GitHub Actions；
- macOS 构建产物。

---

## 十三、测试矩阵

至少包含：

### 浏览器

- Safari
- Chrome
- Chrome 多 Profile
- Brave Stable
- Brave Nightly
- Brave 多 Profile
- Edge
- Arc
- Firefox
- Firefox 多 Profile
- Firefox Nightly
- LibreWolf
- Waterfox

### 数据情况

- 空书签库；
- 单 Profile；
- 多 Profile；
- 同名 Profile；
- 同名文件夹不同 parent；
- URL 完全重复；
- 标题不同但 URL 相同；
- 相同 URL 位于不同文件夹；
- 相同 URL 位于不同浏览器；
- 相同 URL 位于同一文件夹多次；
- URL 前后有空白；
- fragment 不同；
- query 不同；
- Unicode 域名；
- emoji 文件夹；
- separator；
- malformed URL；
- 非 HTTP scheme；
- 极深目录；
- 10 万级书签；
- 浏览器运行中；
- SQLite WAL 存在；
- Chromium Bookmarks JSON 损坏；
- Safari 权限不足。

### 不变量测试

- 原始 occurrence 数量守恒；
- 原始树导出不丢节点；
- 去重后唯一 URL 数正确；
- `kept + removed == original occurrences`；
- 所有 removed occurrence 都出现在报告中；
- 相同输入生成字节稳定或语义稳定输出；
- dry-run 不产生浏览器数据修改；
- 任意失败不能覆盖已有成功输出。

---

## 十四、验收标准

项目只有满足以下条件才算完成：

1. 能列出所有受支持浏览器的全部 Profile。
2. 每个 Profile 都有来源标识。
3. 原始导出完整保留层级和顺序。
4. 原始导出不执行去重。
5. 精确去重只合并 trim 后完全相同的 URL。
6. 去重结果每个精确 URL 只出现一次。
7. 每个被移除 occurrence 都能在 JSON/CSV 中追溯。
8. 去重结果确定、可重复。
9. 默认不修改任何浏览器数据库。
10. 检查 URL 时不会把 401、403、429、5xx、timeout 自动当成死链。
11. 所有关键功能都有 fixture 和自动化测试。
12. README 的功能声明必须与实际测试一致。
13. 不允许仅实现 happy path。
14. 不允许用“测试未运行”冒充完成。
15. 不允许在没有备份和报告的情况下删除数据。
