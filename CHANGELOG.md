# Changelog

## [2024-12-20] v1.2.0

### New Features
- **Quick Temp Processing** (`quick_temp.sh`): One-click script to extract temp folders from all browsers, organize, check dead links, and merge output
- **Merge Results** (`merge_results.sh`): Combine check results into single HTML with categorized folders

### Improvements
- **Folder Structure Preservation**: HTML import now correctly parses `<DT><H3>` folder tags using stack-based parsing
- **Dead Link Detection**: 403/503/429 responses now treated as valid (server online, browser accessible)
- **Reduced Uncertain Rate**: From 87% to 3.7% by optimizing validation logic
- **Accurate Bookmark Count**: Fixed log display using recursive count instead of array length

### Bug Fixes
- Fixed URL checker hanging at 23516/23520 by adding `connect_timeout` and `pool_idle_timeout`
- Fixed manifest path error in `smart_build.sh` for modern_format_boost

### Output Structure
```
📁镜像文件夹
├── ✅ 有效
├── ❌ 无效
├── ❓ 不确定
├── ⏭️ 跳过
└── 👀 临时 (placeholder)
```

---

## 更新日志

## [2024-12-20] v1.2.0

### 新功能
- **快速临时处理** (`quick_temp.sh`): 一键从所有浏览器提取临时文件夹，整理，死链检查，合并输出
- **合并结果** (`merge_results.sh`): 将检查结果合并为单个HTML，按分类文件夹组织

### 改进
- **文件夹结构保留**: HTML导入现在正确解析`<DT><H3>`文件夹标签
- **死链检测优化**: 403/503/429响应视为有效（服务器在线）
- **不确定率降低**: 从87%降至3.7%
- **书签计数准确**: 使用递归计数修复日志显示

### 修复
- 修复URL检查器在23516/23520卡死问题
- 修复smart_build.sh路径错误
