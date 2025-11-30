# 📖 Browser Bookmark Sync - Usage Guide

## 🔄 Recommended Workflow

### Step 1: Clean and Sync
First, clean up your bookmarks and sync between browsers:

```bash
# Full sync with automatic cleanup
browser-bookmark-sync sync --browsers "waterfox,brave-nightly" --mode full
```

This will:
- ✅ Remove empty folders (99.9% cleanup rate)
- ✅ Remove folders with "/" or empty names
- ✅ Deduplicate folder structures
- ✅ Deduplicate bookmark URLs
- ✅ Sync between specified browsers

### Step 2: Organize with Rules (Optional)
After cleaning, organize bookmarks using the rule engine:

```bash
# Smart organize using built-in rules
browser-bookmark-sync smart-organize --browsers "waterfox,brave-nightly"

# Or with custom rules
browser-bookmark-sync smart-organize --rules-file my-rules.json

# Only organize uncategorized bookmarks
browser-bookmark-sync smart-organize --uncategorized-only
```

This will:
- ✅ Create category folders (登录入口, 社交媒体, 视频流媒体, etc.)
- ✅ Move bookmarks into appropriate categories
- ✅ Preserve existing folder structure
- ✅ Show classification statistics

---

## 🧹 Cleanup Features

### Automatic Cleanup (During Sync)

The sync command automatically performs these cleanup operations:

#### 1. Empty Folder Removal
- Removes folders with 0 children
- Preserves system folders (menu, tags, unfiled, mobile)
- Recursive cleanup (bottom-up)

#### 2. Invalid Folder Removal
- Removes folders named "/"
- Removes folders with empty names
- Prevents data corruption

#### 3. Folder Structure Deduplication
- Detects duplicate folder hierarchies
- Uses signature matching (name + child count + child names)
- Removes duplicates while preserving first occurrence

#### 4. URL Deduplication
- Global deduplication across entire bookmark tree
- Smart selection (prefers deeper folders, newer bookmarks)
- Preserves folder structure

### Results
Typical cleanup results:
- **Folders:** 6,379 → 947 (85.2% reduction)
- **Empty folders:** 3,923 → 4 (99.9% reduction)
- **Invalid folders:** 916 → 0 (100% removal)
- **Valid bookmarks:** Preserved (no data loss)

---

## 🧠 Rule Engine

### Built-in Categories (18 Rules)

1. **登录入口** (Login) - login., signin., auth.
2. **社交媒体** (Social Media) - facebook.com, twitter.com, instagram.com
3. **视频流媒体** (Video/Streaming) - youtube.com, netflix.com, bilibili.com
4. **开发工具** (Development) - github.com, stackoverflow.com, gitlab.com
5. **购物网站** (Shopping) - amazon.com, taobao.com, jd.com
6. **新闻资讯** (News) - news., bbc.com, cnn.com
7. **文档参考** (Documentation) - docs., documentation, api.
8. **云存储** (Cloud Storage) - drive.google.com, dropbox.com, onedrive.com
9. **邮箱通讯** (Email) - mail., gmail.com, outlook.com
10. **金融理财** (Finance) - bank., paypal.com, alipay.com
11. **AI工具** (AI Tools) - openai.com, claude.ai, chatgpt.com
12. **设计创意** (Design) - figma.com, canva.com, dribbble.com
13. **教育学习** (Education) - coursera.org, udemy.com, edx.org
14. **音乐音频** (Music) - spotify.com, soundcloud.com, music.
15. **游戏娱乐** (Gaming) - steam, game, play.
16. **论坛社区** (Forums) - reddit.com, forum., community.
17. **管理后台** (Admin) - admin., dashboard., console.
18. **API服务** (API) - api., gateway., service.

### Custom Rules

Create a JSON file with your own rules:

```json
[
  {
    "name": "work",
    "folder_name": "工作相关",
    "folder_name_en": "Work",
    "url_patterns": ["company.com", "work."],
    "domain_patterns": [],
    "path_patterns": [],
    "title_patterns": ["工作", "work"],
    "priority": 90,
    "description": "Work-related bookmarks"
  }
]
```

Then use it:
```bash
browser-bookmark-sync smart-organize --rules-file my-rules.json
```

---

## ⚠️ Important Notes

### Execution Order Matters

**Correct Order:**
1. First: `sync` (cleanup + sync)
2. Then: `smart-organize` (categorize)

**Why?**
- Sync removes empty folders and duplicates
- Smart-organize creates new category folders
- If you run smart-organize first, sync will NOT remove the category folders (they have bookmarks)

### Dry-Run Mode

Always test with `--dry-run` first:

```bash
# Test sync
browser-bookmark-sync sync --mode full --dry-run

# Test organize
browser-bookmark-sync smart-organize --dry-run --show-stats
```

### Backups

Automatic backups are created before every operation:
- Firefox/Waterfox: `places.sqlite.backup`
- Chromium browsers: `Bookmarks.json.backup`

To restore:
```bash
# Waterfox
cp places.sqlite.backup places.sqlite

# Brave Nightly
cp Bookmarks.json.backup Bookmarks
```

---

## 📊 Example Workflow

### Complete Cleanup and Organization

```bash
# Step 1: Full sync with cleanup
browser-bookmark-sync sync --browsers "waterfox,brave-nightly" --mode full

# Output:
# 🧹 Phase 1: Cleaning up empty folders...
#    Removed 5435 empty folders
# 🔄 Phase 2: Deduplicating folder structures...
# 🔄 Phase 3: Deduplicating bookmarks by URL...
# ✨ Cleanup complete: removed 5435 items total

# Step 2: Smart organize
browser-bookmark-sync smart-organize --browsers "waterfox,brave-nightly" --show-stats

# Output:
# 📊 Classification Statistics:
#   Total processed: 17,674
#   Classified: 8,234 (46.6%)
#   Unclassified: 9,440 (53.4%)
#   
#   By category:
#     登录入口: 234
#     社交媒体: 156
#     视频流媒体: 89
#     ...

# Step 3: Verify
browser-bookmark-sync validate --detailed
```

---

## 🎯 Common Use Cases

### Use Case 1: Clean Up Messy Bookmarks
```bash
# Just cleanup, no organization
browser-bookmark-sync sync --mode full
```

### Use Case 2: Organize Existing Bookmarks
```bash
# Organize without moving already categorized bookmarks
browser-bookmark-sync smart-organize --uncategorized-only
```

### Use Case 3: Sync Between Two Browsers
```bash
# Sync only between specific browsers
browser-bookmark-sync sync --browsers "waterfox,brave-nightly"
```

### Use Case 4: Custom Organization
```bash
# Use your own rules
browser-bookmark-sync smart-organize --rules-file my-rules.json
```

---

## 🔍 Troubleshooting

### Issue: "No bookmarks organized"
**Solution:** Make sure bookmarks match rule patterns. Use `--show-stats` to see what was classified.

### Issue: "Empty folders still exist"
**Solution:** Run sync again. Some folders may have been created after cleanup.

### Issue: "Bookmarks in wrong categories"
**Solution:** Create custom rules with higher priority to override built-in rules.

### Issue: "Sync too slow"
**Solution:** Use `--mode incremental` for faster syncs after initial full sync.

---

**For more information, see:**
- `README.md` - Feature overview
- `CLEANUP_SUCCESS_REPORT.md` - Cleanup details
- `CHANGELOG_2024-11-30.md` - Recent changes
