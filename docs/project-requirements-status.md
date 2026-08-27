# 收藏夹清理要求状态

本文档是 `.agy/` 要求的可审计状态表。状态只使用 `NOT_STARTED`、`PARTIAL`、`IMPLEMENTED`、`VERIFIED`、`BLOCKED`；“实机运行”与“代码验证”分开记录。

| REQUIREMENT | STATUS | EVIDENCE | REMAINING GAP |
| --- | --- | --- | --- |
| 全部支持的浏览器与全部 profile 采集 | IMPLEMENTED | `src/sync/collector.rs` 的安装与 profile discovery、快照隔离、`InventoryReport` | 当前实机采集进行中 |
| 指定 HTML/Waterfox 数据目录混入 | IMPLEMENTED | `import_bookmarks_from_source_with_report` 与 `Export --include` | 当前实机采集进行中 |
| 精确 URL 去重 | VERIFIED | `src/sync/dedupe.rs`、`normalize_url_for_dedupe`、exact-variant 回归测试 | 无 |
| 收集阶段排除 `.onion` | VERIFIED | collector/export 默认 `exclude_onion=true`，`remove_onion_bookmarks` | 实机零 `.onion` 计数尚未验证 |
| 保留原始来源与 profile 证据 | IMPLEMENTED | `RunManifest`、`InventoryReport`、snapshot BLAKE3 哈希 | 当前实机报告生成中 |
| 网站级死亡检测 | IMPLEMENTED | `ProbeScope::Site`、PSL `site_key`、DNS 多 resolver、site state-machine 规则 | 当前实机网络检测进行中 |
| 内容/帖文/视频消失检测 | IMPLEMENTED | `ProbeScope::Content`、site preflight gate、内容报告与独立状态规则 | 当前实机网络检测进行中 |
| 站点与内容分两步执行 | VERIFIED | `check` 在 content scope 先运行 site preflight，再 gate 内容目标 | 无 |
| 当天内快速二次探测与恢复优先 | VERIFIED | same-day cache/retry、`Live`/`Recovered` 成功边界测试 | 实机时间窗口尚未验证 |
| 存疑、临时故障、限流、TLS、SERVFAIL/REFUSED 保留 | VERIFIED | `src/health/state_machine.rs` 与 DNS 分类测试 | 无 |
| 极强死亡证据才进入隔离 | IMPLEMENTED | completed run、actual request、cache 排除、resolver/family 约束 | 实机候选数量尚未验证 |
| 删除只通过显式 quarantine/apply 副本流程 | IMPLEMENTED | `src/health/quarantine.rs` 的数据库重验证、哈希与原子写入 | 尚未执行真实 apply |
| 详细 JSON/HTML/CSV health 报告 | VERIFIED | `ReportExporter::export_all_for_scope` 与 reporting tests | 当前实机报告生成中 |
| 外部工具失败可诊断且不误判 | VERIFIED | 有界 stdout/stderr、超时终止、结构化解析失败关闭 | 依赖工具在目标机上的可用性待实机确认 |
| DNS 路由策略 | VERIFIED | `dns_policy.json`、AliDNS/Cloudflare 默认策略与安全 IP 过滤 | 实机 resolver 可达性待确认 |
| 热门站点跳过清单 | VERIFIED | `trusted_site_skip_list.json`，版本/哈希写入配置 | 实机跳过数量待确认 |
| 不写回浏览器数据 | VERIFIED | 核心 CLI 仅复制/导出；写回命令隐藏并禁用 | 无 |
| GUI、历史导出、自动整理等非核心功能 | PARTIAL | CLI 明确禁用；GUI 保留为延期范围 | 用户明确延期，当前不纳入核心验收 |
| 多天历史扫描 | VERIFIED | 当前规则仅消费当天 run/retry；不以跨日跨度授权删除 | 无 |

## 本轮验证记录

- 已运行：`cargo test --lib`（363 passed）、`cargo test --bins`（6 passed）、健康模块定向测试、`cargo fmt`、`cargo check --all-targets`。
- 已运行并通过：`cargo test --workspace`（379 passed）、`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo build --release`、`git diff --check`。
- 实机采集/健康检测：`RUNNING`（后台任务 PID 90931；输出目录 `/Users/nyamiiko/Downloads/Bookmark/final`）。完成后再回填总数、去重、`.onion` 与输入指纹证据。

## 安全边界

不把单次失败、缓存失败、未完成 run、代理/本地网络错误、TLS/超时、429/5xx、SERVFAIL/REFUSED 或内容级 404 当作可直接删除证据；所有不确定项留在原始集合与复核报告中。
