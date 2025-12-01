# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added - 2025-12-01

#### 🧠 规则引擎大幅扩展（48→75条）
- **新增27条分类规则**: 从48条扩展到75条,全面覆盖常见场景
- **技术类新增** (7条):
  - DevOps & CI/CD (优先级76): Jenkins、GitLab CI、CircleCI等
  - 数据库服务 (优先级74): PostgreSQL、MongoDB、Redis等
  - 区块链加密 (优先级54): 以太坊、比特币、NFT、DeFi等
  - 服务器监控 (优先级58): Grafana、Prometheus、Datadog等
  - API工具 (优先级62): Postman、Swagger、Insomnia等
  - 容器云原生 (优先级66): Docker、Kubernetes、K8s等
  - 开源许可 (优先级33): choosealicense.com、开源协议等

- **生活服务新增** (6条):
  - 旅游出行 (优先级41): Booking、Airbnb、携程等
  - 外卖美食 (优先级39): UberEats、美团、饿了么等
  - 地图导航 (优先级53): Google Maps、高德地图等
  - 健康医疗 (优先级47): WebMD、Mayo Clinic等
  - 天气服务 (优先级31): 天气预报服务
  - 求职招聘 (优先级43): LinkedIn、Indeed、BOSS直聘等

- **媒体内容新增** (5条):
  - 播客节目 (优先级37): Apple Podcasts、Spotify Podcasts等
  - 电子书阅读 (优先级29): Kindle、Goodreads、Z-Library等
  - 漫画在线 (优先级27): Webtoons、漫画站等
  - 摄影图片 (优先级23): 500px、Flickr、摄影平台
  - 体育运动 (优先级21): ESPN、NBA、体育赛事

- **电商细分新增** (3条):
  - 二手交易 (优先级19): eBay、闲鱼、二手市场
  - 团购优惠 (优先级17): Groupon、什么值得买等
  - 价格比较 (优先级13): 价格跟踪、比价平台

- **特殊场景新增** (4条):
  - 短链接服务 (优先级12): bit.ly、短链接服务
  - 本地开发 (优先级11): localhost、本地服务器
  - 翻译服务 (优先级49): Google翻译、DeepL、有道等
  - 字体资源 (优先级25): Google Fonts、字体下载

- **本地化新增** (2条):
  - 日本服务 (优先级51): nicovideo.jp、pixiv.net、DMM等
  - 韩国服务 (优先级51): naver.com、kakao.com等

**目标**: 最大限度减少「未分类」书签,提升自动化分类准确度

**文档更新**:
- 更新 `README_CN.md`: 规则数量48→75条,新增规则表格
- 添加 🆕 标识标记新增规则

### Fixed - 2024-11-30 (Update 4)

#### 🔧 主页整理功能修复
- **问题**: `organize` 命令在每个子文件夹都创建"网站主页"文件夹，而不是统一收集到根目录
- **修复**: 重写 `organize_homepages_recursive` 为 `collect_homepages_recursive`，先递归收集所有主页书签，再统一放到根目录的"网站主页"文件夹
- **效果**: Brave Nightly 成功整理 12,676 个主页书签到根目录

**修复前**: 每个子文件夹可能创建自己的"网站主页"文件夹
**修复后**: 所有主页统一收集到根级别的"网站主页"文件夹

### Added - 2024-11-30 (Update 3)

#### 📁 场景文件夹同步功能
- **新命令**: `sync-scenario` - 同步指定书签文件夹场景到多个浏览器
- **灵活路径**: 支持多层级路径（如 `"工作/项目"`, `"个人/财务"`）
- **智能合并**: 自动合并多个浏览器的同名场景文件夹
- **自动去重**: 场景文件夹内容自动去重
- **创建功能**: 如果目标浏览器不存在该文件夹，自动创建

**使用示例**:
```bash
# 同步工作项目文件夹
browser-bookmark-sync sync-scenario \
  --scenario-path "工作/项目" \
  --browsers "chrome,firefox"

# 预览模式
browser-bookmark-sync sync-scenario \
  -p "个人/财务" \
  -b "waterfox,brave-nightly" \
  --dry-run
```

**技术实现**:
- `find_folder_by_path()` - 路径解析和文件夹查找
- `merge_scenario_folders()` - 智能合并和去重
- `replace_folder_by_path()` - 替换或创建文件夹
- 递归处理，支持任意深度的文件夹层级

#### 🧹 智能清理功能
- **新命令**: `cleanup` - 清理重复书签和空收藏夹文件夹
- **去重选项**: `--remove-duplicates` 删除重复书签
- **空文件夹清理**: `--remove-empty-folders` 删除空文件夹
- **可选目标**: 可指定特定浏览器或全部浏览器
- **统计报告**: 详细报告删除的重复数和空文件夹数

**使用示例**:
```bash
# 完整清理（推荐）
browser-bookmark-sync cleanup \
  --remove-duplicates \
  --remove-empty-folders

# 仅清理特定浏览器
browser-bookmark-sync cleanup \
  --browsers "chrome,firefox" \
  --remove-duplicates

# 预览清理结果
browser-bookmark-sync cleanup \
  --remove-duplicates \
  --remove-empty-folders \
  --dry-run
```

**测试结果**:
```
✅ Waterfox: 41,661 bookmarks → 23,513 bookmarks
   🔄 Removed 18,148 duplicate bookmarks (43.5%)
   🗑️  Removed 515 empty folders (26.6%)

✅ Brave Nightly: 41,333 bookmarks → 23,513 bookmarks
   🔄 Removed 17,820 duplicate bookmarks (43.1%)
   🗑️  Removed 515 empty folders (26.6%)
```

**技术实现**:
- `remove_empty_folders()` - 递归删除空文件夹
- `CleanupStats` - 统计清理结果
- 自底向上清理策略，确保父文件夹的子文件夹先被清理
- 与现有 `deduplicate_bookmarks_global()` 集成

#### 📖 文档增强
- **README_CN.md**: 新增功能说明、命令参考、使用示例
- **QUICK_REFERENCE.md**: 快速参考指南，包含所有场景和故障排除
- **test-production.sh**: 实战测试脚本，包含深度备份和分步验证
- **walkthrough.md**: 完整实现总结和性能分析

#### 🔧 代码质量
- ✅ 零编译警告
- ✅ 零编译错误
- ✅ 完整的 dry-run 支持
- ✅ 详细的日志输出
- ✅ 自动备份机制

#### 性能优化
- **去重性能**: 41,000+ 书签处理 < 0.5 秒
- **空文件夹检测**: O(n) 时间复杂度
- **内存效率**: HashSet 去重，O(1) 查找

### Added - 2024-11-30 (Update 2)

#### 🎉 Safari 历史记录支持
- **新功能**: Safari 历史记录同步完全支持
- **数据量**: 成功读取 6155 条历史记录
- **数据库**: Safari History.db (6.6 MB)
- **时间戳转换**: 正确处理 Safari 的 Core Data 时间戳（从2001-01-01开始）
- **性能**: 全部历史记录读取仅需 0.1 秒

**测试结果**:
```
✅ Safari: 6155 history items (all time)
✅ Safari: 351 history items (7 days)
✅ Waterfox: 6276 history items
📊 Total: 6411 unique history items (merged)
```

#### 🔧 技术实现
- 实现 `read_safari_history()` 函数
- 实现 `write_safari_history()` 函数
- Safari 时间戳转换（2001-01-01 epoch → Unix timestamp）
- SQLite 只读模式访问 History.db
- 支持按天数过滤

**数据库结构**:
- `history_items` 表：URL、访问次数
- `history_visits` 表：访问时间、标题
- JOIN 查询获取完整历史记录

### Added - 2024-11-30 (Update 1)

#### 🎉 历史记录同步功能
- **新命令**: `sync-history` - 同步浏览器历史记录
- **支持浏览器**: Waterfox, Firefox Nightly, Brave, Chrome
- **过滤选项**: `--days` 参数可限制同步最近N天的历史
- **智能去重**: 基于URL哈希的去重机制
- **排序**: 按最后访问时间排序（最新的在前）
- **性能**: SQLite只读模式，避免浏览器锁定问题

**测试结果**:
```
✅ Waterfox Profile 1: 0 history items
✅ Waterfox Profile 2: 396 history items
📊 Total: 396 unique history items (7 days)
```

#### 📚 阅读列表同步功能
- **新命令**: `sync-reading-list` - 同步浏览器阅读列表
- **支持浏览器**: Safari (原生Reading List)
- **智能去重**: 基于URL哈希的去重机制
- **排序**: 按添加时间排序（最新的在前）
- **格式支持**: Safari plist格式解析

#### 🔧 技术改进
- 扩展 `BrowserAdapter` trait，添加历史和阅读列表方法
- 实现 `HistoryItem` 和 `ReadingListItem` 数据结构
- 添加 Chromium 历史数据库读写函数
- 添加 Firefox 历史数据库读写函数
- 添加 Safari 阅读列表 plist 解析函数
- 修复所有编译警告（unused variables）

#### 📖 文档更新
- 更新 `USAGE.md` - 添加历史记录和阅读列表使用指南
- 更新 `PROJECT_SUMMARY.md` - 记录新功能
- 创建 `CHANGELOG.md` - 版本变更记录

### Changed

#### 多配置文件支持增强
- Waterfox 现在读取所有配置文件（之前只读取第一个）
- 书签数量: 78 → 25,040 个（增加 320倍）

#### SQLite 数据库访问优化
- 使用只读模式打开数据库（`SQLITE_OPEN_READ_ONLY`）
- 避免浏览器运行时的锁定问题
- 提高并发访问安全性

### Technical Details

#### 新增数据结构
```rust
pub struct HistoryItem {
    pub url: String,
    pub title: Option<String>,
    pub visit_count: i32,
    pub last_visit: Option<i64>,
}

pub struct ReadingListItem {
    pub url: String,
    pub title: String,
    pub date_added: Option<i64>,
}
```

#### 新增 Trait 方法
```rust
trait BrowserAdapter {
    // 历史记录支持
    fn supports_history(&self) -> bool { false }
    fn read_history(&self, days: Option<i32>) -> Result<Vec<HistoryItem>> { Ok(vec![]) }
    fn write_history(&self, items: &[HistoryItem]) -> Result<()> { Ok(()) }
    
    // 阅读列表支持
    fn supports_reading_list(&self) -> bool { false }
    fn read_reading_list(&self) -> Result<Vec<ReadingListItem>> { Ok(vec![]) }
    fn write_reading_list(&self, items: &[ReadingListItem]) -> Result<()> { Ok(()) }
}
```

#### CLI 命令
```bash
# 历史记录同步
browser-bookmark-sync sync-history [--days <N>] [--dry-run] [--verbose]

# 阅读列表同步
browser-bookmark-sync sync-reading-list [--dry-run] [--verbose]
```

### Performance

- **历史记录读取**: ~5ms per profile (SQLite read-only)
- **去重处理**: O(n) 时间复杂度，使用 HashSet
- **排序**: O(n log n) 时间复杂度
- **内存使用**: 每1000条记录约 ~1MB

### Browser Support Matrix

| 浏览器 | 书签 | 历史记录 | 阅读列表 | 多配置文件 |
|--------|------|----------|----------|------------|
| Safari | ✅ | ✅ (6155条) | ✅ | N/A |
| Brave | ✅ | ✅ | ❌ | ❌ |
| Brave Nightly | ✅ | ✅ | ❌ | ❌ |
| Chrome | ✅ | ✅ | ❌ | ❌ |
| Waterfox | ✅ | ✅ (6276条) | ❌ | ✅ |
| Firefox Nightly | ✅ | ✅ | ❌ | ❌ |

### Known Limitations

1. **Chromium 阅读列表**: 暂不支持（需要额外的 API）
2. **历史记录大小**: 不限制天数时可能非常大（建议使用 `--days` 参数）
3. **并发写入**: 浏览器运行时可能无法写入（建议关闭浏览器后同步）
4. **Safari 书签**: Safari的Bookmarks.plist可能为空（用户未使用Safari书签）

### Future Enhancements

- [ ] Cookies 同步
- [ ] 扩展/插件同步
- [ ] 表单数据同步
- [ ] 密码同步（需要加密）
- [ ] 增量同步模式
- [ ] 冲突解决策略
- [ ] 更多浏览器支持（Firefox, Edge, Opera）

## [0.1.0] - 2024-11-29

### Added
- 初始版本
- 书签同步功能
- 定时同步功能
- 验证功能
- Safari HTML 导入功能

