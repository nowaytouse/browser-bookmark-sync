# 收藏夹清理要求状态

本文档是 `.agy/` 要求的可审计状态表。状态只使用 `NOT_STARTED`、`PARTIAL`、`IMPLEMENTED`、`VERIFIED`、`BLOCKED`；“实机运行”与“代码验证”分开记录。

| REQUIREMENT | STATUS | EVIDENCE | REMAINING GAP |
| --- | --- | --- | --- |
| 全部支持的浏览器与全部 profile 采集 | VERIFIED | `src/sync/collector.rs` 的安装与 profile discovery、快照隔离、`InventoryReport`；实机导出保留全部 profile 清单与告警 | 无静默漏扫；无书签/不可读 profile 仍在报告中显式保留 |
| 指定 HTML/Waterfox 数据目录混入 | VERIFIED | `import_bookmarks_from_source_with_report`、`Export --include`；指定 HTML 与 Waterfox profile 已进入实机来源报告 | 无 |
| 精确 URL 去重 | VERIFIED | `src/sync/dedupe.rs`、`normalize_url_for_dedupe`、exact-variant 回归测试 | 无 |
| 收集阶段排除 `.onion` | VERIFIED | collector/export 默认 `exclude_onion=true`，`remove_onion_bookmarks`；实机唯一输入 23,686 项且 `.onion` 为 0 | 无 |
| 保留原始来源与 profile 证据 | VERIFIED | `RunManifest`、`InventoryReport`、snapshot BLAKE3 哈希；实机输入 SHA-256 已留存 | 无 |
| 网站级死亡检测 | IMPLEMENTED | `ProbeScope::Site`、PSL `site_key`、DNS 多 resolver、site state-machine 规则 | 规则 v8 实机复测待执行 |
| 内容/帖文/视频消失检测 | IMPLEMENTED | `ProbeScope::Content`、site preflight 只剔除 `ConfirmedDead` 站点；trusted 站点仍逐 URL 检查；内容报告与独立状态规则 | 需用规则 v8 重新实机复测 |
| 站点与内容分两步执行 | VERIFIED | `check` 在 content scope 先运行 site preflight，再 gate 内容目标 | 无 |
| 当天内快速二次探测与恢复优先 | VERIFIED | same-day cache/retry、`Live`/`Recovered` 成功边界测试；404/410 不再跳过已配置的二次验证器；真实本地 HTTP → 引擎 → SQLite → 隔离清单回归通过 | 实机时间窗口尚未验证 |
| 存疑、临时故障、限流、TLS、SERVFAIL/REFUSED 保留 | VERIFIED | `src/health/state_machine.rs` 与 DNS 分类测试 | 无 |
| 极强死亡证据才进入隔离 | IMPLEMENTED | completed run、actual request、cache 排除、resolver/family 约束；当前 run 必须在合格证据集合中 | 实机候选数量尚未验证 |
| 删除只通过显式 quarantine/apply 副本流程 | IMPLEMENTED | `src/health/quarantine.rs` 的数据库重验证、哈希与原子写入 | 尚未执行真实 apply |
| 详细 JSON/HTML/CSV health 报告 | VERIFIED | `ReportExporter::export_all_for_scope`、reporting tests 与已完成实机报告 | 规则 v8 实机复测待执行 |
| 外部工具失败可诊断且不误判 | VERIFIED | 有界 stdout/stderr、超时终止、结构化解析失败关闭；curl 非零退出不能凭响应头成为强证据；代理端点不再误当成目标 IP | 本地代理回归已通过；真实网站路径仍待复测 |
| DNS 路由策略 | VERIFIED | `dns_policy.json`、AliDNS/Cloudflare 默认策略与安全 IP 过滤 | 实机 resolver 可达性待确认 |
| 热门站点跳过清单 | VERIFIED | `trusted_site_skip_list.json`，版本/哈希写入配置 | 实机跳过数量待确认 |
| 长任务日志与异常终止诊断 | VERIFIED | 默认持久化 `browser-sync.log`、原子 `check-status.json`、每 10 项进度事件与状态回归测试 | 强杀会保留 `running`，结合 PID 缺失与最后日志时间判定中断 |
| 受控负载与长尾加速 | IMPLEMENTED | trusted 站点只跳过站点预检；内容阶段不重复 DNS；主探测与二次验证共用 host/domain 限速队列和冷却；慢速模式同域计划探测至少间隔 2 秒，每轮最多一个二次验证器；本地请求时间戳验证通过 | 规则 v8 实机复测待执行；限速不能保证不触发所有网站风控 |
| 不写回浏览器数据 | VERIFIED | 核心 CLI 仅复制/导出；写回命令隐藏并禁用 | 无 |
| GUI、历史导出、自动整理等非核心功能 | PARTIAL | CLI 明确禁用；GUI 保留为延期范围 | 用户明确延期，当前不纳入核心验收 |
| 多天历史扫描 | VERIFIED | 当前规则仅消费当天 run/retry；不以跨日跨度授权删除 | 无 |

## 2026-09-07：规则 v8 代码验证

- 修复内容预检漏检：仅明确 `ConfirmedDead` 的站点跳过内容检查，未知/受保护站点仍检查具体 URL；trusted 清单不豁免内容。
- 原始 404/410 仍要求三个已完成、非缓存 run 和至少两个验证器家族；本轮不合格时不会借用旧记录确认失效。没有放宽自动删除权限。
- 本地端到端测试覆盖完整 404/410、正文中断、旧强证据后出现不完整复核、成功/403 保留、隔离清单与数据库一致性，并测量普通/慢速模式请求间隔。使用默认内网目标保护和显式本机代理，不访问真实网站。
- 七处旧测试 SQL 改为绑定当前规则版本，避免同轮成功、DNS 分歧等负面测试仅因版本不匹配而空集合通过；规则版本测试改为核验真实持久化与重建结果。
- `cargo test --locked --workspace`：exit 0，383 passed，10 suites；`cargo fmt --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings`、`git diff --check`：均 exit 0。Cargo 仍提示既有的五个二进制命名及两个未使用依赖告警，未修改无关文件。
- `cargo build --locked --release`：exit 0；发布构建完成，保留上述既有 Cargo 告警。
- 本轮未运行真实网站全量扫描、未执行真实 quarantine apply，未改写浏览器 profile 或原始收藏夹。规则 v8 实机验收仍待执行，不能沿用旧报告宣称完成。未登录的私有资源若返回普通 404，仍需人工复核。

## 2026-09-07：限流补强与实机重新启动

- 启动前发现 `Retry-After: 120` 被缩短至 60 秒、过期/零冷却绕过间隔的问题；回归先得到 exit 101（2 failed），修复后 scheduler 三项测试 exit 0。保留服务器完整等待时长，超大值不溢出，短冷却不覆盖长冷却，其他域名不受阻塞。
- native/curl 的 429/503 等待时间进入同一调度器；采用最长等待，慢速无有效头时默认冷却 60 秒。真实本机 HTTP → curl → scheduler 回归通过，不访问互联网。
- `cargo test --locked --workspace`：exit 0，385 passed（10 suites）；`cargo fmt --check`：exit 0；严格 Clippy 首次因新增测试未处理读取长度退出 101，修复后 exit 0；`cargo build --locked --release`、`git diff --check`：exit 0。仍有七项既有 Cargo 告警。
- 全新采集发现 12 个浏览器安装位置、读取 21 个 profile；11 个 profile 为空，没有读取/发现错误。两个指定外部来源均已纳入，其中 HTML 是空书签导出。采集报告保留 `partial` 和空 profile/无 profile 告警，不改成无条件完整成功。
- 排除 onion 后的 281,687 条原始记录 − 258,000 条精确重复 = 23,687 个唯一书签。独立解析导出 HTML 验证重复为 0、onion 为 0；4 个非 HTTP/无效目标保留并由检查计划记录原因。
- 实机已于北京时间 2026-09-07 15:55 启动，PID 25784；网站预检后逐 URL 内容检查，`--limit 0 --slow --concurrency 2 --secondary curl`。使用当时系统已启用的本机 HTTP 代理；代理模式的解析由代理负责，不宣称已经执行直连 DoH 验证。
- 三小时看门狗于 18:55 发出正常中断信号；未完成时保留 SQLite 检查点和原始输入，标记 PARTIAL。每 30 分钟检查一次，不连续轮询，不为赶时限缩短服务器冷却或放宽删除证据。
- 本轮输出在 `/Users/nyamiiko/Downloads/Bookmark/final/run-20260907-v8-CtQH0M/`；固定构建副本、只读检查输入、来源清单、精确去重报告、输入校验和运行清单均独立留存。启动核验时已记录 37 次真实请求（含 30 个 HTTP 200），不只是空进程。尚未完成全量检查或执行 apply，删除数为 0，浏览器 profile 和原始收藏夹未修改。

## 历史验证记录（不代表规则 v8 本轮验收）

- 已运行并通过：`cargo test --workspace`（381 passed）、健康模块定向测试、`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo build --release`、`git diff --check`。
- 已完成实机基线：原始 281,686 项，精确重复 258,000 项，唯一输入 23,686 项，`.onion` 为 0；输入 SHA-256 为 `193f26c127406c299cf7bc9702158c2c5862ea7fecbd295ea76fa7cebecc3937`。
- 新性能/可观测性版本的隔离实机复测会写入 `/Users/nyamiiko/Downloads/Bookmark/final/` 下独立目录，不覆盖上述输入或浏览器 profile。

## 安全边界

不把单次失败、缓存失败、未完成 run、代理/本地网络错误、TLS/超时、429/5xx、SERVFAIL/REFUSED 或内容级 404 当作可直接删除证据；所有不确定项留在原始集合与复核报告中。
