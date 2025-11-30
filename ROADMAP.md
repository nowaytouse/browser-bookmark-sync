# 浏览器同步工具 - 改进路线图

## 当前状态

### ✅ 已实现功能
- 5 个浏览器支持（Waterfox, Safari, Brave, Brave Nightly, Chrome）
- 书签同步（70 个唯一书签）
- 自动备份
- 智能去重
- 定时同步

### ⚠️ 发现的问题

1. **多配置文件支持不完整**
   - Waterfox 有 2 个配置文件，但只读取了 1 个
   - 其他浏览器也可能有多个配置文件

2. **书签数量不准确**
   - 用户报告 Waterfox 有几千个书签
   - 当前只检测到 78 个（第一个配置文件）
   - 第二个配置文件被锁定（浏览器正在运行）

3. **缺少功能**
   - 阅读列表同步
   - 历史记录同步
   - 增量 vs 完整同步模式

## 🎯 改进计划

### Phase 1: 多配置文件支持（高优先级）

**目标**: 扫描并同步所有浏览器配置文件

**实现**:
```rust
pub struct BrowserProfile {
    pub name: String,
    pub path: PathBuf,
    pub is_default: bool,
}

trait BrowserAdapter {
    fn detect_profiles(&self) -> Result<Vec<BrowserProfile>>;
    fn read_bookmarks_from_all_profiles(&self) -> Result<Vec<Bookmark>>;
    fn write_bookmarks_to_all_profiles(&self, bookmarks: &[Bookmark]) -> Result<()>;
}
```

**影响**:
- Waterfox: 78 → 可能 1000+ 书签
- 其他浏览器: 也会读取所有配置文件

### Phase 2: 数据库锁定处理（高优先级）

**问题**: 浏览器运行时数据库被锁定

**解决方案**:
1. **只读模式**: 使用 SQLite 的只读连接
   ```rust
   Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?
   ```

2. **WAL 模式**: 启用 Write-Ahead Logging
   ```sql
   PRAGMA journal_mode=WAL;
   ```

3. **提示用户**: 建议关闭浏览器以获得最佳效果

### Phase 3: 阅读列表同步（中优先级）

**支持的浏览器**:
- Safari: `~/Library/Safari/ReadingList.plist`
- Firefox/Waterfox: `places.sqlite` 表 `moz_anno_attributes`
- Chrome/Brave: 通过 Reading List API

**数据结构**:
```rust
pub struct ReadingListItem {
    pub id: String,
    pub title: String,
    pub url: String,
    pub date_added: Option<i64>,
    pub preview_text: Option<String>,
}
```

### Phase 4: 历史记录同步（中优先级）

**支持的浏览器**:
- Firefox/Waterfox: `places.sqlite` 表 `moz_historyvisits`
- Chrome/Brave: `History` 文件
- Safari: `History.db`

**数据结构**:
```rust
pub struct HistoryItem {
    pub url: String,
    pub title: Option<String>,
    pub visit_count: i32,
    pub last_visit: Option<i64>,
}
```

**同步模式**:
- **增量模式**: 只同步最近 N 天的历史
- **完整模式**: 同步所有历史（可能很慢）

### Phase 5: 同步模式选择（中优先级）

**增量同步**:
```bash
# 只同步新增的书签（基于时间戳）
browser-bookmark-sync sync --mode incremental --since "7 days ago"
```

**完整同步**:
```bash
# 完全覆盖，确保所有浏览器完全一致
browser-bookmark-sync sync --mode full
```

**智能同步**:
```bash
# 自动检测变化，只同步差异
browser-bookmark-sync sync --mode smart
```

### Phase 6: 更多浏览器支持（低优先级）

**待添加**:
- [ ] Firefox (稳定版)
- [ ] Edge
- [ ] Opera
- [ ] Vivaldi
- [ ] Arc
- [ ] Orion

### Phase 7: 高级功能（低优先级）

**冲突解决**:
- 时间戳优先
- 用户选择
- 合并策略

**数据清理**:
- 删除重复书签
- 删除失效链接
- 整理文件夹结构

**统计报告**:
- 每个浏览器的书签数量
- 重复项数量
- 同步历史

## 🚀 立即可做的改进

### 1. 修复多配置文件读取

**当前代码**:
```rust
fn detect_bookmark_path(&self) -> Result<PathBuf> {
    // 只返回第一个找到的配置文件
    for entry in std::fs::read_dir(&path)? {
        if bookmarks_path.exists() {
            return Ok(bookmarks_path);  // ← 这里就返回了
        }
    }
}
```

**改进后**:
```rust
fn detect_all_profiles(&self) -> Result<Vec<PathBuf>> {
    let mut profiles = Vec::new();
    for entry in std::fs::read_dir(&path)? {
        if bookmarks_path.exists() {
            profiles.push(bookmarks_path);  // ← 收集所有配置文件
        }
    }
    Ok(profiles)
}

fn read_bookmarks(&self) -> Result<Vec<Bookmark>> {
    let mut all_bookmarks = Vec::new();
    for profile in self.detect_all_profiles()? {
        all_bookmarks.extend(read_from_profile(&profile)?);
    }
    Ok(all_bookmarks)
}
```

### 2. 添加只读模式

```rust
use rusqlite::{Connection, OpenFlags};

fn read_firefox_bookmarks(db_path: &Path) -> Result<Vec<Bookmark>> {
    // 使用只读模式，避免锁定问题
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
    )?;
    
    // ... 读取书签
}
```

### 3. 添加详细日志

```bash
# 显示每个配置文件的书签数量
./browser-bookmark-sync sync --verbose

# 输出示例：
# Waterfox Profile 1 (default-release): 78 bookmarks
# Waterfox Profile 2 (ll4fbmm0): 1234 bookmarks
# Total Waterfox: 1312 bookmarks
```

## 📊 预期改进效果

### 书签数量
- **当前**: 70 个唯一书签
- **改进后**: 1000+ 个唯一书签（包含所有配置文件）

### 支持的数据类型
- **当前**: 仅书签
- **改进后**: 书签 + 阅读列表 + 历史记录

### 同步模式
- **当前**: 仅完整同步
- **改进后**: 增量/完整/智能 三种模式

## 🔧 实施建议

### 优先级排序
1. **立即修复**: 多配置文件支持 + 只读模式
2. **短期**: 阅读列表同步
3. **中期**: 历史记录同步 + 同步模式
4. **长期**: 更多浏览器 + 高级功能

### 测试策略
1. 关闭所有浏览器进行测试
2. 使用备份文件测试
3. 先用 `--dry-run` 预览
4. 逐步启用新功能

## 📝 注意事项

### 数据安全
- ✅ 始终先备份
- ✅ 使用事务保证原子性
- ✅ 验证数据完整性
- ⚠️ 浏览器运行时可能无法写入

### 性能考虑
- 大量书签（1000+）可能需要更长时间
- 历史记录可能非常大（10000+ 条）
- 考虑添加进度条
- 考虑并行处理多个浏览器

### 兼容性
- 不同浏览器的数据格式不同
- 某些功能可能不是所有浏览器都支持
- 需要优雅降级

## 🎯 下一步行动

1. **立即**: 实现多配置文件扫描
2. **今天**: 添加只读模式支持
3. **本周**: 实现阅读列表同步
4. **本月**: 完成历史记录同步

---

**最后更新**: 2024-11-30
**状态**: 规划中
