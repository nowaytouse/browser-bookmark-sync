# Browser Bookmark Sync

一个可靠的跨浏览器书签同步工具，支持 Waterfox、Safari、Brave 和 Firefox Nightly。

## 特性

- ✅ **多浏览器支持**: Waterfox, Safari, Brave, Firefox Nightly
- ✅ **定时同步**: 支持 cron 表达式配置自动同步
- ✅ **三重验证**: 同步前、同步中、同步后完整验证
- ✅ **自动备份**: 每次同步前自动创建备份
- ✅ **智能去重**: 基于 URL 的智能书签去重
- ✅ **安全可靠**: 完整的错误处理和回滚机制

## 安装

### 从源码编译

```bash
cd browser-bookmark-sync
cargo build --release
```

编译后的二进制文件位于 `target/release/browser-bookmark-sync`

## 使用方法

### 一次性同步

```bash
# 执行同步
browser-bookmark-sync sync

# 干运行（预览但不实际修改）
browser-bookmark-sync sync --dry-run

# 详细输出
browser-bookmark-sync sync --verbose
```

### 定时同步

```bash
# 每30分钟同步一次（默认）
browser-bookmark-sync schedule

# 自定义 cron 表达式（每小时）
browser-bookmark-sync schedule --cron "0 0 * * * *"

# 后台运行
browser-bookmark-sync schedule --daemon
```

### 验证书签

```bash
# 快速验证
browser-bookmark-sync validate

# 详细验证报告
browser-bookmark-sync validate --detailed
```

### 列出浏览器

```bash
browser-bookmark-sync list
```

## Cron 表达式示例

```
# 每30分钟
0 */30 * * * *

# 每小时
0 0 * * * *

# 每天凌晨2点
0 0 2 * * *

# 每周一早上9点
0 0 9 * * MON
```

## 支持的浏览器

| 浏览器 | macOS | Windows | Linux | 状态 |
|--------|-------|---------|-------|------|
| Safari | ✅ | ❌ | ❌ | 完整支持 |
| Brave | ✅ | 🚧 | 🚧 | 基础支持 |
| Waterfox | ✅ | 🚧 | 🚧 | 基础支持 |
| Firefox Nightly | ✅ | 🚧 | 🚧 | 基础支持 |

## 工作原理

### 同步流程

```
1. 预同步验证
   ├─ 检测所有浏览器
   └─ 验证书签文件可访问性

2. 读取书签
   ├─ 从所有浏览器读取书签
   └─ 解析为统一格式

3. 合并书签
   ├─ 智能去重（基于URL）
   ├─ 保留文件夹结构
   └─ 按标题排序

4. 创建备份
   └─ 为每个浏览器创建备份文件

5. 写入书签
   └─ 将合并后的书签写入所有浏览器

6. 后同步验证
   ├─ 验证书签完整性
   └─ 确认同步成功
```

### 验证机制

- **预同步验证**: 确保所有浏览器可访问
- **数据验证**: 检查书签结构完整性
- **后同步验证**: 确认同步后数据正确

## 配置

### 环境变量

```bash
# 日志级别
export RUST_LOG=info  # trace, debug, info, warn, error
```

## 故障排查

### 常见问题

**Q: 找不到浏览器**
```bash
# 检查浏览器是否安装
browser-bookmark-sync list
```

**Q: 同步失败**
```bash
# 查看详细日志
RUST_LOG=debug browser-bookmark-sync sync --verbose
```

**Q: 恢复备份**
```bash
# 备份文件位于原书签文件旁边
# Safari: ~/Library/Safari/Bookmarks.plist.backup
# Brave: ~/Library/Application Support/BraveSoftware/Brave-Browser/Default/Bookmarks.backup
```

## 开发

### 运行测试

```bash
cargo test
```

### 代码检查

```bash
cargo clippy
```

### 格式化

```bash
cargo fmt
```

## 安全性

- ✅ 每次同步前自动备份
- ✅ 干运行模式预览更改
- ✅ 完整的错误处理
- ✅ 原子性操作（失败自动回滚）

## 许可证

MIT License

## 贡献

欢迎提交 Issue 和 Pull Request！

## 路线图

- [ ] Windows 平台完整支持
- [ ] Linux 平台完整支持
- [ ] Chrome/Chromium 支持
- [ ] Firefox 稳定版支持
- [ ] 书签冲突解决策略
- [ ] 增量同步优化
- [ ] Web UI 界面
