# 书签清理（bsync）

[English](README.md) · [简体中文](README.zh-Hans.md)

`bsync` 是面向 macOS 的 Rust 工具，用于采集多个浏览器和配置文件中的书签、生成去重副本，并检查网站和页面是否可访问。清理结果写入新 HTML，可在复核后手动导入浏览器。

## 构建与启动

本地构建和 CI 通过 `rust-toolchain.toml` 跟随 Rust nightly。项目要求 Rust 1.100 或更高版本。

```sh
git clone https://github.com/nowaytouse/browser-bookmark-sync.git
cd browser-bookmark-sync
cargo build --locked --release
target/release/browser-bookmark-sync inventory
```

在 macOS 上运行 `bash packaging/macos/build-app.sh`，可在项目根目录生成 `Bookmark Cleaner.app`。构建脚本使用当前 Rust 工具链，应用进行本地临时签名。开发期间也可用下面的命令启动原生窗口。

```sh
cargo run --features native-gui --bin bookmark-review
```

## 使用原生窗口

1. 在欢迎页选择 HTML、自动导入浏览器，或用合成示例试用。导入报告分别记录各浏览器、配置文件和阅读列表的读取情况。
2. 按文件夹、网站或检查状态分组，复核重复书签。相同完整 URL 的后续出现项会标记为移除，可以撤销。
3. 如需检测链接，点 **开始检测**。macOS 上的检测批次由独立进程执行，可关窗后再次打开批次。**刷新进度**仅查看进度，**载入检测结果**保留现有清理决定。
4. 复核保留和移除项后导出新 HTML。程序核对原文件身份和保留条目，拒绝覆盖现有文件。

设置支持跟随系统、英语和简体中文，以及主题、密度、缩放和字体。打开书签详情中的 **检查历史** 可查看当前 URL 的本地记录。历史页仅读取本地数据库。

## 使用命令行

构建后可将可执行文件命名为 `bsync` 并放到 PATH 中。以下命令中的路径可按需替换。

```sh
bsync inventory
bsync export -o ./all-bookmarks.html --all-profiles -d --clean
bsync check --file ./all-bookmarks.html --slow \
  --history-db ./bookmark-health.sqlite --export-dir ./check-results
bsync health quarantine --db ./bookmark-health.sqlite \
  --output ./quarantine-selection.json
```

确认选择文件后，可从原导出 HTML 生成另一份过滤结果。

```sh
bsync health apply-quarantine --db ./bookmark-health.sqlite \
  --selection ./quarantine-selection.json --input ./all-bookmarks.html \
  --output ./bookmarks-cleaned.html --confirm
```

书签采集覆盖可读取本地存储的 Chromium 系、Gecko 系和 Safari。`--reading-list` 可额外导出 Safari 和 Chromium 的阅读列表。无法读取或在采集中变化的数据源会明确报错。

## 检查证据与隐私

- 网站可访问性和完整页面 URL 分开检查。网络暂时失败、受限内容和证据不足的链接保留待复核。
- 单次 404 或 410 不构成清理依据。失效候选需要符合条件的同日实际请求和不同校验方式，并由用户复核。
- 原始浏览器配置、密码、Cookie 和浏览历史不参与写入或导出。清理范围是用户选择的新 HTML 副本。
- 默认提供同日复用、请求节流和可追踪的历史记录。状态文件的完成仅表示该批次流程结束，采集覆盖和删除决定各自核验。

详见 [检查准确性和证据](docs/health-accuracy.md)及 [原生复核界面](docs/native-review.md)。

## 维护欢迎页与工具栏翻译

欢迎页和工具栏读取内嵌的 [英语](assets/locales/en.json)与 [简体中文](assets/locales/zh-Hans.json) JSON。调整文案时同步维护两个文件的同名键。新增键时在[目录结构](src/native_review/entry_copy.rs)添加对应字段，并在两个文件中填入文案。

```sh
cargo test --features native-gui entry_catalogs_have_matching_nonempty_labels
```
