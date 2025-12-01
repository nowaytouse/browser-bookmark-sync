# 🚀 书签同步工具 - 新功能快速参考

## 新增命令

### 📁 场景文件夹同步

同步特定书签文件夹到多个浏览器。

#### 基本用法

```bash
browser-bookmark-sync sync-scenario \
  --scenario-path "文件夹路径" \
  --browsers "浏览器列表"
```

#### 参数说明

- `-p, --scenario-path` - 场景文件夹路径（如 `"工作/项目"`）
- `-b, --browsers` - 目标浏览器，逗号分隔（如 `"chrome,firefox"`）
- `-d, --dry-run` - 预览模式，不实际执行
- `-v, --verbose` - 详细输出

#### 示例

```bash
# 预览工作项目文件夹同步
browser-bookmark-sync sync-scenario \
  -p "工作/项目" \
  -b "chrome,firefox" \
  --dry-run

# 执行同步
browser-bookmark-sync sync-scenario \
  -p "工作/项目" \
  -b "chrome,firefox"

# 同步个人财务文件夹到 Waterfox
browser-bookmark-sync sync-scenario \
  -p "个人/财务" \
  -b "waterfox"
```

---

### 🧹 智能清理

清理重复书签和空文件夹。

#### 基本用法

```bash
browser-bookmark-sync cleanup \
  --remove-duplicates \
  --remove-empty-folders
```

#### 参数说明

- `-b, --browsers` - 目标浏览器（可选，默认所有）
- `--remove-duplicates` - 清理重复书签
- `--remove-empty-folders` - 清理空文件夹
- `-d, --dry-run` - 预览模式
- `-v, --verbose` - 详细输出

#### 示例

```bash
# 预览所有浏览器的清理
browser-bookmark-sync cleanup \
  --remove-duplicates \
  --remove-empty-folders \
  --dry-run

# 仅清理 Chrome 的重复书签
browser-bookmark-sync cleanup \
  -b "chrome" \
  --remove-duplicates

# 清理所有浏览器的空文件夹
browser-bookmark-sync cleanup \
  --remove-empty-folders

# 完整清理（推荐）
browser-bookmark-sync cleanup \
  --remove-duplicates \
  --remove-empty-folders
```

---

## 🔧 常用工作流

### 工作流 1: 场景管理

适用于工作/个人分离管理。

```bash
# 1. 同步工作书签
browser-bookmark-sync sync-scenario \
  -p "工作/项目" \
  -b "chrome,firefox"

# 2. 同步个人书签
browser-bookmark-sync sync-scenario \
  -p "个人" \
  -b "waterfox,brave-nightly"
```

### 工作流 2: 定期维护

每月/每周执行一次。

```bash
# 1. 检查当前状态
browser-bookmark-sync validate --detailed

# 2. 清理重复和空文件夹
browser-bookmark-sync cleanup \
  --remove-duplicates \
  --remove-empty-folders

# 3. 验证结果
browser-bookmark-sync validate
```

### 工作流 3: 书签迁移

从旧浏览器迁移到新浏览器。

```bash
# 1. 完整同步到中枢浏览器
browser-bookmark-sync sync

# 2. 清理重复
browser-bookmark-sync cleanup --remove-duplicates

# 3. 清理空文件夹
browser-bookmark-sync cleanup --remove-empty-folders

# 4. 验证
browser-bookmark-sync validate --detailed
```

---

## ⚠️ 重要提示

### 使用前

1. **关闭所有浏览器** - 避免数据库锁定
2. **使用 dry-run** - 先预览再执行
3. **检查备份** - 自动备份在 `~/Desktop/browser_backup_*`

### 场景路径规则

- 路径**区分大小写**
- 使用 `/` 分隔层级（如 `"工作/项目"`）
- 确保路径在源浏览器中存在

### 清理建议

1. 先执行 `--dry-run` 查看将被删除的内容
2. 从单个浏览器开始测试
3. 验证成功后再扩大范围

---

## 📊 输出说明

### 清理输出示例

```
📊 Waterfox : 41661 bookmarks, 1936 folders
  🔄 Removed 18148 duplicate bookmarks
  🗑️  Removed 515 empty folders
  ✅ Cleanup complete: 23513 bookmarks, 1421 folders remaining
```

### 场景同步输出示例

```
📁 Starting scenario folder synchronization
🎯 Scenario path: 工作/项目
🌐 Target browsers: ["chrome", "firefox"]

📖 Phase 1: Reading scenario folders...
  ✅ Chrome : found folder with 150 bookmarks
  ✅ Firefox : found folder with 145 bookmarks

🔄 Phase 2: Merging...
  📊 Merged folder contains 180 bookmarks

✍️  Phase 4: Updating scenario folders...
  ✅ Chrome : scenario folder updated
  ✅ Firefox : scenario folder updated

✅ Scenario synchronization complete!
```

---

## 🆘 故障排除

### 问题: "Operation not permitted"

**原因**: macOS 权限限制（通常是 Safari）

**解决**: 
1. 系统设置 → 隐私与安全性 → 完全磁盘访问权限
2. 添加终端或 IDE

### 问题: "Database is locked"

**原因**: 浏览器正在运行

**解决**: 关闭所有浏览器后重试

### 问题: "Scenario folder not found"

**原因**: 路径不存在或大小写错误

**解决**: 
1. 验证路径是否存在
2. 检查大小写
3. 使用 `browser-bookmark-sync validate --detailed` 查看书签结构

---

## 🎯 性能参考

基于实测数据：

- **处理速度**: ~41,000 书签 < 0.5 秒
- **去重效率**: 能检测到 ~43% 的重复（18,148/41,661）
- **空文件夹**: ~27% 的文件夹为空（515/1,936）

---

## 📞 获取帮助

```bash
# 查看所有命令
browser-bookmark-sync --help

# 查看特定命令帮助
browser-bookmark-sync sync-scenario --help
browser-bookmark-sync cleanup --help

# 详细验证报告
browser-bookmark-sync validate --detailed
```
