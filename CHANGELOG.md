# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

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

