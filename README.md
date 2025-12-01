# 🔄 Browser Sync (macOS)

A powerful, macOS-only command-line tool for synchronizing and managing bookmarks, history, and reading lists across multiple browsers. It features a sophisticated **"Base & Merge"** sync strategy, an intelligent **Rule-based Organizer**, and advanced tools for handling cloud sync conflicts.

[中文文档](./README_CN.md)

## ✨ Core Features

- **macOS Native:** Deep integration with browser data files on macOS (SQLite, Plist).
- **Advanced Sync Logic:** Uses a **"Base & Merge"** strategy where the browser with the most organized bookmark structure becomes the template for others.
- **Smart Deduplication:** Automatically resolves duplicate bookmarks by prioritizing entries in deeper folders and those added more recently.
- **Intelligent Organizer:** A powerful, priority-based engine with **90 built-in trilingual (EN/CN/JP) rules** to automatically classify bookmarks into folders. Extensible with custom JSON rules.
- **Firefox Sync Integration:** Offers two strategies (`api` or `local`) to safely sync with Firefox Sync, preventing cloud data from overwriting local changes.
- **Complete Data Management:** Provides a rich set of tools to `sync`, `cleanup`, `organize`, `validate`, `backup`, `restore`, and `schedule` operations.
- **Cloud Reset Wizard:** A guided process (`cloud-reset`) to resolve complex Firefox Sync data conflicts.
- **Cron-based Scheduling:** Run sync operations automatically in the background on a cron schedule.
- **Auto-Close Browsers:** Automatically closes browsers before sync to ensure data safety (graceful quit + force kill).

## 🖥️ Supported Platforms

**This tool is for macOS only.** It directly accesses browser-specific database and property list files.

| Browser | Bookmarks | History | Reading List | Notes |
| :--- | :---: | :---: | :---: | :--- |
| **Waterfox** | ✅ | ✅ | - | Hub browser candidate |
| **Brave Nightly** | ✅ | ✅ | - | Hub browser candidate |
| **Brave** | ✅ | ✅ | - | |
| **Chrome** | ✅ | ✅ | - | |
| **Safari** | ✅ | ✅ | ✅ | |
| **Firefox Nightly**| ✅ | ✅ | - | |

---

## 🔬 How It Works

### The "Base & Merge" Sync Strategy

This tool **does not perform a simple merge**. To ensure a clean and organized result, it uses a "Base & Merge" strategy:

1.  **Analyze:** It reads bookmark data from **all** specified browsers.
2.  **Score:** It scores each browser's folder structure. A higher score is given to browsers with more folders and bookmarks, prioritizing organization.
3.  **Select Base:** The browser with the **highest score** is chosen as the **"base"**. Its structure becomes the canonical source of truth for the sync.
4.  **Merge & Deduplicate:** Bookmarks from other browsers are merged into the base structure. Duplicates are resolved using the smart deduplication logic.
5.  **Overwrite:** The final, merged bookmark set is **written back to all hub browsers**, overwriting their previous bookmark data.

> ⚠️ **IMPORTANT**: This is a one-way street. If a bookmark exists in a lower-scoring browser but not in the "base" browser, **it will be deleted** after the sync. This design choice prioritizes a single, clean structure over retaining scattered bookmarks.

### Smart Deduplication Logic

When duplicate URLs are found, the tool resolves the conflict with two rules:
1.  **Depth Priority:** The bookmark located deeper within a folder structure is kept.
2.  **Recency Priority:** If depths are equal, the bookmark that was added more recently is kept.

### Firefox Sync: Dual Strategies

To prevent conflicts with Mozilla's servers, the tool offers two ways to handle Firefox Sync after a local merge:

-   `--firefox-sync=api`: (Default) The tool acts as a **direct API client**. It authenticates with your Firefox Account and uploads the newly merged bookmark collection to Mozilla's servers, becoming the new source of truth in the cloud.
-   `--firefox-sync=local`: The tool **triggers the browser's own internal sync mechanism**. This is a less direct approach that asks the browser to perform the sync itself.

---

## 🚀 Installation

1.  **Prerequisites:** Ensure you have Rust and Cargo installed.
2.  **Clone and Build:**

    ```bash
    git clone https://github.com/your-username/browser-bookmark-sync.git
    cd browser-bookmark-sync
    cargo build --release
    ```
3.  **Install (Optional):** Copy the executable to a location in your PATH.
    ```bash
    cp target/release/browser-bookmark-sync /usr/local/bin/
    ```

##  कमांड Usage (Commands)

All commands are run via `browser-bookmark-sync <COMMAND>`.

### Core Commands

| Command | Description | Example |
| :--- | :--- | :--- |
| `sync` | The main command. Synchronizes bookmarks, history, and reading lists between hub browsers using the "Base & Merge" strategy. | `browser-bookmark-sync sync` |
| `smart-organize`| **Automatically classifies all bookmarks** using the rule engine. | `browser-bookmark-sync smart-organize` |
| `cleanup` | Removes duplicate bookmarks and/or empty folders without a full sync. | `browser-bookmark-sync cleanup --remove-duplicates` |
| `schedule` | Starts the sync daemon to run tasks on a cron schedule. | `browser-bookmark-sync schedule --cron "0 * * * *"` |
| `validate` | Checks data integrity, looking for duplicates or malformed entries. | `browser-bookmark-sync validate --detailed` |
| `cloud-reset` | Starts a **guided wizard** to resolve Firefox Sync server data issues. | `browser-bookmark-sync cloud-reset` |
| `list` | Lists all detected browsers and their data paths. | `browser-bookmark-sync list` |
| `list-rules` | Displays all 75 built-in classification rules. | `browser-bookmark-sync list-rules` |

### Common Options

-   `--dry-run`: Preview the changes without modifying any files. **Highly recommended for first-time use.**
-   `--browsers "brave,safari"`: Specifies which browsers to include in the operation.
-   `--firefox-sync <api|local>`: (For `sync`) Chooses the Firefox Sync strategy.
-   `-v, --verbose`: Enables detailed logging output.

### Example Workflows

#### First-Time Sync (Preview)
```bash
# See what the sync would do without changing anything.
browser-bookmark-sync sync --dry-run -v
```

#### Daily Sync
```bash
# Sync between the default hub browsers (Waterfox, Brave Nightly).
browser-bookmark-sync sync
```

#### Full Re-organization
```bash
# Use the rule engine to file every bookmark into a category.
browser-bookmark-sync smart-organize --show-stats
```

#### Run Sync Hourly
```bash
# Run the scheduler in the background. (Use a process manager like launchd for persistence)
browser-bookmark-sync schedule --cron "0 * * * *" &
```

---

## 🧠 Rule Engine for Organization

The `smart-organize` command uses a powerful engine to automatically categorize your bookmarks.

-   **Priority-based:** Rules with higher priority are checked first. The first rule to match a bookmark wins.
-   **Multi-faceted Matching:** Rules can match based on a bookmark's URL, domain, path, or title.
-   **Bilingual:** All 75 built-in rules have both English and Chinese folder names.
-   **Extensible:** Provide your own rules via a JSON file.

#### Custom Rules

Create a `my-rules.json` file:
```json
[
  {
    "name": "work-tools",
    "folder_name": "工作工具",
    "folder_name_en": "Work Tools",
    "url_patterns": ["jira", "confluence"],
    "domain_patterns": ["atlassian.com"],
    "priority": 110,
    "description": "Atlassian stack for work."
  }
]
```

And use it:
```bash
# Run the organization with your custom rules taking precedence.
browser-bookmark-sync smart-organize --rules-file my-rules.json
```
---

## ⚠️ Important Notes

1.  **CLOSE YOUR BROWSERS:** Browsers must be fully closed before running any sync or cleanup operations. The tool directly modifies database files that will be overwritten if the browser is open.
2.  **AUTOMATIC BACKUPS:** Before any destructive operation, the tool automatically backs up your browser profiles to `~/Desktop/browser_backup_*`.
3.  **DEFAULT HUBS:** The default "hub" browsers for sync are Waterfox and Brave Nightly. You can change this with the `--browsers` flag.

## 📜 License

This project is licensed under the MIT License.