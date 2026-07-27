你正在维护现有 Rust 仓库：

`nowaytouse/browser-bookmark-sync`

不要从零创建新项目。不要另写一次性 Python 脚本。必须在现有 Rust 架构上增量改造，并尽量复用现有浏览器发现、解析、HTML 导出、备份、dry-run、analyze、validate 和 check 逻辑。

## 任务目标

将该项目升级为一个高保真、只读优先、可审计的全浏览器书签采集和清理工具。

需要实现：

1. 自动发现 macOS 上全部受支持浏览器。
2. 自动枚举每个浏览器的全部 Profile，而非只读取默认或当前 Profile。
3. 读取所有 Profile 中的全部书签。
4. 保留：
   - 浏览器名称与 channel；
   - Profile ID 与显示名称；
   - 浏览器根节点；
   - 完整文件夹层级；
   - 节点顺序；
   - 书签顺序；
   - 标题；
   - 原始 URL；
   - 创建与修改时间；
   - 可获得的浏览器原始元数据。
5. 所有读取必须基于一致性快照或副本。
6. 默认绝不修改浏览器原始数据库。
7. 输出不去重的完整原始结构 HTML。
8. 输出全局精确 URL 去重后的 HTML。
9. 去重结果中每个精确 URL 只保留一次。
10. 输出每组重复书签的完整 JSON 和 CSV 审计记录。
11. 记录保留项、被合并项、所有原始路径和选择理由。
12. 为后续联网 URL 有效性检查建立可靠的数据模型和接口。
13. 改进现有 `check`，避免错误地把所有 4xx/5xx 和网络错误当成永久失效。

## 执行方式

先审计，再设计，再分阶段修改。

第一步必须完整阅读：

- `Cargo.toml`
- `README.md`
- `CHANGELOG.md`
- `src/`
- `tests/`
- `examples/`
- GitHub Actions
- 现有 property tests
- 所有 browser adapter
- export、merge、flat、deduplicate、analyze、validate、check、backup 实现

在修改代码之前，先输出：

1. 当前仓库模块图。
2. 当前书签数据模型。
3. 当前支持的浏览器和 Profile 发现能力。
4. 当前 `--merge` 和 `--flat` 的真实行为。
5. 当前 deduplicate key。
6. 当前去重保留项选择逻辑。
7. 当前 `check` 的 HTTP 状态分类。
8. 当前会不会修改浏览器数据。
9. 可以复用的代码。
10. 需要重构的代码。
11. 实施文件列表。
12. 预期兼容性风险。

不要根据 README 猜测实现。必须核对源码。

## 架构要求

引入明确的统一内部模型，至少区分：

- `SourceDescriptor`
- `BrowserInstallation`
- `BrowserProfile`
- `ProfileSnapshot`
- `BookmarkTree`
- `FolderNode`
- `BookmarkOccurrence`
- `CanonicalBookmark`
- `DuplicateGroup`
- `RunManifest`

`BookmarkOccurrence` 表示书签的一次实际出现。

`CanonicalBookmark` 表示精确 URL 去重后，由一个主要 occurrence 和多个重复 occurrence 组成的逻辑书签。

采集阶段不得按 URL 丢弃 occurrence。

所有 occurrence 必须在去重前完整保存。

## 精确去重要求

默认去重键只能是：

```rust
raw_url.trim()
```

以下不得在 exact 模式自动合并：

- HTTP 与 HTTPS；
- 有无 `www`；
- 有无末尾 `/`；
- fragment 不同；
- query 不同；
- tracking parameter 不同；
- 重定向目标相同；
- 标题相似；
- 网页内容相似。

建立确定性的主 occurrence 选择规则。

必须支持可配置 source priority。

没有配置时必须使用稳定顺序，不能依赖 HashMap 随机顺序。

去重后必须生成：

- `bookmarks-original.html`
- `bookmarks-deduplicated.html`
- `inventory.json`
- `duplicates.json`
- `duplicates.csv`
- `run-manifest.json`

## 原始结构要求

完整原始导出顶层按照：

```text
Browser
└── Profile
    └── Browser Root
        └── Original Folder Tree
```

不得默认 flatten。

不得默认合并同名文件夹。

不得默认清理空目录。

不得默认重新分类。

必须保留 separator。

未知节点和未知根类型应保留或产生明确警告，不能静默丢弃。

## Profile 要求

### Chromium 系

枚举 User Data 下所有实际 Profile，结合 `Local State` 获取 Profile 信息。

支持稳定版和可检测到的 Beta、Dev、Canary、Nightly channel。

### Firefox 系

读取 `profiles.ini` 和 `installs.ini`。

支持 Firefox、Developer Edition、Nightly、LibreWolf、Waterfox，以及结构兼容时可配置的其他浏览器。

使用 SQLite 一致性副本或 backup API。

正确处理 WAL。

### Safari

明确记录 Safari 的书签共享模型。

不要假定 Safari Profile 与 Chromium Profile 等价。

权限不足时输出清晰、可操作的错误，不得返回空结果并宣称成功。

## CLI 要求

优先引入：

```text
bsync inventory
bsync collect
bsync dedupe
bsync check
bsync pipeline
```

保持现有命令尽可能兼容。

将现有含混的 merge 语义拆成：

- preserve source tree
- flatten
- merge folders
- deduplicate

默认：

- preserve source tree；
- 不 flatten；
- 不去重，除非用户明确要求；
- 不检查 URL；
- 不修改浏览器。

## URL 检查要求

检查和去重必须是两个独立阶段。

状态至少区分：

- valid
- redirected
- authentication_required
- forbidden_or_bot_blocked
- rate_limited
- not_found
- gone
- server_error
- dns_error
- tls_error
- timeout
- connection_error
- unsupported_scheme
- uncertain

不得将以下状态自动删除：

- 401
- 403
- 429
- 5xx
- timeout
- TLS error
- 单次 DNS error
- 单网络路径失败

支持：

- 并发限制；
- 每域名限制；
- HEAD→GET fallback 或可靠 GET 策略；
- redirect chain；
- Retry-After；
- 指数退避；
- 缓存；
- 暂停恢复；
- 直连与代理；
- 第二轮复检；
- JSON/CSV 报告。

默认只报告，不删除。

如保留 `--delete`，必须加入强安全门控，并且只能操作导出工作集，不能直接编辑浏览器数据库。

## 安全要求

- 所有浏览器数据先 snapshot。
- 记录 SHA-256。
- 输出采用临时文件和原子 rename。
- 每次运行生成独立目录。
- 任一步骤失败都不能覆盖之前成功的输出。
- dry-run 必须真正无副作用。
- 不得读取与书签任务无关的密码、Cookie 或其他敏感数据。
- 不要因为项目原先支持密码/Cookie 导出，就把它们加入本次默认流程。

## 测试要求

必须创建真实结构 fixture，覆盖：

- Chromium 单 Profile、多 Profile；
- Brave Stable、Nightly；
- Firefox 多 Profile；
- LibreWolf；
- Waterfox；
- Safari；
- 相同 URL 跨浏览器；
- 相同 URL 跨 Profile；
- 相同 URL 在同一 Profile 的多个文件夹；
- 同名文件夹不同 parent；
- fragment 和 query 不同；
- Unicode、emoji；
- separator；
- 空文件夹；
- WAL；
- 浏览器运行状态；
- 损坏数据；
- 权限不足；
- 大规模数据。

必须验证不变量：

```text
所有原始 occurrence
=
所有保留 occurrence
+
所有被合并 occurrence
```

同样输入必须得到确定性结果。

添加 property-based tests，特别测试：

- 树遍历不丢节点；
- 去重计数守恒；
- 输出顺序稳定；
- 任意重复分组只保留一个 primary；
- 报告能够覆盖所有 removed occurrence。

## 工作顺序

按以下顺序执行，不要一次性大面积重写：

1. 仓库审计。
2. 编写设计说明。
3. 统一内部数据模型。
4. 实现 inventory 和全 Profile 发现。
5. 实现 snapshot 与 manifest。
6. 实现高保真原始树导出。
7. 实现 exact deduplicate。
8. 实现 duplicates JSON/CSV。
9. 改进 URL check 分类。
10. 添加测试和 fixtures。
11. 更新中英文 README 和 CHANGELOG。
12. 运行 fmt、clippy、全部测试和 release build。

每阶段完成后：

- 展示修改文件；
- 说明设计决定；
- 运行相关测试；
- 报告失败；
- 在继续前修复回归。

## 禁止事项

- 不要从零重写。
- 不要改成 Python。
- 不要用简单的扁平 `HashMap<Url, Bookmark>` 替代树。
- 不要在采集阶段去重。
- 不要静默丢失浏览器/Profile 信息。
- 不要默认 flatten。
- 不要直接修改浏览器书签数据库。
- 不要将 403 或 timeout 视为永久死链。
- 不要声称测试通过，除非确实运行并展示结果。
- 不要为了让测试通过而删除失败测试。
- 不要无关重构密码、Cookie、历史记录功能。
- 不要把整个项目替换为依赖外部 Lychee 的包装器。
- 可以提供可选 Lychee 集成，但核心模型和报告必须由项目自身控制。

## 最终交付

最终回复必须包含：

1. 实际完成的功能。
2. 未完成的功能。
3. 修改文件列表。
4. 新数据模型。
5. CLI 使用示例。
6. 输出目录示例。
7. 去重策略。
8. URL 状态分类。
9. 安全保证。
10. 测试结果。
11. clippy 和 release build 结果。
12. 已知限制。
13. 推荐的下一阶段工作。

先执行仓库审计。未完成审计前不要开始编码。
