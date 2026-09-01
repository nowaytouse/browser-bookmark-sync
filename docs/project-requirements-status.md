# 收藏夹清理要求状态

本文档是 `.agy/` 要求的可审计状态表。状态只使用 `NOT_STARTED`、`PARTIAL`、`IMPLEMENTED`、`VERIFIED`、`BLOCKED`；“实机运行”与“代码验证”分开记录。

| REQUIREMENT | STATUS | EVIDENCE | REMAINING GAP |
| --- | --- | --- | --- |
| 全部支持的浏览器与全部 profile 采集 | VERIFIED | `src/sync/collector.rs` 的安装与 profile discovery、快照隔离、`InventoryReport`；实机导出保留全部 profile 清单与告警 | 无静默漏扫；无书签/不可读 profile 仍在报告中显式保留 |
| 指定 HTML/Waterfox 数据目录混入 | VERIFIED | `import_bookmarks_from_source_with_report`、`Export --include`；指定 HTML 与 Waterfox profile 已进入实机来源报告 | 无 |
| 精确 URL 去重 | VERIFIED | `src/sync/dedupe.rs`、`normalize_url_for_dedupe`、exact-variant 回归测试 | 无 |
| 收集阶段排除 `.onion` | VERIFIED | collector/export 默认 `exclude_onion=true`，`remove_onion_bookmarks`；实机唯一输入 23,686 项且 `.onion` 为 0 | 无 |
| 保留原始来源与 profile 证据 | VERIFIED | `RunManifest`、`InventoryReport`、snapshot BLAKE3 哈希；实机输入 SHA-256 已留存 | 无 |
| 网站级死亡检测 | IMPLEMENTED | `ProbeScope::Site`、PSL `site_key`、DNS 多 resolver、site state-machine 规则 | 当前实机网络检测进行中 |
| 内容/帖文/视频消失检测 | IMPLEMENTED | `ProbeScope::Content`、site preflight gate、内容报告与独立状态规则 | 当前实机网络检测进行中 |
| 站点与内容分两步执行 | VERIFIED | `check` 在 content scope 先运行 site preflight，再 gate 内容目标 | 无 |
| 当天内快速二次探测与恢复优先 | VERIFIED | same-day cache/retry、`Live`/`Recovered` 成功边界测试 | 实机时间窗口尚未验证 |
| 存疑、临时故障、限流、TLS、SERVFAIL/REFUSED 保留 | VERIFIED | `src/health/state_machine.rs` 与 DNS 分类测试 | 无 |
| 极强死亡证据才进入隔离 | IMPLEMENTED | completed run、actual request、cache 排除、resolver/family 约束 | 实机候选数量尚未验证 |
| 删除只通过显式 quarantine/apply 副本流程 | IMPLEMENTED | `src/health/quarantine.rs` 的数据库重验证、哈希与原子写入 | 尚未执行真实 apply |
| 详细 JSON/HTML/CSV health 报告 | VERIFIED | `ReportExporter::export_all_for_scope`、reporting tests 与已完成实机报告 | 新性能版本复测进行中 |
| 外部工具失败可诊断且不误判 | VERIFIED | 有界 stdout/stderr、超时终止、结构化解析失败关闭 | 依赖工具在目标机上的可用性待实机确认 |
| DNS 路由策略 | VERIFIED | `dns_policy.json`、AliDNS/Cloudflare 默认策略与安全 IP 过滤 | 实机 resolver 可达性待确认 |
| 热门站点跳过清单 | VERIFIED | `trusted_site_skip_list.json`，版本/哈希写入配置 | 实机跳过数量待确认 |
| 长任务日志与异常终止诊断 | VERIFIED | 默认持久化 `browser-sync.log`、原子 `check-status.json`、每 10 项进度事件与状态回归测试 | 强杀会保留 `running`，结合 PID 缺失与最后日志时间判定中断 |
| 受控负载与长尾加速 | IMPLEMENTED | GitHub 等 trusted 站点跳过站点预检；内容直发一次 GET；内容阶段不重复 DNS；每域 200ms 启动间隔、最多 8 并发及 429 冷却 | 新版本实机复测进行中 |
| 不写回浏览器数据 | VERIFIED | 核心 CLI 仅复制/导出；写回命令隐藏并禁用 | 无 |
| GUI、历史导出、自动整理等非核心功能 | PARTIAL | CLI 明确禁用；GUI 保留为延期范围 | 用户明确延期，当前不纳入核心验收 |
| 多天历史扫描 | VERIFIED | 当前规则仅消费当天 run/retry；不以跨日跨度授权删除 | 无 |

## 本轮验证记录

- 已运行并通过：`cargo test --workspace`（381 passed）、健康模块定向测试、`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo build --release`、`git diff --check`。
- 已完成实机基线：原始 281,686 项，精确重复 258,000 项，唯一输入 23,686 项，`.onion` 为 0；输入 SHA-256 为 `193f26c127406c299cf7bc9702158c2c5862ea7fecbd295ea76fa7cebecc3937`。
- 新性能/可观测性版本的隔离实机复测会写入 `/Users/nyamiiko/Downloads/Bookmark/final/` 下独立目录，不覆盖上述输入或浏览器 profile。

## 安全边界

不把单次失败、缓存失败、未完成 run、代理/本地网络错误、TLS/超时、429/5xx、SERVFAIL/REFUSED 或内容级 404 当作可直接删除证据；所有不确定项留在原始集合与复核报告中。
