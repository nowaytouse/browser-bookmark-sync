//! Native bookmark cleaning workspace. All changes are exported to a new HTML copy.

use crate::health::review::{audit_path_for, Decision, ReviewDocument};
use crate::health::state_machine::StateMachine;
use crate::native_groups::{self, BookmarkGroup, GroupRow};
use crate::native_preferences::{
    AccentChoice, DisplayDensity, DnsStrategy, FontChoice, GroupChoice, LanguageChoice,
    LayoutChoice, NativePreferences, NetworkRoute, NetworkSettings, PreferencesStore, ProxyHost,
    ThemeChoice,
};
use crate::native_scan::{self, ScanHandle, ScanStatus};
use crate::native_sources::{self, BrowserImport, SourceRow, SourceState};
use crate::sync::cancellation::Cancellation;
use chrono::{DateTime, Utc};
use eframe::egui::{self, RichText};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

fn tr(zh: bool, chinese: &'static str, english: &'static str) -> &'static str {
    if zh {
        chinese
    } else {
        english
    }
}

fn group_label(zh: bool, choice: GroupChoice) -> &'static str {
    match choice {
        GroupChoice::Folder => tr(zh, "按文件夹分组", "Group by folder"),
        GroupChoice::Website => tr(zh, "按网站分组", "Group by website"),
        GroupChoice::Status => tr(zh, "按检查状态分组", "Group by check status"),
        GroupChoice::None => tr(zh, "平铺列表", "Flat list"),
    }
}

fn count_verdicts(doc: &ReviewDocument) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::<&str, usize>::new();
    for item in &doc.items {
        *counts.entry(&item.verdict).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(verdict, count)| (verdict.to_owned(), count))
        .collect()
}

fn app_mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let ink = ui.visuals().hyperlink_color;
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(12),
        ui.visuals().selection.bg_fill,
    );
    let at = |x: f32, y: f32| egui::pos2(rect.left() + x * size, rect.top() + y * size);
    for polygon in [
        vec![
            at(0.30, 0.22),
            at(0.70, 0.22),
            at(0.70, 0.62),
            at(0.30, 0.62),
        ],
        vec![at(0.30, 0.62), at(0.50, 0.62), at(0.30, 0.78)],
        vec![at(0.50, 0.62), at(0.70, 0.62), at(0.70, 0.78)],
    ] {
        ui.painter().add(egui::epaint::PathShape::convex_polygon(
            polygon,
            ink,
            egui::Stroke::NONE,
        ));
    }
}

enum Event {
    Loaded(anyhow::Result<ReviewDocument>, bool, Option<ScanHandle>),
    Imported(anyhow::Result<BrowserImport>),
    ScanStarted(anyhow::Result<ScanHandle>),
    ProxyChecked(NetworkSettings, Result<(), String>),
    EvidenceLoaded(anyhow::Result<(ReviewDocument, bool)>),
    Exported(anyhow::Result<(PathBuf, usize, usize)>),
}

struct RepaintOnExit(egui::Context);

impl Drop for RepaintOnExit {
    fn drop(&mut self) {
        self.0.request_repaint();
    }
}

pub fn start(
    input: Option<PathBuf>,
    db: Option<PathBuf>,
    source_review: Option<PathBuf>,
) -> anyhow::Result<()> {
    eframe::run_native(
        "Bookmark Cleaner",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1180.0, 780.0])
                .with_min_inner_size([800.0, 580.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            install_chinese_font(&cc.egui_ctx);
            let mut app = ReviewApp {
                db,
                source_review,
                context: cc.egui_ctx.clone(),
                ..Default::default()
            };
            match PreferencesStore::local() {
                Ok(store) => {
                    let loaded = store.load();
                    app.preferences = loaded.preferences;
                    if let Some(error) = loaded.error {
                        app.message = error.to_string();
                    }
                    app.store = Some(store);
                }
                Err(error) => app.message = error.to_string(),
            }
            apply_style(&cc.egui_ctx, app.preferences);
            if let Some(path) = input {
                app.load(path);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Could not start Bookmark Cleaner: {error}"))
}

fn install_chinese_font(ctx: &egui::Context) {
    #[cfg(target_os = "macos")]
    let paths = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
    ];
    #[cfg(target_os = "windows")]
    let paths = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let paths = [
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ];
    let mut fonts = egui::FontDefinitions::default();
    for path in paths {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("cjk".into(), Arc::new(egui::FontData::from_owned(bytes)));
            // Keep egui's Latin face and use the system face for missing CJK glyphs.
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push("cjk".into());
            }
            break;
        }
    }
    let interface = fonts.families[&egui::FontFamily::Proportional].clone();
    #[cfg(target_os = "macos")]
    let interface = {
        let mut interface = interface;
        if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
            fonts.font_data.insert(
                "system-ui".into(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            interface.insert(0, "system-ui".into());
        }
        interface
    };
    fonts
        .families
        .insert(egui::FontFamily::Name("interface".into()), interface);
    ctx.set_fonts(fonts);
}

fn apply_style(ctx: &egui::Context, preferences: NativePreferences) {
    ctx.set_theme(match preferences.theme {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    });
    ctx.set_zoom_factor(preferences.text_scale);
    ctx.all_styles_mut(|style| {
        let compact = preferences.density == DisplayDensity::Compact;
        style.spacing.item_spacing = egui::vec2(8.0, if compact { 6.0 } else { 8.0 });
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.interact_size.y = if compact { 32.0 } else { 34.0 };
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(22.0));

        let dark = style.visuals.dark_mode;
        let (canvas, surface, subtle, input, text, muted, border, hover) = if dark {
            (
                egui::Color32::from_rgb(24, 29, 35),
                egui::Color32::from_rgb(36, 43, 51),
                egui::Color32::from_rgb(43, 51, 60),
                egui::Color32::from_rgb(29, 35, 42),
                egui::Color32::from_rgb(237, 241, 246),
                egui::Color32::from_rgb(182, 192, 204),
                egui::Color32::from_rgb(48, 56, 66),
                egui::Color32::from_rgb(49, 59, 69),
            )
        } else {
            (
                egui::Color32::from_rgb(242, 244, 247),
                egui::Color32::WHITE,
                egui::Color32::from_rgb(234, 238, 243),
                egui::Color32::WHITE,
                egui::Color32::from_rgb(32, 40, 50),
                egui::Color32::from_rgb(93, 105, 120),
                egui::Color32::from_rgb(215, 221, 229),
                egui::Color32::from_rgb(232, 239, 247),
            )
        };
        let family = match preferences.font {
            FontChoice::System => egui::FontFamily::Name("interface".into()),
            FontChoice::Readable => egui::FontFamily::Proportional,
        };
        for (text_style, font) in &mut style.text_styles {
            if *text_style != egui::TextStyle::Monospace {
                font.family = family.clone();
            }
        }
        let accent = match preferences.accent {
            AccentChoice::Blue => {
                if dark {
                    egui::Color32::from_rgb(117, 175, 255)
                } else {
                    egui::Color32::from_rgb(30, 91, 182)
                }
            }
            AccentChoice::Teal => {
                if dark {
                    egui::Color32::from_rgb(102, 214, 191)
                } else {
                    egui::Color32::from_rgb(0, 105, 92)
                }
            }
            AccentChoice::Violet => {
                if dark {
                    egui::Color32::from_rgb(190, 168, 255)
                } else {
                    egui::Color32::from_rgb(102, 65, 170)
                }
            }
            AccentChoice::Monochrome => {
                if dark {
                    egui::Color32::from_gray(220)
                } else {
                    egui::Color32::from_gray(45)
                }
            }
        };

        let visuals = &mut style.visuals;
        visuals.panel_fill = canvas;
        visuals.window_fill = surface;
        visuals.window_stroke = egui::Stroke::new(1.0, border);
        visuals.window_corner_radius = egui::CornerRadius::same(10);
        visuals.menu_corner_radius = egui::CornerRadius::same(6);
        visuals.override_text_color = Some(text);
        visuals.weak_text_color = Some(muted);
        visuals.faint_bg_color = subtle;
        visuals.extreme_bg_color = input;
        visuals.text_edit_bg_color = Some(input);
        visuals.code_bg_color = subtle;
        visuals.hyperlink_color = accent;
        visuals.selection.bg_fill = accent.gamma_multiply(if dark { 0.28 } else { 0.12 });
        visuals.selection.stroke = egui::Stroke::new(1.0, accent);

        let radius = egui::CornerRadius::same(5);
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = radius;
        }
        visuals.widgets.noninteractive.bg_fill = canvas;
        visuals.widgets.noninteractive.weak_bg_fill = subtle;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border);
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text);
        visuals.widgets.inactive.bg_fill = surface;
        visuals.widgets.inactive.weak_bg_fill = subtle;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, border);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, text);
        visuals.widgets.hovered.bg_fill = hover;
        visuals.widgets.hovered.weak_bg_fill = hover;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, accent);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, text);
        visuals.widgets.active.bg_fill = accent.gamma_multiply(if dark { 0.28 } else { 0.12 });
        visuals.widgets.active.weak_bg_fill = visuals.widgets.active.bg_fill;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, accent);
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, text);
        visuals.widgets.open.bg_fill = subtle;
        visuals.widgets.open.weak_bg_fill = subtle;
        visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, border);
        visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0, text);
    });
}

struct ReviewApp {
    context: egui::Context,
    doc: Option<Arc<ReviewDocument>>,
    verdict_counts: BTreeMap<String, usize>,
    db: Option<PathBuf>,
    source_review: Option<PathBuf>,
    import_directory: Option<tempfile::TempDir>,
    sources: Vec<SourceRow>,
    decisions: BTreeMap<usize, Decision>,
    saved_decisions: BTreeMap<usize, Decision>,
    duplicates: BTreeSet<usize>,
    active_db: bool,
    selected: BTreeSet<usize>,
    focused: Option<usize>,
    history: Vec<Vec<(usize, Option<Decision>)>>,
    search: String,
    verdict_filter: String,
    decision_filter: u8,
    filtered: Vec<usize>,
    groups: Vec<BookmarkGroup>,
    group_rows: Vec<GroupRow>,
    collapsed_groups: BTreeSet<String>,
    grouping: GroupChoice,
    filter_dirty: bool,
    sender: mpsc::Sender<Event>,
    receiver: mpsc::Receiver<Event>,
    worker: Option<std::thread::JoinHandle<()>>,
    pending_result: Option<Event>,
    import_cancellation: Option<Cancellation>,
    busy: bool,
    message: String,
    message_is_error: bool,
    source_details_open: Option<bool>,
    pending_input: Option<PathBuf>,
    pending_browser_import: bool,
    confirm_export: Option<PathBuf>,
    close_after_confirm: bool,
    preferences: NativePreferences,
    store: Option<PreferencesStore>,
    settings: Option<NativePreferences>,
    settings_tab: u8,
    settings_error: Option<String>,
    proxy_test_running: bool,
    proxy_test_result: Option<(NetworkSettings, Result<(), String>)>,
    scan: Option<ScanHandle>,
    scan_status: ScanStatus,
    pending_scan: Option<ScanHandle>,
    scan_verified: bool,
}

impl Default for ReviewApp {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            context: egui::Context::default(),
            doc: None,
            verdict_counts: BTreeMap::new(),
            db: None,
            source_review: None,
            import_directory: None,
            sources: Vec::new(),
            decisions: BTreeMap::new(),
            saved_decisions: BTreeMap::new(),
            duplicates: BTreeSet::new(),
            active_db: false,
            selected: BTreeSet::new(),
            focused: None,
            history: Vec::new(),
            search: String::new(),
            verdict_filter: String::new(),
            decision_filter: 0,
            filtered: Vec::new(),
            groups: Vec::new(),
            group_rows: Vec::new(),
            collapsed_groups: BTreeSet::new(),
            grouping: GroupChoice::default(),
            filter_dirty: true,
            sender,
            receiver,
            worker: None,
            pending_result: None,
            import_cancellation: None,
            busy: false,
            message: String::new(),
            message_is_error: false,
            source_details_open: None,
            pending_input: None,
            pending_browser_import: false,
            confirm_export: None,
            close_after_confirm: false,
            preferences: NativePreferences::default(),
            store: None,
            settings: None,
            settings_tab: 0,
            settings_error: None,
            proxy_test_running: false,
            proxy_test_result: None,
            scan: None,
            scan_status: ScanStatus::default(),
            pending_scan: None,
            scan_verified: false,
        }
    }
}

impl ReviewApp {
    fn zh(&self) -> bool {
        self.preferences.language.prefers_chinese()
    }

    fn open_settings(&mut self) {
        if self.settings.is_none() {
            self.settings = Some(self.preferences);
            self.settings_error = None;
            self.proxy_test_result = None;
        }
    }

    fn load_demo(&mut self) -> anyhow::Result<()> {
        let directory = tempfile::Builder::new()
            .prefix("bookmark-cleaner-demo-")
            .tempdir()?;
        let input = directory.path().join("synthetic-bookmarks.html");
        std::fs::write(
            &input,
            include_bytes!("../tests/fixtures/native-grouped-review.html"),
        )?;
        let doc = ReviewDocument::load(&input, None, None)?;
        self.accept_document(doc, false);
        self.import_directory = Some(directory);
        self.message = tr(self.zh(),"这是合成示例，不是你的浏览器数据。可练习分组、选择、撤销和导出；检测尚未运行。","This is synthetic sample data, not your browser data. Try grouping, selection, Undo and export. No checks have run.").into();
        Ok(())
    }

    fn developer_flags(&self) -> Option<String> {
        self.preferences.developer_mode.then(|| format!("DEV · grouping={:?} · filtered={} · selected={} · flags: --slow --scope content --concurrency 2 --timeout {} --secondary curl --limit 0 {}", self.grouping, self.filtered.len(), self.selected.len(), self.preferences.network.timeout_secs, if self.preferences.network.route == NetworkRoute::Direct { "(direct)" } else { "--proxy (local)" }))
    }

    fn load(&mut self, path: PathBuf) {
        if self.busy {
            return;
        }
        let scan = self.pending_scan.take();
        let db = scan
            .as_ref()
            .map(ScanHandle::db)
            .or_else(|| self.db.clone());
        let source = match &scan {
            Some(handle) => handle.report().is_file().then(|| handle.report()),
            None => self.source_review.clone(),
        };
        self.busy = true;
        self.message_is_error = false;
        self.message = tr(self.zh(), "正在读取书签…", "Reading bookmarks…").into();
        self.spawn_worker(move || {
            let result = ReviewDocument::load(&path, db.as_deref(), source.as_deref());
            Event::Loaded(result, db.is_some(), scan)
        });
    }

    fn request_load(&mut self, path: PathBuf) {
        if self.busy {
            return;
        }
        if self
            .pending_scan
            .as_ref()
            .is_some_and(|handle| handle.input() != path)
        {
            self.pending_scan = None;
        }
        if self.dirty() {
            self.pending_input = Some(path);
        } else {
            self.load(path);
        }
    }

    fn import_browsers(&mut self) {
        if self.busy {
            return;
        }
        self.pending_browser_import = false;
        self.busy = true;
        self.message_is_error = false;
        self.message = tr(
            self.zh(),
            "正在读取浏览器、Profile 和阅读列表…",
            "Reading browsers, profiles and reading lists…",
        )
        .into();
        let cancellation = Cancellation::default();
        self.import_cancellation = Some(cancellation.clone());
        self.spawn_worker(move || {
            Event::Imported(native_sources::collect_browser_copy_cancellable(
                &cancellation,
            ))
        });
    }

    fn cancel_import(&mut self) {
        if let Some(cancellation) = &self.import_cancellation {
            cancellation.cancel();
            self.message = tr(
                self.zh(),
                "正在取消；等待当前读取结束，不再读取后续来源…",
                "Cancelling; waiting for the current read to finish, then stopping further sources…",
            )
            .into();
            self.context.request_repaint();
        }
    }

    fn spawn_worker(&mut self, task: impl FnOnce() -> Event + Send + 'static) {
        let sender = self.sender.clone();
        self.context.request_repaint();
        let context = self.context.clone();
        match std::thread::Builder::new().spawn(move || {
            let _repaint = RepaintOnExit(context);
            let _ = sender.send(task());
        }) {
            Ok(worker) => self.worker = Some(worker),
            Err(error) => {
                self.busy = false;
                self.import_cancellation = None;
                self.message_is_error = true;
                self.message = format!(
                    "{} {error}",
                    tr(
                        self.zh(),
                        "无法启动后台任务：",
                        "Could not start background task:"
                    )
                );
            }
        }
    }

    fn set_document(&mut self, doc: ReviewDocument) {
        self.verdict_counts = count_verdicts(&doc);
        self.doc = Some(Arc::new(doc));
    }

    fn accept_document(&mut self, doc: ReviewDocument, active_db: bool) {
        self.message_is_error = false;
        self.source_details_open = None;
        self.scan = None;
        self.scan_status = ScanStatus::default();
        self.scan_verified = false;
        self.sources = doc
            .coverage
            .as_ref()
            .map(native_sources::rows_from_report)
            .unwrap_or_default();
        self.duplicates = duplicate_ids(&doc);
        self.set_document(doc);
        self.decisions.clear();
        self.saved_decisions.clear();
        self.selected.clear();
        self.history.clear();
        self.focused = None;
        self.active_db = active_db;
        self.search.clear();
        self.verdict_filter.clear();
        self.decision_filter = 0;
        self.grouping = self.preferences.grouping;
        self.collapsed_groups.clear();
        self.filter_dirty = true;
        self.decide(
            self.duplicates.iter().copied().collect(),
            Some(Decision::Remove),
        );
        self.message = tr(self.zh(), "导入完成。精确重复项已标记移除，可撤销；未检查和不确定项保留。", "Imported. Exact duplicates are marked for removal and can be undone. Untested and uncertain links are retained.").into();
    }

    fn poll(&mut self) {
        let finished = self
            .worker
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished);
        // Join only finished workers, before draining their queued result.
        if finished
            && self
                .worker
                .take()
                .is_some_and(|worker| worker.join().is_err())
        {
            self.busy = false;
            self.import_cancellation = None;
            self.pending_result = None;
            self.message_is_error = true;
            self.scan_verified = false;
            self.message = tr(
                self.zh(),
                "后台任务意外中止；当前内容和清理决定未改变。请先检查输出文件或检测批次状态，再重试。",
                "The background task ended unexpectedly; current content and decisions are unchanged. Check output files or the check batch before retrying.",
            )
            .into();
        }
        let mut pending = finished.then(|| self.pending_result.take()).flatten();
        while let Some(event) = pending.take().or_else(|| self.receiver.try_recv().ok()) {
            if let Event::ProxyChecked(network, result) = event {
                self.proxy_test_running = false;
                self.proxy_test_result = Some((network, result));
                continue;
            }
            // A queued result is not completion until the worker has exited.
            if self.worker.is_some() {
                self.pending_result = Some(event);
                continue;
            }
            self.busy = false;
            self.message_is_error = false;
            if matches!(&event, Event::Imported(_))
                && self
                    .import_cancellation
                    .take()
                    .is_some_and(|cancellation| cancellation.is_cancelled())
            {
                self.message = tr(
                    self.zh(),
                    "已取消浏览器导入；当前内容和清理决定未改变。",
                    "Browser import cancelled; current content and decisions are unchanged.",
                )
                .into();
                continue;
            }
            match event {
                Event::ScanStarted(Ok(handle)) => {
                    self.message = tr(self.zh(), "检测已独立启动。可关闭窗口，之后打开批次载入结果。", "Check started independently. You can close this window and reopen the batch to load results.").into();
                    self.scan = Some(handle);
                    self.refresh_scan();
                }
                Event::ScanStarted(Err(error)) => {
                    self.message_is_error = true;
                    self.message = format!(
                        "{} {error:#}",
                        tr(self.zh(), "检测未启动：", "Check did not start:")
                    )
                }
                Event::ProxyChecked(_, _) => unreachable!("proxy results were handled above"),
                Event::EvidenceLoaded(Ok((mut doc, verified))) => {
                    if self
                        .doc
                        .as_ref()
                        .is_some_and(|old| old.input_hash == doc.input_hash)
                    {
                        doc.coverage = self.doc.as_ref().and_then(|old| old.coverage.clone());
                        self.set_document(doc);
                        self.active_db = true;
                        self.scan_verified = verified;
                        self.filter_dirty = true;
                        self.message = if verified { tr(self.zh(), "网络检测覆盖已验收，当前清理决定已保留。", "Network target coverage verified; current decisions were preserved.") } else { tr(self.zh(), "已载入当前证据；全量检测尚未通过验收，未检查及不确定项保留。", "Current evidence loaded. Full check coverage is not verified; untested and uncertain links are retained.") }.into();
                    } else {
                        self.message = tr(
                            self.zh(),
                            "输入已变化，未载入检测证据。",
                            "Input changed; evidence was not loaded.",
                        )
                        .into();
                    }
                }
                Event::EvidenceLoaded(Err(error)) => {
                    self.message_is_error = true;
                    self.scan_verified = false;
                    self.message = format!("{error:#}");
                }
                Event::Loaded(Ok(doc), active_db, scan) => {
                    self.accept_document(doc, active_db);
                    self.import_directory = None;
                    self.scan = scan;
                    self.refresh_scan();
                }
                Event::Imported(Ok(import)) => {
                    self.accept_document(import.document, false);
                    self.sources = import.sources;
                    self.import_directory = Some(import.directory);
                    let limitations = source_limitation_count(&self.sources);
                    if limitations > 0 {
                        self.message = format!(
                            "{} {limitations} {}",
                            tr(
                                self.zh(),
                                "已导入可读取的浏览器内容，但有",
                                "Readable browser content imported with"
                            ),
                            tr(
                                self.zh(),
                                "个来源存在读取问题或覆盖限制；请查看来源明细。",
                                "source read issues or coverage limitations; see source details."
                            )
                        );
                    }
                    self.source_details_open = self
                        .sources
                        .iter()
                        .any(|row| {
                            matches!(
                                row.state,
                                SourceState::Failed
                                    | SourceState::PermissionDenied
                                    | SourceState::Unverified
                            )
                        })
                        .then_some(true);
                }
                Event::Imported(Err(error)) => {
                    self.message_is_error = true;
                    let summary = if self.doc.is_some() {
                        tr(self.zh(), "浏览器导入失败；原有内容和清理决定未改变。", "Browser import failed; the previous document and decisions are unchanged.")
                    } else {
                        tr(
                            self.zh(),
                            "浏览器导入失败；没有导入新内容。",
                            "Browser import failed; no new content was imported.",
                        )
                    };
                    self.message = format!("{summary}\n{error:#}");
                }
                Event::Loaded(Err(error), _, _) => {
                    self.message_is_error = true;
                    self.message = format!(
                        "{} {error:#}",
                        tr(
                            self.zh(),
                            "导入失败；原有内容已保留。",
                            "Import failed; the previous document is unchanged."
                        )
                    );
                }
                Event::Exported(Ok((path, retained, removed))) => {
                    self.saved_decisions = self.decisions.clone();
                    self.message = format!(
                        "{} {retained} · {} {removed} · {}",
                        tr(self.zh(), "已导出，保留", "Exported; retained"),
                        tr(self.zh(), "移除", "removed"),
                        path.display()
                    );
                }
                Event::Exported(Err(error)) => {
                    self.message_is_error = true;
                    self.message = format!(
                        "{} {error:#}",
                        tr(self.zh(), "导出失败：", "Export failed:")
                    )
                }
            }
        }
    }

    fn rebuild_filter(&mut self) {
        if !self.filter_dirty {
            return;
        }
        self.filtered.clear();
        if let Some(doc) = &self.doc {
            let query = self.search.trim().to_lowercase();
            for item in &doc.items {
                if self.verdict_filter == "duplicates" {
                    if !self.duplicates.contains(&item.id) {
                        continue;
                    }
                } else if !self.verdict_filter.is_empty() && item.verdict != self.verdict_filter {
                    continue;
                }
                let decision = self.decisions.get(&item.id);
                let matches_decision = match self.decision_filter {
                    1 => decision.is_none(),
                    2 => decision == Some(&Decision::Keep),
                    3 => decision == Some(&Decision::Remove),
                    _ => true,
                };
                if !matches_decision {
                    continue;
                }
                if !query.is_empty()
                    && ![
                        item.title.as_str(),
                        item.url.as_str(),
                        &item.path.join(" / "),
                    ]
                    .iter()
                    .any(|s| s.to_lowercase().contains(&query))
                {
                    continue;
                }
                self.filtered.push(item.id);
            }
        }
        self.groups = self
            .doc
            .as_ref()
            .map(|doc| native_groups::groups(&doc.items, &self.filtered, self.grouping))
            .unwrap_or_default();
        self.group_rows = native_groups::rows(&self.groups, &self.collapsed_groups, self.grouping);
        self.filter_dirty = false;
    }

    fn decide(&mut self, ids: Vec<usize>, decision: Option<Decision>) {
        let prior: Vec<_> = ids
            .into_iter()
            .filter_map(|id| {
                let previous = self.decisions.get(&id).copied();
                (previous != decision).then_some((id, previous))
            })
            .collect();
        if prior.is_empty() {
            return;
        }
        for (id, _) in &prior {
            if let Some(value) = decision {
                self.decisions.insert(*id, value);
            } else {
                self.decisions.remove(id);
            }
        }
        self.history.push(prior);
        self.filter_dirty = true;
    }

    fn undo(&mut self) {
        if let Some(prior) = self.history.pop() {
            for (id, old) in prior {
                if let Some(value) = old {
                    self.decisions.insert(id, value);
                } else {
                    self.decisions.remove(&id);
                }
            }
            self.filter_dirty = true;
        }
    }

    fn dirty(&self) -> bool {
        self.decisions != self.saved_decisions
    }
    fn removed(&self) -> usize {
        self.decisions
            .values()
            .filter(|&&d| d == Decision::Remove)
            .count()
    }

    fn choose_export(&mut self) {
        if self.doc.is_some() && !self.busy {
            self.confirm_export = rfd::FileDialog::new()
                .add_filter("HTML", &["html", "htm"])
                .set_file_name("bookmarks-cleaned.html")
                .save_file();
        }
    }

    fn start_scan(&mut self) {
        if self.busy || self.scan.is_some() {
            return;
        }
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let network = self.preferences.network;
        self.busy = true;
        self.message_is_error = false;
        self.message = tr(
            self.zh(),
            "正在准备独立检测…",
            "Preparing an independent check…",
        )
        .into();
        self.spawn_worker(move || {
            Event::ScanStarted(native_scan::begin(
                &doc.input_path,
                &doc.input_hash,
                doc.coverage.as_ref(),
                network,
            ))
        });
    }

    fn refresh_scan(&mut self) {
        if let Some(handle) = &self.scan {
            match handle.status() {
                Ok(status) => {
                    self.scan_status = status;
                    if self.message_is_error {
                        self.message_is_error = false;
                        self.message = tr(
                            self.zh(),
                            "进度已刷新；覆盖验收需载入检测结果后重新核验。",
                            "Progress refreshed; reload results to verify target coverage.",
                        )
                        .into();
                    }
                }
                Err(error) => {
                    self.message_is_error = true;
                    self.scan_verified = false;
                    self.message = format!("{error:#}");
                }
            }
        }
    }

    fn load_scan_evidence(&mut self) {
        if self.busy {
            return;
        }
        let (Some(handle), Some(doc)) = (self.scan.clone(), self.doc.clone()) else {
            return;
        };
        self.busy = true;
        self.spawn_worker(move || {
            let result = ReviewDocument::load(&doc.input_path, Some(&handle.db()), None)
                .map(|doc| (doc, handle.verify_complete().is_ok()));
            Event::EvidenceLoaded(result)
        });
    }

    fn open_batch(&mut self) {
        if self.busy {
            return;
        }
        if let Some(directory) = rfd::FileDialog::new().pick_folder() {
            let handle = ScanHandle { directory };
            if handle.input().is_file() && handle.db().is_file() {
                self.pending_scan = Some(handle.clone());
                self.request_load(handle.input());
            } else {
                self.message = tr(
                    self.zh(),
                    "所选目录不是检测批次。",
                    "The selected folder is not a check batch.",
                )
                .into();
            }
        }
    }

    fn export(&mut self, path: PathBuf) {
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let decisions = self.decisions.clone();
        self.busy = true;
        self.message_is_error = false;
        self.message = tr(
            self.zh(),
            "正在导出新的 HTML…",
            "Exporting a new HTML file…",
        )
        .into();
        self.spawn_worker(move || {
            let result = doc
                .export_copy(&decisions, &path, true)
                .map(|audit| (path, audit.retained_count, audit.removed_count));
            Event::Exported(result)
        });
    }
}

impl eframe::App for ReviewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
        if self.settings.is_some() {
            ui.disable();
        }
        self.poll();
        if self.busy || self.proxy_test_running {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if ctx.input(|i| i.viewport().close_requested()) && (self.busy || self.dirty()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.busy {
                self.close_after_confirm = true;
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            self.open_settings();
        }
        let has_modal = self.pending_input.is_some()
            || self.pending_browser_import
            || self.confirm_export.is_some()
            || self.close_after_confirm;
        if self.settings.is_none() && !self.busy && !has_modal {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
                if let Some(path) = pick_html() {
                    self.request_load(path);
                }
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
                self.choose_export();
            }
            if !ctx.egui_wants_keyboard_input()
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z))
            {
                self.undo();
            }
            let dropped =
                ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
            if let Some(path) = dropped {
                self.request_load(path);
            }
        }
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::default()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(8),
            )
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::default()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(8),
            )
            .show(ui, |ui| {
                if self.message_is_error {
                    egui::ScrollArea::vertical()
                        .id_salt("import-error-details")
                        .max_height(140.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&self.message)
                                        .small()
                                        .color(ui.visuals().warn_fg_color),
                                )
                                .wrap(),
                            );
                        });
                    return;
                }
                ui.horizontal(|ui| {
                    if self.busy {
                        ui.spinner();
                    }
                    if let Some(cancellation) = &self.import_cancellation {
                        if ui
                            .add_enabled(
                                !cancellation.is_cancelled(),
                                egui::Button::new(tr(self.zh(), "取消导入", "Cancel import")),
                            )
                            .clicked()
                        {
                            self.cancel_import();
                        }
                    }
                    ui.add(egui::Label::new(RichText::new(&self.message).small()).truncate())
                        .on_hover_text(&self.message);
                });
                if let Some(flags) = self.developer_flags() {
                    ui.small(flags);
                }
            });
        self.rebuild_filter();
        let narrow = ui.available_width() < 880.0;
        if self.doc.is_some() {
            egui::Panel::top("workflow")
                .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(16, 4)))
                .show(ui, |ui| {
                    centered_content(ui, 1040.0 / self.preferences.text_scale, false, |ui| {
                        self.summary(ui)
                    })
                });
            if self.preferences.show_details && self.focused.is_some() && !narrow {
                egui::Panel::right("inspector")
                    .exact_size(
                        (ui.available_width() * self.preferences.layout.inspector_fraction())
                            .clamp(260.0, 460.0),
                    )
                    .min_size(260.0)
                    .max_size(460.0)
                    .resizable(false)
                    .frame(egui::Frame::default().inner_margin(18))
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!self.busy, |ui| self.details(ui));
                    });
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::default().inner_margin(16))
                .show(ui, |ui| {
                    centered_content(ui, 1040.0 / self.preferences.text_scale, true, |ui| {
                        ui.add_enabled_ui(!self.busy, |ui| self.list(ui));
                    });
                });
        } else {
            egui::CentralPanel::default().show(ui, |ui| self.welcome(ui));
        }
        if narrow && self.preferences.show_details && self.focused.is_some() {
            let mut open = true;
            egui::Window::new(tr(self.zh(), "详情与证据", "Details & evidence"))
                .id(egui::Id::unique("compact-inspector"))
                .open(&mut open)
                .default_width(390.0)
                .default_height(380.0)
                .show(&ctx, |ui| {
                    ui.add_enabled_ui(!self.busy, |ui| self.details(ui));
                });
            if !open {
                self.focused = None;
            }
        }
        // Modals are created after the main panels so their layers stay on top.
        self.settings_window(&ctx);
        self.modal(&ctx);
    }
}

impl ReviewApp {
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        ui.horizontal(|ui| {
            app_mark(ui, 20.0);
            ui.label(
                RichText::new(tr(zh, "书签清理", "Bookmark Cleaner"))
                    .size(18.0)
                    .strong(),
            );
            if let Some(doc) = &self.doc {
                ui.separator();
                let filename_width =
                    (ui.available_width() - if zh { 260.0 } else { 320.0 }).clamp(30.0, 280.0);
                ui.add_sized(
                    [filename_width, ui.spacing().interact_size.y],
                    egui::Label::new(
                        RichText::new(
                            doc.input_path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                        )
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    )
                    .truncate(),
                )
                .on_hover_text(doc.input_path.display().to_string());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.doc.is_some() && !self.busy,
                        egui::Button::new(
                            RichText::new(tr(zh, "导出 HTML…", "Export HTML…"))
                                .strong()
                                .color(egui::Color32::WHITE),
                        )
                        .fill(action_fill(self.preferences.accent))
                        .stroke(egui::Stroke::NONE),
                    )
                    .clicked()
                {
                    self.choose_export();
                }
                if ui
                    .button(tr(zh, "设置", "Settings"))
                    .on_hover_text("⌘ ,")
                    .clicked()
                {
                    self.open_settings();
                }
                if self.doc.is_some() {
                    ui.add_enabled_ui(!self.busy, |ui| {
                        ui.menu_button(tr(zh, "导入", "Import"), |ui| {
                            if ui.button(tr(zh, "HTML 文件…", "HTML file…")).clicked() {
                                ui.close();
                                if let Some(path) = pick_html() {
                                    self.request_load(path);
                                }
                            }
                            if ui
                                .button(tr(zh, "自动导入浏览器", "Import from browsers"))
                                .clicked()
                            {
                                ui.close();
                                if self.dirty() {
                                    self.pending_browser_import = true;
                                } else {
                                    self.import_browsers();
                                }
                            }
                            if ui
                                .button(tr(zh, "打开检测批次…", "Open check batch…"))
                                .clicked()
                            {
                                ui.close();
                                self.open_batch();
                            }
                        });
                    });
                }
            });
        });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        egui::ScrollArea::vertical()
            .id_salt("welcome-guide")
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.set_max_width(760.0);
                    ui.add_space((ui.available_height() * 0.12).clamp(20.0, 64.0));
                    app_mark(ui, 44.0);
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(tr(zh, "给收藏夹留一点空间。", "Make room for the links that matter."))
                            .size(28.0)
                            .strong(),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(tr(
                            zh,
                            "去重、复核失效链接，然后导出一份新的整合书签。",
                            "Deduplicate, review dead links, and export a clean copy.",
                        ))
                        .size(15.0)
                        .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(24.0);
                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::symmetric(20, 16))
                        .show(ui, |ui| {
                            ui.with_layout(
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.label(
                                        RichText::new(tr(
                                            zh,
                                            "选择书签来源",
                                            "Choose a bookmark source",
                                        ))
                                        .strong(),
                                    );
                                    ui.add_space(12.0);
                                    let action_width =
                                        180.0 + 205.0 + ui.spacing().item_spacing.x;
                                    ui.horizontal_wrapped(|ui| {
                                        ui.add_space(
                                            (ui.available_width() - action_width).max(0.0) * 0.5,
                                        );
                                        if ui
                                            .add_enabled(
                                                !self.busy,
                                                egui::Button::new(
                                                    RichText::new(tr(
                                                        zh,
                                                        "导入 HTML…",
                                                        "Import HTML…",
                                                    ))
                                                    .strong()
                                                    .color(egui::Color32::WHITE),
                                                )
                                                .fill(action_fill(self.preferences.accent))
                                                .stroke(egui::Stroke::NONE)
                                                .min_size(egui::vec2(180.0, 42.0)),
                                            )
                                            .clicked()
                                        {
                                            if let Some(path) = pick_html() {
                                                self.request_load(path);
                                            }
                                        }
                                        if ui
                                            .add_enabled(
                                                !self.busy,
                                                egui::Button::new(tr(
                                                    zh,
                                                    "自动导入浏览器",
                                                    "Import from browsers",
                                                ))
                                                .stroke(egui::Stroke::new(
                                                    1.0,
                                                    ui.visuals().hyperlink_color,
                                                ))
                                                .min_size(egui::vec2(205.0, 42.0)),
                                            )
                                            .clicked()
                                        {
                                            self.import_browsers();
                                        }
                                    });
                                    ui.add_space(10.0);
                                    ui.small(tr(
                                        zh,
                                        "也可以把 HTML 文件拖到此窗口。",
                                        "You can also drop an HTML file into this window.",
                                    ));
                                },
                            );
                        });
                    ui.add_space(14.0);
                    ui.small(tr(
                        zh,
                        "自动导入会逐项显示浏览器、Profile 和阅读列表的读取结果。",
                        "Browser import reports the result for each browser, profile and reading list.",
                    ));
                    ui.small(tr(
                        zh,
                        "清理只影响导出的新文件；未确认失效的链接会保留。",
                        "Cleaning affects only the exported copy; uncertain links are kept.",
                    ));
                    ui.add_space(14.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .small_button(tr(zh, "用示例书签试一试", "Try sample bookmarks"))
                            .on_hover_text(tr(
                                zh,
                                "只使用合成数据，不读取浏览器或真实书签。",
                                "Uses synthetic data only; no browser or real bookmarks are read.",
                            ))
                            .clicked()
                        {
                            if let Err(error) = self.load_demo() {
                                self.message = error.to_string();
                                self.message_is_error = true;
                            }
                        }
                        if ui
                            .small_button(tr(zh, "打开检测批次…", "Open check batch…"))
                            .clicked()
                        {
                            self.open_batch();
                        }
                    });
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(tr(
                            zh,
                            "导入  →  去重与复核  →  导出",
                            "Import  →  Deduplicate & review  →  Export",
                        ))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                });
            });
    }

    fn summary(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let untested = self.verdict_counts.get("untested").copied().unwrap_or(0);
        let dead = self
            .verdict_counts
            .get("confirmed_dead")
            .copied()
            .unwrap_or(0);
        let removed = self.removed();
        let retained = doc.items.len().saturating_sub(removed);
        egui::Frame::default()
            .inner_margin(egui::Margin::symmetric(0, 4))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    for (label, value, primary) in [
                        (tr(zh, "导出将保留", "To retain"), retained, true),
                        (tr(zh, "已导入", "Imported"), doc.items.len(), false),
                        (
                            tr(zh, "精确重复", "Exact duplicates"),
                            self.duplicates.len(),
                            false,
                        ),
                        (tr(zh, "确认失效", "Confirmed dead"), dead, false),
                    ] {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(
                                RichText::new(value.to_string())
                                    .size(if primary { 24.0 } else { 16.0 })
                                    .strong()
                                    .color(if primary {
                                        ui.visuals().hyperlink_color
                                    } else {
                                        ui.visuals().text_color()
                                    }),
                            )
                            .on_hover_text(format!("{label}: {value}"));
                            if primary {
                                ui.label(label);
                            } else {
                                ui.weak(label);
                            }
                        });
                    }
                    if self.scan.is_none()
                        && ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::new(tr(zh, "开始检测", "Start check")),
                            )
                            .on_hover_text(tr(
                                zh,
                                "检测在后台运行；未检查的链接仍会保留。",
                                "Checks run in the background; untested links stay in the export.",
                            ))
                            .clicked()
                    {
                        self.start_scan();
                    }
                    if self.scan.is_none() && self.sources.is_empty() {
                        if untested > 0 {
                            ui.small(format!("{untested} {}", tr(zh, "尚未检查", "untested")))
                                .on_hover_text(tr(
                                    zh,
                                    "未检查的链接会保留在导出文件中。",
                                    "Untested links stay in the export.",
                                ));
                        }
                        ui.weak(tr(zh, "未附带来源报告", "No source report"));
                    }
                });
                if self.scan.is_some() {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::new(tr(zh, "刷新进度", "Refresh progress")),
                            )
                            .clicked()
                        {
                            self.refresh_scan();
                        }
                        if ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::new(tr(zh, "载入检测结果", "Load check results")),
                            )
                            .clicked()
                        {
                            self.load_scan_evidence();
                        }
                        if ui
                            .small_button(tr(zh, "显示批次文件夹", "Reveal check folder"))
                            .clicked()
                        {
                            if let Some(handle) = &self.scan {
                                let _ = open::that(&handle.directory);
                            }
                        }
                    });
                }
            });
        if self.scan.is_none() && self.sources.is_empty() {
            return;
        }
        if self.scan.is_some() {
            let status = &self.scan_status;
            let phase = match status.phase.as_str() {
                "site" => tr(zh, "站点", "Sites"),
                "content" => tr(zh, "完整网址", "Full URLs"),
                _ => tr(zh, "准备中", "Preparing"),
            };
            let state = if self.scan_verified {
                tr(zh, "网络检测已验收", "Network check verified")
            } else {
                match status.state.as_str() {
                    "running" => tr(zh, "运行中", "Running"),
                    "completed" => tr(zh, "已结束，待验收", "Finished; coverage pending"),
                    "aborted" => tr(zh, "已中断", "Interrupted"),
                    "failed" => tr(zh, "失败", "Failed"),
                    _ => tr(zh, "准备中", "Preparing"),
                }
            };
            ui.label(format!(
                "{} · {phase} · {state}",
                tr(zh, "最近报告的状态", "Last reported state")
            ))
            .on_hover_text(tr(
                zh,
                "读取批次文件中最近报告的状态，不核验进程是否仍在运行。",
                "Reads the batch's last saved status; this does not verify that the process is still running.",
            ));
            if let (Some(done), Some(total)) = (status.completed, status.total) {
                ui.add(
                    egui::ProgressBar::new(if total == 0 {
                        0.0
                    } else {
                        done as f32 / total as f32
                    })
                    .text(format!("{done} / {total}")),
                );
            }
        }
        let limitations = source_limitation_count(&self.sources);
        ui.horizontal_wrapped(|ui| {
            if untested > 0 {
                ui.small(format!(
                    "{} {untested} · {}",
                    tr(zh, "尚未检查", "Untested"),
                    tr(zh, "未检查项会保留", "unchecked links stay in the export"),
                ));
            } else if self.active_db {
                ui.weak(tr(
                    zh,
                    "已匹配检测证据，可在详情中查看时间及复核后端。",
                    "Check evidence is attached; inspect its age and backends in Details.",
                ));
            }
            if limitations > 0 {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "{} {limitations} {}",
                        tr(zh, "来源覆盖不完整 ·", "Incomplete source coverage ·"),
                        tr(
                            zh,
                            "个来源存在限制或问题",
                            "sources have limitations or issues"
                        ),
                    ),
                );
            }
            if self.sources.is_empty() {
                ui.small(tr(
                    zh,
                    "未附带浏览器来源报告",
                    "No browser source report attached",
                ));
            }
        });
        if self.sources.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(format!("{}  ·  {}", tr(zh, "已导入来源与覆盖范围", "Imported sources & coverage"), self.sources.len())).open(self.source_details_open.take()).show(ui, |ui| {
            ui.colored_label(ui.visuals().warn_fg_color, tr(zh, "以下是已记录的来源；未知存储、Safari 自定义收藏夹映射等不能据此宣称全部覆盖。", "These are recorded sources. Unknown stores and custom Safari Favorites mappings are not proof of universal coverage."));
            if self.sources.is_empty() { ui.label(tr(zh, "此 HTML 没有附带浏览器来源报告。", "This HTML has no browser source report attached.")); }
            egui::ScrollArea::vertical().id_salt("sources-scroll").max_height(190.0).show(ui, |ui| {
                let mut browser = "";
                for row in &self.sources {
                    if row.browser != browser {
                        ui.strong(if row.browser == "Unclassified directory" { tr(zh, "未识别的目录候选", "Unclassified directory candidates") } else { &row.browser });
                        browser = &row.browser;
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.label(&row.profile);
                        ui.weak(collection_label(zh, &row.collection));
                        let label = source_label(zh, row.state);
                        let response = if matches!(row.state, SourceState::Imported | SourceState::Empty) { ui.label(label) }
                            else { ui.colored_label(ui.visuals().warn_fg_color, label) };
                        response.on_hover_text(&row.detail);
                        if let Some(count) = row.count { ui.label(count.to_string()); }
                    });
                    if matches!(row.state, SourceState::Failed | SourceState::PermissionDenied | SourceState::Unverified) {
                        ui.add(egui::Label::new(RichText::new(&row.detail).small()).wrap());
                    }
                }
            });
            ui.small(tr(zh, "数量为来源原始记录数，尚未去重。悬停状态可查看路径和原因。", "Counts are source records before deduplication. Hover a state for its path and explanation."));
        });
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let mut changed = false;
        egui::Frame::default()
            .inner_margin(egui::Margin::symmetric(0, 2))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.spacing_mut().button_padding = egui::vec2(8.0, 2.0);
                    ui.spacing_mut().interact_size.y = 28.0;
                    let search_response = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text(tr(
                                    zh,
                                    "搜索标题、网址、文件夹",
                                    "Search title, URL or folder",
                                ))
                                .desired_width((ui.available_width() * 0.26).clamp(180.0, 300.0)),
                        )
                        .accessible_name(tr(zh, "搜索书签", "Search bookmarks"));
                    changed |= search_response.changed();
                    egui::ComboBox::from_id_salt("verdict")
                        .selected_text(verdict_label(zh, &self.verdict_filter))
                        .show_ui(ui, |ui| {
                            let mut verdicts: BTreeSet<_> =
                                self.verdict_counts.keys().map(String::as_str).collect();
                            verdicts.insert("");
                            verdicts.insert("duplicates");
                            for value in verdicts {
                                changed |= ui
                                    .selectable_value(
                                        &mut self.verdict_filter,
                                        value.into(),
                                        verdict_label(zh, value),
                                    )
                                    .changed();
                            }
                        });
                    let labels = [
                        tr(zh, "全部决定", "All decisions"),
                        tr(zh, "未决定", "Undecided"),
                        tr(zh, "保留", "Keep"),
                        tr(zh, "移除", "Remove"),
                    ];
                    egui::ComboBox::from_id_salt("decision")
                        .selected_text(labels[usize::from(self.decision_filter)])
                        .show_ui(ui, |ui| {
                            for (value, label) in (0_u8..).zip(labels) {
                                changed |= ui
                                    .selectable_value(&mut self.decision_filter, value, label)
                                    .changed();
                            }
                        });
                    egui::ComboBox::from_id_salt("grouping")
                        .selected_text(group_label(zh, self.grouping))
                        .show_ui(ui, |ui| {
                            for value in [
                                GroupChoice::Folder,
                                GroupChoice::Website,
                                GroupChoice::Status,
                                GroupChoice::None,
                            ] {
                                changed |= ui
                                    .selectable_value(
                                        &mut self.grouping,
                                        value,
                                        group_label(zh, value),
                                    )
                                    .changed();
                            }
                        });
                    if !self.history.is_empty() && ui.button(tr(zh, "撤销", "Undo")).clicked() {
                        self.undo();
                    }
                    ui.menu_button(
                        tr(zh, "更多选择…", "More selection options…"),
                        |ui| {
                            if ui
                                .button(tr(zh, "全选筛选结果", "Select all filtered"))
                                .clicked()
                            {
                                self.selected = self.filtered.iter().copied().collect();
                                ui.close();
                            }
                            if ui
                                .button(tr(
                                    zh,
                                    "只选尚未检查与待复核项",
                                    "Select untested and uncertain",
                                ))
                                .clicked()
                            {
                                self.selected = self
                                    .filtered
                                    .iter()
                                    .copied()
                                    .filter(|&id| {
                                        matches!(
                                            doc.items[id].verdict.as_str(),
                                            "untested"
                                                | "needs_review"
                                                | "temporary_failure"
                                                | "protected"
                                                | "probable_dead"
                                        )
                                    })
                                    .collect();
                                ui.close();
                            }
                            if ui
                                .button(tr(zh, "反选筛选结果", "Invert filtered selection"))
                                .clicked()
                            {
                                for &id in &self.filtered {
                                    if !self.selected.remove(&id) {
                                        self.selected.insert(id);
                                    }
                                }
                                ui.close();
                            }
                            if !self.selected.is_empty()
                                && ui.button(tr(zh, "清空选择", "Clear selection")).clicked()
                            {
                                self.selected.clear();
                                ui.close();
                            }
                            if self.grouping != GroupChoice::None {
                                ui.separator();
                                if ui
                                    .button(tr(zh, "展开全部分组", "Expand all groups"))
                                    .clicked()
                                {
                                    self.collapsed_groups.clear();
                                    self.group_rows = native_groups::rows(
                                        &self.groups,
                                        &self.collapsed_groups,
                                        self.grouping,
                                    );
                                    ui.close();
                                }
                                if ui
                                    .button(tr(zh, "折叠全部分组", "Collapse all groups"))
                                    .clicked()
                                {
                                    self.collapsed_groups =
                                        self.groups.iter().map(|g| g.key.clone()).collect();
                                    self.group_rows = native_groups::rows(
                                        &self.groups,
                                        &self.collapsed_groups,
                                        self.grouping,
                                    );
                                    ui.close();
                                }
                            }
                        },
                    )
                    .response
                    .on_hover_text(tr(
                        zh,
                        "批量选择和分组展开操作",
                        "Bulk selection and group expansion actions",
                    ));
                    ui.weak(format!("{}/{}", self.filtered.len(), doc.items.len()));
                });
            });
        if changed {
            self.filter_dirty = true;
            self.rebuild_filter();
        }
        if !self.selected.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "{} {}",
                    self.selected.len(),
                    tr(zh, "项已选", "selected")
                ));
                let hidden = self
                    .selected
                    .iter()
                    .filter(|id| self.filtered.binary_search(id).is_err())
                    .count();
                if hidden > 0 {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!("{hidden} {}", tr(zh, "项被筛选隐藏", "hidden by filter")),
                    );
                }
                if ui.button(tr(zh, "保留", "Keep")).clicked() {
                    self.decide(
                        self.selected.iter().copied().collect(),
                        Some(Decision::Keep),
                    );
                }
                if ui
                    .button(tr(zh, "移除", "Remove"))
                    .on_hover_text(tr(
                        zh,
                        "只标记为不导出，可撤销；原 HTML 不会改变。",
                        "Marks items out of the export and can be undone; the original HTML is unchanged.",
                    ))
                    .clicked()
                {
                    self.decide(
                        self.selected.iter().copied().collect(),
                        Some(Decision::Remove),
                    );
                }
                if ui.small_button(tr(zh, "重置", "Reset")).clicked() {
                    self.decide(self.selected.iter().copied().collect(), None);
                }
                if ui.small_button(tr(zh, "取消选择", "Deselect")).clicked() {
                    self.selected.clear();
                }
            });
        }
        ui.add_space(4.0);
        if self.filtered.is_empty() {
            ui.label(tr(
                zh,
                "没有匹配的书签。试着清空搜索或重置筛选。",
                "No bookmarks match. Clear search or reset filters.",
            ));
            if ui
                .button(tr(zh, "重置搜索与筛选", "Reset search and filters"))
                .clicked()
            {
                self.search.clear();
                self.verdict_filter.clear();
                self.decision_filter = 0;
                self.filter_dirty = true;
            }
            return;
        }
        let row_height = if self.preferences.developer_mode {
            68.0
        } else if self.preferences.density == DisplayDensity::Compact {
            36.0
        } else {
            40.0
        };
        let mut action = None;
        let mut collapse_changed = false;
        // Virtual rows must fit their children, including the second URL line.
        ui.spacing_mut().interact_size.y = 20.0;
        ui.spacing_mut().button_padding.y = 0.0;
        egui::ScrollArea::vertical()
            .id_salt("bookmarks")
            .auto_shrink([false, false])
            .show_rows(ui, row_height, self.group_rows.len(), |ui, range| {
                for row in range {
                    let id = match self.group_rows[row] {
                        GroupRow::Bookmark(id) => id,
                        GroupRow::Header(index) => {
                            let group = &self.groups[index];
                            let label = match self.grouping {
                                GroupChoice::Folder if group.label.is_empty() => {
                                    tr(zh, "未分组", "Ungrouped")
                                }
                                GroupChoice::Website if group.label == "non_web" => {
                                    tr(zh, "非网页链接", "Non-web links")
                                }
                                GroupChoice::Status => verdict_label(zh, &group.label),
                                _ => &group.label,
                            };
                            let count = group
                                .ids
                                .iter()
                                .filter(|id| self.selected.contains(id))
                                .count();
                            let mut all = count == group.ids.len();
                            let partial = count > 0 && !all;
                            egui::Frame::default()
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE)
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(8, 0))
                                .show(ui, |ui| {
                                    ui.set_min_height(row_height);
                                    ui.push_id(&group.key, |ui| {
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(ui.available_width(), row_height),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                            let group_selection = ui
                                                .add_sized(
                                                    [24.0, 24.0],
                                                    egui::Checkbox::new(&mut all, "")
                                                    .indeterminate(partial),
                                                )
                                                .accessible_name(format!(
                                                    "{}: {label}",
                                                    tr(zh, "选择分组", "Select group")
                                                ));
                                            if group_selection.changed() {
                                                native_groups::select_group(
                                                    &mut self.selected,
                                                    &group.ids,
                                                    all,
                                                );
                                            }
                                            let collapsed =
                                                self.collapsed_groups.contains(&group.key);
                                            let toggle = tr(
                                                zh,
                                                if collapsed { "展开" } else { "收起" },
                                                if collapsed { "Expand" } else { "Collapse" },
                                            );
                                            let collapse_clicked = {
                                                let response = ui.add_sized(
                                                    [24.0, 24.0],
                                                    egui::Button::new("").frame(false),
                                                );
                                                let center = response.rect.center();
                                                let points = if collapsed {
                                                    [
                                                        center + egui::vec2(-2.0, -5.0),
                                                        center + egui::vec2(3.0, 0.0),
                                                        center + egui::vec2(-2.0, 5.0),
                                                    ]
                                                } else {
                                                    [
                                                        center + egui::vec2(-5.0, -2.0),
                                                        center + egui::vec2(0.0, 3.0),
                                                        center + egui::vec2(5.0, -2.0),
                                                    ]
                                                };
                                                let stroke = egui::Stroke::new(
                                                    1.5,
                                                    ui.style().interact(&response).fg_stroke.color,
                                                );
                                                ui.painter().line_segment([points[0], points[1]], stroke);
                                                ui.painter().line_segment([points[1], points[2]], stroke);
                                                response
                                                    .accessible_name(format!("{toggle}: {label}"))
                                                    .on_hover_text(toggle)
                                                    .clicked()
                                            };
                                            if collapse_clicked {
                                                collapse_changed = true;
                                                if collapsed {
                                                    self.collapsed_groups.remove(&group.key);
                                                } else {
                                                    self.collapsed_groups.insert(group.key.clone());
                                                }
                                            }
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(label).strong().size(13.0),
                                                )
                                                .truncate(),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.weak(if count == 0 {
                                                        format!("{} {}", group.ids.len(), tr(zh, "项", "items"))
                                                    } else {
                                                        format!("{} {} · {} {}", group.ids.len(), tr(zh, "项", "items"), tr(zh, "已选", "selected"), count)
                                                    });
                                                },
                                            );
                                        });
                                    });
                                });
                            continue;
                        }
                    };
                    let item = &doc.items[id];
                    ui.push_id(id, |ui| {
                        let highlighted = self.focused == Some(id) || self.selected.contains(&id);
                        egui::Frame::default()
                            .fill(if highlighted {
                                ui.visuals().selection.bg_fill
                            } else {
                                egui::Color32::TRANSPARENT
                            })
                            .corner_radius(egui::CornerRadius::same(5))
                            .inner_margin(egui::Margin::symmetric(8, 0))
                            .show(ui, |ui| {
                                    ui.set_min_height(row_height);
                                ui.horizontal(|ui| {
                                    let title = if item.title.is_empty() {
                                        tr(zh, "无标题", "Untitled")
                                    } else {
                                        &item.title
                                    };
                                    let mut selected = self.selected.contains(&id);
                                    let selection_response = ui
                                        .add_sized([24.0, 24.0], egui::Checkbox::new(&mut selected, ""))
                                        .on_hover_text(tr(zh, "选择此书签", "Select this bookmark"))
                                        .accessible_name(format!(
                                            "{}: {title}",
                                            tr(zh, "选择书签", "Select bookmark")
                                        ));
                                    if selection_response.changed() {
                                        if selected {
                                            self.selected.insert(id);
                                        } else {
                                            self.selected.remove(&id);
                                        }
                                    }
                                    if self.grouping != GroupChoice::None {
                                        ui.add_space(32.0);
                                    }
                                    let status_width = 120.0;
                                    let text_width = (ui.available_width()
                                        - status_width
                                        - ui.spacing().item_spacing.x)
                                        .max(40.0);
                                    let response = ui.allocate_ui_with_layout(
                                        egui::vec2(text_width, if self.preferences.developer_mode { 44.0 } else { row_height }),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.spacing_mut().item_spacing.y = 2.0;
                                        let response = ui.add_sized(
                                            [text_width, 20.0],
                                            egui::Button::new("")
                                                .left_text(title)
                                                .selected(self.focused == Some(id))
                                                .frame(false)
                                                .truncate(),
                                        )
                                        .on_hover_text(title);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&item.url).small().weak(),
                                            )
                                            .halign(egui::Align::Min)
                                            .truncate(),
                                        )
                                        .on_hover_text(&item.url);
                                        response
                                    });
                                    if response.inner.clicked() {
                                        self.focused = Some(id);
                                    }
                                    response.inner.context_menu(|ui| {
                                        for (label, decision) in [
                                            (tr(zh, "保留", "Keep"), Some(Decision::Keep)),
                                            (tr(zh, "移除", "Remove"), Some(Decision::Remove)),
                                            (tr(zh, "重置", "Reset"), None),
                                        ] {
                                            let button = ui.button(label);
                                            let button =
                                                if matches!(decision, Some(Decision::Remove)) {
                                                    button.on_hover_text(tr(
                                                        zh,
                                                        "只标记为不导出，可撤销；原 HTML 不会改变。",
                                                        "Marks the item out of the export and can be undone; the original HTML is unchanged.",
                                                    ))
                                                } else {
                                                    button
                                                };
                                            if button.clicked() {
                                                action = Some((id, decision));
                                                ui.close();
                                            }
                                        }
                                    });
                                    let (status, color) = match self.decisions.get(&id) {
                                        Some(Decision::Remove) => (
                                            tr(zh, "待移除", "Remove"),
                                            ui.visuals().error_fg_color,
                                        ),
                                        Some(Decision::Keep) => (
                                            tr(zh, "保留", "Keep"),
                                            ui.visuals().hyperlink_color,
                                        ),
                                        None => {
                                            let color = match item.verdict.as_str() {
                                                "confirmed_dead" => ui.visuals().error_fg_color,
                                                "needs_review" | "temporary_failure" => {
                                                    ui.visuals().warn_fg_color
                                                }
                                                "live" | "recovered" => ui.visuals().hyperlink_color,
                                                _ => ui.visuals().weak_text_color(),
                                            };
                                            (verdict_label(zh, &item.verdict), color)
                                        }
                                    };
                                    ui.add_sized(
                                        [status_width, 20.0],
                                        egui::Label::new(RichText::new(status).small().strong().color(color))
                                            .halign(egui::Align::Max)
                                            .truncate(),
                                    )
                                    .on_hover_text(status);
                                });
                                if self.preferences.developer_mode {
                                    ui.monospace(format!(
                                        "[id={id}; verdict={}; decision={:?}]",
                                        item.verdict,
                                        self.decisions.get(&id)
                                    ));
                                }
                            });
                    });
                }
            });
        if collapse_changed {
            self.group_rows =
                native_groups::rows(&self.groups, &self.collapsed_groups, self.grouping);
        }
        if let Some((id, decision)) = action {
            self.decide(vec![id], decision);
        }
    }

    fn details(&mut self, ui: &mut egui::Ui) {
        let zh = self.zh();
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let Some(item) = self.focused.and_then(|id| doc.items.get(id)) else {
            return;
        };
        ui.horizontal(|ui| {
            ui.strong(tr(zh, "详情与证据", "Details & evidence"));
            if ui.small_button(tr(zh, "收起", "Hide")).clicked() {
                self.focused = None;
            }
        });
        egui::ScrollArea::vertical().id_salt("evidence").show(ui, |ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(&item.title).size(18.0).strong());
            ui.label(RichText::new(&item.url).small());
            ui.weak(item.path.join(" / "));
            ui.label(verdict_label(zh, &item.verdict));
            ui.horizontal(|ui| {
                if ui.button(tr(zh, "保留", "Keep")).clicked() { self.decide(vec![item.id], Some(Decision::Keep)); }
                if ui.button(tr(zh, "移除", "Remove")).clicked() { self.decide(vec![item.id], Some(Decision::Remove)); }
                if ui.small_button(tr(zh, "重置", "Reset")).clicked() { self.decide(vec![item.id], None); }
            });
            if (item.url.starts_with("https://") || item.url.starts_with("http://")) && ui.button(tr(zh, "在浏览器中打开", "Open in browser")).clicked() {
                if let Err(error) = open::that(&item.url) { self.message = error.to_string(); }
            }
            ui.separator();
            if let Some(when) = &item.last_checked_at {
                ui.small(when);
                if DateTime::parse_from_rfc3339(when).map(|t| !StateMachine::is_current_evidence(t.with_timezone(&Utc), Utc::now())).unwrap_or(true) {
                    ui.colored_label(ui.visuals().warn_fg_color, tr(zh, "证据已超过 24 小时、时间在未来或时间无效，请在移除前重新核对。", "Evidence is older than 24 hours or has a future or invalid timestamp. Recheck before removal."));
                }
            }
            if item.evidence.is_empty() { ui.label(tr(zh, "暂无检测证据；这不代表链接失效。", "No check evidence is available. This does not mean the link is dead.")); }
            for evidence in &item.evidence {
                ui.group(|ui| {
                    ui.strong(&evidence.backend);
                    if let Some(code) = evidence.status_code { ui.label(format!("HTTP {code}")); }
                    if let Some(error) = &evidence.error_kind { ui.label(error); }
                    if let Some(when) = &evidence.finished_at { ui.small(when); }
                });
            }
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.settings else {
            return;
        };
        let zh = draft.language.prefers_chinese();
        let mut open = true;
        let mut save = false;
        let mut cancel = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        egui::Window::new(tr(zh, "设置", "Settings")).id(egui::Id::unique("settings-window")).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).open(&mut open).collapsible(false).resizable(false).default_width(560.0).show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    for (tab, label) in [
                        (0, tr(zh, "外观", "Appearance")),
                        (1, tr(zh, "导入与证据", "Import & evidence")),
                        (2, tr(zh, "网络与检测", "Network & checks")),
                        (3, tr(zh, "高级", "Advanced")),
                        (4, tr(zh, "关于", "About")),
                    ] {
                        if ui
                            .add(egui::Button::selectable(self.settings_tab == tab, label))
                            .clicked()
                        {
                            self.settings_tab = tab;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().id_salt("settings-content").max_height(ctx.content_rect().height() * 0.42).show(ui, |ui| {
                    ui.heading(match self.settings_tab {
                        0 => tr(zh, "外观", "Appearance"),
                        1 => tr(zh, "导入与证据", "Import & evidence"),
                        2 => tr(zh, "网络与检测", "Network & checks"),
                        3 => tr(zh, "高级", "Advanced"),
                        _ => tr(zh, "关于", "About"),
                    });
                    ui.separator();
                    match self.settings_tab {
                0 => {
                    preference_row(
                        ui,
                        tr(zh, "主题", "Theme"),
                        tr(zh, "跟随系统外观", "Follow system appearance"),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for (value, label) in [
                                    (ThemeChoice::System, tr(zh, "系统", "System")),
                                    (ThemeChoice::Light, tr(zh, "浅色", "Light")),
                                    (ThemeChoice::Dark, tr(zh, "深色", "Dark")),
                                ] {
                                    ui.radio_value(&mut draft.theme, value, label);
                                }
                            });
                        },
                    );
                    preference_row(
                        ui,
                        tr(zh, "强调色", "Accent color"),
                        tr(zh, "用于按钮与选中状态", "Used for actions and selection"),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for (value, label) in [
                                    (AccentChoice::Blue, tr(zh, "蓝色", "Blue")),
                                    (AccentChoice::Teal, tr(zh, "青绿", "Teal")),
                                    (AccentChoice::Violet, tr(zh, "紫色", "Violet")),
                                    (AccentChoice::Monochrome, tr(zh, "黑白", "Monochrome")),
                                ] {
                                    ui.radio_value(&mut draft.accent, value, label);
                                }
                            });
                        },
                    );
                    preference_row(
                        ui,
                        tr(zh, "语言", "Language"),
                        tr(zh, "更改后立即预览", "Preview changes immediately"),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.radio_value(
                                    &mut draft.language,
                                    LanguageChoice::System,
                                    tr(zh, "系统", "System"),
                                );
                                ui.radio_value(
                                    &mut draft.language,
                                    LanguageChoice::SimplifiedChinese,
                                    "简体中文",
                                );
                                ui.radio_value(
                                    &mut draft.language,
                                    LanguageChoice::English,
                                    "English",
                                );
                            });
                        },
                    );
                    preference_row(
                        ui,
                        tr(zh, "列表密度", "List density"),
                        tr(zh, "控制每行书签的留白", "Space between bookmark rows"),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.radio_value(
                                    &mut draft.density,
                                    DisplayDensity::Comfortable,
                                    tr(zh, "舒适", "Comfortable"),
                                );
                                ui.radio_value(
                                    &mut draft.density,
                                    DisplayDensity::Compact,
                                    tr(zh, "紧凑", "Compact"),
                                );
                            });
                        },
                    );
                    preference_row(
                        ui,
                        tr(zh, "界面缩放", "Interface scale"),
                        tr(zh, "调整文字与控件大小", "Adjust text and control size"),
                        |ui| {
                            ui.add(egui::Slider::new(&mut draft.text_scale, 0.9..=1.3));
                        },
                    );
                    preference_row(
                        ui,
                        tr(zh, "书签详情", "Bookmark details"),
                        tr(zh, "选择书签时显示详情", "Show details for a selected bookmark"),
                        |ui| {
                            ui.checkbox(&mut draft.show_details, tr(zh, "显示详情", "Show details"));
                        },
                    );
                    preference_row(
                        ui,
                        tr(zh, "画面比例", "Layout proportions"),
                        tr(zh, "详情不会挤压窄窗口中的列表", "Details float on narrow windows"),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for (value, label) in [
                                    (LayoutChoice::Balanced, tr(zh, "均衡", "Balanced")),
                                    (LayoutChoice::WideList, tr(zh, "列表优先", "More list")),
                                    (LayoutChoice::WideDetails, tr(zh, "详情优先", "More details")),
                                ] {
                                    ui.radio_value(&mut draft.layout, value, label);
                                }
                            });
                        },
                    );
                    preference_row(
                        ui,
                        tr(zh, "字体", "Typeface"),
                        tr(zh, "系统字体或清晰阅读", "System or reading face"),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.radio_value(
                                    &mut draft.font,
                                    FontChoice::System,
                                    tr(zh, "系统", "System"),
                                );
                                ui.radio_value(
                                    &mut draft.font,
                                    FontChoice::Readable,
                                    tr(zh, "清晰阅读", "Reading"),
                                );
                            });
                        },
                    );
                }
                1 => {
                    ui.label(tr(zh, "导入 HTML 或自动读取已发现的浏览器与 Profile。阅读列表的支持情况会单独列出。", "Import HTML or read discovered browsers and profiles. Reading-list support is reported separately."));
                    ui.add_space(8.0);
                    ui.strong(tr(zh, "已有检测结果（可选）", "Existing check results (optional)"));
                    if ui.add_enabled(!self.busy, egui::Button::new(tr(zh, "选择健康数据库…", "Choose health database…"))).clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("SQLite", &["sqlite", "db"]).pick_file() { self.db = Some(path); }
                    }
                    if let Some(path) = &self.db { ui.small(path.display().to_string()); }
                    if ui.add_enabled(!self.busy, egui::Button::new(tr(zh, "选择来源报告…", "Choose source report…"))).clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() { self.source_review = Some(path); }
                    }
                    ui.small(tr(zh, "在下次打开 HTML 时读取。选择文件不会更改当前决定。", "Attached on the next HTML import. Choosing a file does not change current decisions."));
                    ui.separator();
                    ui.label(tr(zh, "未决定的书签会保留；精确重复按完整 URL 匹配。导出时保留第一个重复项及原有文件夹。", "Undecided bookmarks are retained. Exact duplicates match the full URL; the first occurrence and original folders are kept."));
                    ui.strong(tr(zh,"默认分组方式","Default grouping"));
                    egui::ComboBox::from_id_salt("default-grouping").selected_text(group_label(zh,draft.grouping)).show_ui(ui,|ui| {
                        for value in [GroupChoice::Folder,GroupChoice::Website,GroupChoice::Status,GroupChoice::None] { ui.selectable_value(&mut draft.grouping,value,group_label(zh,value)); }
                    });
                    ui.small(tr(zh,"也可以在列表中临时切换分组；不会移动书签或改变导出文件夹。","You can change grouping in the list too. It never moves bookmarks or changes exported folders."));
                }
                2 => {
                    ui.label(tr(zh, "仅用于下一批新检测。应用设置不会更改已启动的检测，也不会修改系统网络设置。", "Used for the next new check batch. Applying settings does not change running checks or system networking."));
                    ui.separator();
                    ui.strong(tr(zh, "连接方式", "Connection route"));
                    ui.small(tr(zh,"只控制书签检测的连接方式，不改变浏览器打开网址时的代理。","Controls bookmark checks only, not the proxy used when opening links in your browser."));
                    ui.horizontal_wrapped(|ui| {
                        ui.radio_value(&mut draft.network.route, NetworkRoute::LocalHttpProxy, tr(zh, "本地 HTTP 代理", "Local HTTP proxy"));
                        ui.radio_value(&mut draft.network.route, NetworkRoute::Direct, tr(zh, "明确直连", "Explicit direct"));
                    });
                    let proxy_enabled = draft.network.route == NetworkRoute::LocalHttpProxy;
                    ui.add_enabled_ui(proxy_enabled, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(tr(zh, "代理主机", "Proxy host"));
                            ui.selectable_value(&mut draft.network.proxy_host, ProxyHost::Ipv4Loopback, "127.0.0.1");
                            ui.selectable_value(&mut draft.network.proxy_host, ProxyHost::Localhost, "localhost");
                        });
                        ui.horizontal(|ui| {
                            let label = ui.label(tr(zh, "代理端口", "Proxy port"));
                            ui.add(egui::DragValue::new(&mut draft.network.proxy_port).range(0..=u16::MAX)).labelled_by(label.id);
                        });
                    });
                    ui.horizontal(|ui| {
                        if ui.add_enabled(proxy_enabled && draft.network.validate().is_ok() && !self.proxy_test_running, egui::Button::new(tr(zh, "检查代理端口", "Check proxy port"))).clicked() {
                            let network = draft.network;
                            let sender = self.sender.clone();
                            let context = self.context.clone();
                            self.proxy_test_running = true;
                            self.proxy_test_result = None;
                            self.context.request_repaint();
                            std::thread::spawn(move || {
                                let _repaint = RepaintOnExit(context);
                                let _ = sender.send(Event::ProxyChecked(network, network.check_proxy_connection()));
                            });
                        }
                        if self.proxy_test_running { ui.spinner(); ui.label(tr(zh, "正在检查…", "Checking…")); }
                    });
                    if let Some((network, result)) = &self.proxy_test_result {
                        if *network == draft.network {
                            match result {
                                Ok(()) => { ui.label(tr(zh, "本地端口可连接；尚未验证 HTTP 代理或外网。", "Local port is reachable. HTTP proxying and internet access are not verified.")); }
                                Err(error) => { ui.colored_label(ui.visuals().error_fg_color, format!("{} {error}", tr(zh, "本地端口不可连接：", "Local port was not reachable:"))); }
                            }
                        }
                    }
                    ui.small(if proxy_enabled {
                        tr(zh, "仅接受本机 HTTP 代理；启动前检查端口连接。不保存账号或密码。", "Local HTTP proxies only. The port is checked before launch. No credentials are stored.")
                    } else {
                        tr(zh, "不继承环境代理。直连会向目标网站暴露本机网络出口；请确认此方式符合你的网络需求。", "Ignores environment proxies. Sites see your direct network connection; choose this only when it suits your network.")
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        let label = ui.label(tr(zh, "单次请求超时", "Request timeout"));
                        ui.add(egui::DragValue::new(&mut draft.network.timeout_secs).range(5..=60).suffix(tr(zh, " 秒", " s"))).labelled_by(label.id);
                    });
                    ui.small(tr(zh, "超时只是暂时失败，不会因此判为死亡。", "A timeout is a temporary failure, not proof that a bookmark is dead."));
                    ui.separator();
                    ui.strong(tr(zh, "检测域名时使用的 DNS", "DNS service for domain checks"));
                    ui.small(tr(zh,"DNS 用来查询网址对应的服务器。这里的额外查询用于核对失败的网址，不修改电脑的 DNS。","DNS finds the server for a website. These extra queries help review failed links and do not change your computer's DNS."));
                    ui.add_enabled_ui(!proxy_enabled, |ui| {
                        ui.radio_value(&mut draft.network.dns_strategy, DnsStrategy::Automatic, tr(zh, "自动：国内域名优先 AliDNS，其余优先 Cloudflare", "Automatic: AliDNS for Chinese domains, Cloudflare otherwise"));
                        ui.radio_value(&mut draft.network.dns_strategy, DnsStrategy::CloudflareFirst, tr(zh, "优先 Cloudflare", "Cloudflare first"));
                        ui.radio_value(&mut draft.network.dns_strategy, DnsStrategy::AliDnsFirst, tr(zh, "优先 AliDNS", "AliDNS first"));
                        ui.checkbox(&mut draft.network.dns_fallback_on_error,tr(zh,"主 DNS 暂时失败时尝试备用 DNS","Try backup DNS when the primary service fails"));
                        ui.small(tr(zh,"备用路径：Cloudflare → AliDNS；AliDNS → Cloudflare。两家均为加密查询，不改系统 DNS。","Backup path: Cloudflare → AliDNS, or AliDNS → Cloudflare. Both use encrypted queries; system DNS is unchanged."));
                    });
                    ui.small(if proxy_enabled {
                        tr(zh, "代理模式不执行独立直连 DNS 检查，域名由代理处理。上述策略保留至以后切换直连时使用。", "Proxy mode does not run independent direct DNS checks. The proxy handles hostnames; this choice is saved for future direct checks.")
                    } else {
                        tr(zh, "两家服务都需独立确认“域名不存在”（NXDOMAIN）。一次查询失败或一次否定都不会触发删除。", "Both providers must independently confirm that the domain does not exist (NXDOMAIN). One failed query or one negative answer never triggers removal.")
                    });
                    ui.separator();
                    ui.small(tr(zh, "保守检测：并发 2 · 慢速限流 · curl 复核 · 全量检查。401/403/429、TLS、DNS 和代理临时错误均保留待复核。", "Conservative checks: concurrency 2, slow rate limits, curl confirmation, no target limit. Protected responses and temporary TLS, DNS or proxy errors remain for review."));
                }
                3 => {
                    ui.checkbox(&mut draft.developer_mode,tr(zh,"开启开发者模式","Enable developer mode"));
                    ui.label(tr(zh,"显示每条书签的内部编号、原始状态和决定标识，以及窗口底部的检测参数 flags。关闭后这些标识不显示。","Shows bookmark IDs, raw status and decision tags, plus check flags in the footer. These identifiers stay hidden when this mode is off."));
                    ui.small(tr(zh,"仅影响显示，不允许写入浏览器，也不放宽清理规则。保存设置后生效。","Display only. It does not enable browser writes or weaken cleanup rules. Save settings to apply."));
                }
                _ => {
                    ui.horizontal(|ui| { app_mark(ui,72.0); ui.vertical(|ui| { ui.heading("Bookmark Cleaner"); ui.label(tr(zh,"为值得保留的链接留出空间","Make room for the links worth keeping")); }); });
                    ui.label(format!("{} {}", tr(zh, "版本", "Version"), env!("CARGO_PKG_VERSION")));
                    ui.label(tr(zh, "整理重复与失效链接，导出一个新的整合书签文件。", "Clean duplicate and dead links, then export one consolidated bookmark file."));
                    ui.label(tr(zh, "Rust 原生桌面界面 · 本地处理", "Native Rust desktop interface · Local processing"));
                    ui.hyperlink_to(tr(zh,"查看项目源码与贡献说明","View source and contribute"),"https://github.com/nowaytouse/browser-bookmark-sync");
                    ui.small(tr(zh,"源码公开。当前仓库未附带许可证文件，不能据此假定任意再分发权限。","Source is public. No license file is declared in this repository; do not assume unrestricted redistribution rights."));
                    ui.small(tr(zh, "清理仅写入新的 HTML 和审计文件。浏览器内的收藏夹保持原样。", "Cleaning writes a new HTML file and audit record. Browser bookmarks remain unchanged."));
                    ui.separator();
                    ui.label(tr(zh, "快捷键：⌘O 导入 · ⌘S 导出 · ⌘Z 撤销 · ⌘, 设置", "Shortcuts: ⌘O Import · ⌘S Export · ⌘Z Undo · ⌘, Settings"));
                }
            }
                });
            });
            if draft.network.validate().is_err() {
                ui.colored_label(ui.visuals().error_fg_color, tr(zh, "代理端口须为 1–65535；请求超时须为 5–60 秒。", "Use a proxy port from 1 to 65535 and a request timeout from 5 to 60 seconds."));
            }
            if let Some(error) = &self.settings_error {
                ui.colored_label(ui.visuals().error_fg_color, format!("{} {error}", tr(zh, "设置未保存：", "Settings were not saved:")));
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(tr(zh, "恢复默认", "Restore defaults")).clicked() { draft = NativePreferences::default(); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    save = ui.add_enabled(draft.network.validate().is_ok(), egui::Button::new(tr(zh, "应用", "Apply"))).clicked();
                    cancel |= ui.button(tr(zh, "取消", "Cancel")).clicked();
                });
            });
        });
        if save {
            let result = self
                .store
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Preferences directory unavailable"))
                .and_then(|s| s.save(&draft).map_err(anyhow::Error::from));
            match result {
                Ok(()) => {
                    self.preferences = draft.normalized();
                    self.grouping = draft.grouping;
                    self.filter_dirty = true;
                    self.settings = None;
                    self.settings_error = None;
                    apply_style(ctx, self.preferences);
                }
                Err(error) => {
                    self.message = error.to_string();
                    self.settings_error = Some(error.to_string());
                    self.settings = Some(draft);
                }
            }
        } else if !open || cancel {
            self.settings = None;
            self.settings_error = None;
            apply_style(ctx, self.preferences);
        } else {
            if self.settings != Some(draft) {
                apply_style(ctx, draft);
            }
            self.settings = Some(draft);
        }
    }

    fn modal(&mut self, ctx: &egui::Context) {
        let zh = self.zh();
        if self.pending_input.is_some() || self.pending_browser_import {
            egui::Modal::new(egui::Id::unique("replace-confirm")).show(ctx, |ui| {
                ui.heading(tr(
                    zh,
                    "当前清理尚未导出",
                    "Current changes are not exported",
                ));
                ui.label(tr(
                    zh,
                    "导入另一份书签会放弃当前决定。",
                    "Importing another document will discard current decisions.",
                ));
                ui.horizontal(|ui| {
                    if ui.button(tr(zh, "取消", "Cancel")).clicked() {
                        self.pending_input = None;
                        self.pending_browser_import = false;
                        self.pending_scan = None;
                    }
                    if ui
                        .button(tr(zh, "放弃并导入", "Discard and import"))
                        .clicked()
                    {
                        if let Some(path) = self.pending_input.take() {
                            self.load(path);
                        } else if self.pending_browser_import {
                            self.import_browsers();
                        }
                    }
                });
            });
        } else if self.close_after_confirm {
            egui::Modal::new(egui::Id::unique("close-confirm")).show(ctx, |ui| {
                ui.heading(tr(zh, "退出前导出更改？", "Changes have not been exported"));
                ui.label(tr(
                    zh,
                    "退出将放弃当前的清理决定。",
                    "Quitting will discard current cleaning decisions.",
                ));
                if ui.button(tr(zh, "继续清理", "Continue cleaning")).clicked() {
                    self.close_after_confirm = false;
                }
                if ui
                    .button(tr(zh, "放弃并退出", "Discard and quit"))
                    .clicked()
                {
                    self.decisions.clear();
                    self.saved_decisions.clear();
                    self.close_after_confirm = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        } else if let Some(path) = self.confirm_export.clone() {
            let removed = self.removed();
            let total = self.doc.as_ref().map_or(0, |doc| doc.items.len());
            egui::Modal::new(egui::Id::unique("export-confirm")).show(ctx, |ui| {
                ui.heading(tr(zh, "导出清理后的 HTML", "Export cleaned HTML"));
                ui.label(format!(
                    "{} {}  ·  {} {removed}",
                    tr(zh, "保留", "Retain"),
                    total - removed,
                    tr(zh, "移除", "Remove")
                ));
                ui.label(path.display().to_string());
                ui.small(format!(
                    "{} {}",
                    tr(zh, "审计文件：", "Audit record:"),
                    audit_path_for(&path).display()
                ));
                ui.label(tr(
                    zh,
                    "创建新文件；已有文件不会被覆盖。",
                    "Creates new files; existing files will not be overwritten.",
                ));
                ui.horizontal(|ui| {
                    if ui.button(tr(zh, "取消", "Cancel")).clicked() {
                        self.confirm_export = None;
                    }
                    if ui
                        .add_enabled(!self.busy, egui::Button::new(tr(zh, "确认导出", "Export")))
                        .clicked()
                    {
                        self.confirm_export = None;
                        self.export(path);
                    }
                });
            });
        }
    }
}

fn action_fill(accent: AccentChoice) -> egui::Color32 {
    match accent {
        AccentChoice::Blue => egui::Color32::from_rgb(33, 105, 190),
        AccentChoice::Teal => egui::Color32::from_rgb(0, 113, 96),
        AccentChoice::Violet => egui::Color32::from_rgb(101, 76, 163),
        AccentChoice::Monochrome => egui::Color32::from_rgb(58, 66, 76),
    }
}

fn centered_content(
    ui: &mut egui::Ui,
    max_width: f32,
    fill_height: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let available = ui.available_size();
    let width = available.x.min(max_width);
    let height = if fill_height { available.y } else { 0.0 };
    ui.horizontal(|ui| {
        ui.add_space(((available.x - width) * 0.5).max(0.0));
        ui.allocate_ui_with_layout(
            egui::vec2(width, height),
            egui::Layout::top_down(egui::Align::Min),
            add_contents,
        );
    });
}

fn preference_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    controls: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        let label_width = (ui.available_width() * 0.32).clamp(132.0, 180.0);
        ui.allocate_ui_with_layout(
            egui::vec2(label_width, 44.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.label(RichText::new(title).strong());
                ui.small(description);
            },
        );
        ui.add_space(8.0);
        controls(ui);
    });
    ui.add_space(5.0);
    ui.separator();
    ui.add_space(5.0);
}

fn duplicate_ids(doc: &ReviewDocument) -> BTreeSet<usize> {
    let mut seen = BTreeSet::new();
    doc.items
        .iter()
        .filter_map(|item| (!seen.insert(item.url.as_str())).then_some(item.id))
        .collect()
}

fn pick_html() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("HTML", &["html", "htm"])
        .pick_file()
}

fn verdict_label(zh: bool, verdict: &str) -> &str {
    match verdict {
        "" => tr(zh, "全部状态", "All states"),
        "duplicates" => tr(zh, "精确重复", "Exact duplicates"),
        "untested" => tr(zh, "未检查", "Untested"),
        "live" => tr(zh, "可访问", "Reachable"),
        "recovered" => tr(zh, "已恢复", "Recovered"),
        "protected" => tr(zh, "访问受保护", "Protected"),
        "needs_review" => tr(zh, "待复核", "Needs review"),
        "temporary_failure" => tr(zh, "暂时失败", "Temporary failure"),
        "confirmed_dead" => tr(zh, "死亡候选", "Dead candidate"),
        _ => verdict,
    }
}

fn collection_label(zh: bool, collection: &str) -> &str {
    match collection {
        "bookmarks" => tr(zh, "收藏夹", "Bookmarks"),
        "reading_list" => tr(zh, "阅读列表", "Reading list"),
        "browser_discovery" => tr(zh, "来源发现（未验证）", "Source discovery (unverified)"),
        _ => collection,
    }
}

fn source_limitation_count(sources: &[SourceRow]) -> usize {
    sources
        .iter()
        .filter(|row| !matches!(row.state, SourceState::Imported | SourceState::Empty))
        .count()
}

fn source_label(zh: bool, state: SourceState) -> &'static str {
    match state {
        SourceState::Imported => tr(zh, "已导入", "Imported"),
        SourceState::Empty => tr(zh, "已读取 · 确认为空", "Read · verified empty"),
        SourceState::Detected => tr(zh, "已发现 · 尚未读取", "Detected · not read"),
        SourceState::Unsupported => tr(zh, "当前不支持", "Unsupported"),
        SourceState::Unavailable => tr(zh, "来源不可用", "Unavailable"),
        SourceState::PermissionDenied => tr(zh, "权限不足", "Permission denied"),
        SourceState::Failed => tr(zh, "读取失败", "Failed"),
        SourceState::Unverified => tr(zh, "未验证", "Unverified"),
        SourceState::Skipped => tr(zh, "已跳过", "Skipped"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ReviewDocument {
        ReviewDocument::load(
            std::path::Path::new("tests/fixtures/review-bookmarks.html"),
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn decisions_undo_and_saved_boundary() {
        let mut app = ReviewApp::default();
        app.decide(vec![1, 2], Some(Decision::Remove));
        assert!(app.dirty());
        app.saved_decisions = app.decisions.clone();
        assert!(!app.dirty());
        app.decide(vec![1], Some(Decision::Keep));
        assert!(app.dirty());
        app.undo();
        assert!(!app.dirty());
    }

    #[test]
    fn evidence_details_warn_for_future_expired_and_invalid_times_without_changing_decisions() {
        let now = Utc::now();
        for (language, warning) in [
            (
                LanguageChoice::English,
                "Evidence is older than 24 hours or has a future or invalid timestamp. Recheck before removal.",
            ),
            (
                LanguageChoice::SimplifiedChinese,
                "证据已超过 24 小时、时间在未来或时间无效，请在移除前重新核对。",
            ),
        ] {
            for (when, warn) in [
                (Some((now + chrono::Duration::hours(1)).to_rfc3339()), true),
                (Some((now - chrono::Duration::hours(25)).to_rfc3339()), true),
                (Some("invalid-timestamp".to_string()), true),
                (Some((now - chrono::Duration::hours(1)).to_rfc3339()), false),
                (None, false),
            ] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                let mut doc = fixture();
                doc.items.truncate(1);
                doc.items[0].verdict = if when.is_some() {
                    "confirmed_dead".into()
                } else {
                    "untested".into()
                };
                doc.items[0].last_checked_at = when.clone();
                let verdict = doc.items[0].verdict.clone();
                let mut app = ReviewApp::default();
                app.preferences.language = language;
                app.accept_document(doc, false);
                app.focused = Some(0);
                let decisions = app.decisions.clone();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(900.0, 800.0),
                        )),
                        ..Default::default()
                    },
                    |ui| app.details(ui),
                );
                output.textures_delta.clear();
                let nodes = output.platform_output.accesskit_update.unwrap().nodes;
                assert!(nodes.iter().any(|(_, node)| {
                    node.value() == Some(verdict_label(app.zh(), &verdict))
                }), "the historical verdict must remain visible");
                assert_eq!(
                    nodes.iter().any(|(_, node)| node.value() == Some(warning)),
                    warn,
                    "timestamp {when:?}, language {language:?}"
                );
                assert_eq!(app.decisions, decisions);
                assert_eq!(app.doc.as_ref().unwrap().items[0].verdict, verdict);
            }
        }
    }

    #[test]
    fn background_operation_failures_use_visible_errors_and_preserve_decisions() {
        for event in [
            Event::ScanStarted(Err(anyhow::anyhow!("launcher failed"))),
            Event::EvidenceLoaded(Err(anyhow::anyhow!("database unreadable"))),
            Event::Exported(Err(anyhow::anyhow!("output unavailable"))),
        ] {
            let mut app = ReviewApp::default();
            app.accept_document(fixture(), false);
            app.decide(vec![0], Some(Decision::Keep));
            let doc = app.doc.clone().unwrap();
            let decisions = app.decisions.clone();
            let saved = app.saved_decisions.clone();
            app.busy = true;
            app.sender.send(event).unwrap();

            app.poll();

            assert!(app.message_is_error, "{}", app.message);
            assert!(!app.busy);
            assert!(Arc::ptr_eq(&doc, app.doc.as_ref().unwrap()));
            assert_eq!(app.decisions, decisions);
            assert_eq!(app.saved_decisions, saved);
            assert!(app.dirty());
        }
    }

    #[test]
    fn panicked_worker_releases_busy_state_and_preserves_decisions() {
        let mut app = ReviewApp::default();
        app.accept_document(fixture(), false);
        app.decide(vec![0], Some(Decision::Keep));
        let doc = app.doc.clone().unwrap();
        let decisions = app.decisions.clone();
        app.busy = true;
        app.scan_verified = true;
        app.spawn_worker(|| panic!("synthetic worker panic"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !app.worker.as_ref().unwrap().is_finished() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }

        app.poll();

        assert!(!app.busy, "a terminated worker must not keep the UI busy");
        assert!(app.worker.is_none());
        assert!(app.message_is_error);
        assert!(!app.scan_verified);
        assert!(Arc::ptr_eq(&doc, app.doc.as_ref().unwrap()));
        assert_eq!(app.decisions, decisions);
        assert!(app.dirty());
    }

    #[test]
    fn completed_worker_keeps_its_queued_result() {
        let mut app = ReviewApp::default();
        app.accept_document(fixture(), false);
        app.busy = true;
        app.spawn_worker(|| {
            Event::Loaded(Err(anyhow::anyhow!("synthetic read error")), false, None)
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !app.worker.as_ref().unwrap().is_finished() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }

        app.poll();

        assert!(!app.busy);
        assert!(app.worker.is_none());
        assert!(app.message_is_error);
        assert!(app.message.contains("synthetic read error"));
        assert!(!app.message.contains("unexpectedly"));
    }

    #[test]
    fn cancelled_import_waits_for_worker_exit_and_discards_a_late_success() {
        let mut app = ReviewApp::default();
        app.preferences.language = LanguageChoice::English;
        app.accept_document(fixture(), true);
        app.decide(vec![0], Some(Decision::Keep));
        app.selected.insert(0);
        app.import_directory = Some(tempfile::tempdir().unwrap());
        let document = app.doc.clone().unwrap();
        let decisions = app.decisions.clone();
        let saved = app.saved_decisions.clone();
        let sources = app.sources.clone();
        let history = app.history.clone();
        let snapshot = app.import_directory.as_ref().unwrap().path().to_path_buf();
        let incoming = tempfile::tempdir().unwrap();
        let discarded = incoming.path().to_path_buf();
        let imported = BrowserImport {
            document: fixture(),
            sources: Vec::new(),
            directory: incoming,
        };
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let sender = app.sender.clone();
        app.busy = true;
        app.import_cancellation = Some(Cancellation::default());
        app.worker = Some(std::thread::spawn(move || {
            sender.send(Event::Imported(Ok(imported))).unwrap();
            ready_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
        }));
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();

        app.cancel_import();
        app.proxy_test_running = true;
        let network = NetworkSettings::default();
        app.sender
            .send(Event::ProxyChecked(network, Ok(())))
            .unwrap();
        app.poll();
        let stayed_busy = app.busy;
        let proxy_completed =
            !app.proxy_test_running && app.proxy_test_result == Some((network, Ok(())));
        let waiting_message = app.message.clone();
        let kept_document_while_waiting = Arc::ptr_eq(&document, app.doc.as_ref().unwrap());
        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !app.worker.as_ref().unwrap().is_finished() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        app.poll();

        assert!(
            stayed_busy,
            "must not report cancellation before worker exit"
        );
        assert!(waiting_message.starts_with("Cancelling;"));
        assert!(
            proxy_completed,
            "proxy results must not wait for an import to exit"
        );
        assert!(kept_document_while_waiting);
        assert!(!app.busy);
        assert!(app.worker.is_none());
        assert!(app.import_cancellation.is_none());
        assert!(!app.message_is_error);
        assert_eq!(
            app.message,
            "Browser import cancelled; current content and decisions are unchanged."
        );
        assert!(Arc::ptr_eq(&document, app.doc.as_ref().unwrap()));
        assert_eq!(app.decisions, decisions);
        assert_eq!(app.saved_decisions, saved);
        assert_eq!(app.sources, sources);
        assert_eq!(app.history, history);
        assert_eq!(app.selected, BTreeSet::from([0]));
        assert!(app.active_db);
        assert_eq!(app.import_directory.as_ref().unwrap().path(), snapshot);
        assert!(snapshot.exists());
        assert!(
            !discarded.exists(),
            "discard the cancelled private import copy"
        );
    }

    #[test]
    fn cancellation_does_not_leak_into_the_next_import() {
        let mut app = ReviewApp::default();
        app.preferences.language = LanguageChoice::English;
        app.busy = true;
        app.import_cancellation = Some(Cancellation::default());
        app.cancel_import();
        app.sender
            .send(Event::Imported(Err(
                crate::sync::cancellation::Cancelled.into()
            )))
            .unwrap();
        app.poll();
        assert!(app.doc.is_none());
        assert!(!app.message_is_error);
        assert!(!app.busy);
        assert!(app.import_cancellation.is_none());

        app.busy = true;
        app.import_cancellation = Some(Cancellation::default());
        app.sender
            .send(Event::Imported(Ok(BrowserImport {
                document: fixture(),
                sources: Vec::new(),
                directory: tempfile::tempdir().unwrap(),
            })))
            .unwrap();
        app.poll();
        assert!(app.doc.is_some());
        assert!(!app.busy);
        assert!(app.import_cancellation.is_none());
        assert!(!app.message_is_error);
    }

    #[test]
    fn worker_completion_wakes_the_ui_even_after_a_panic() {
        use std::sync::atomic::{AtomicBool, Ordering};

        for panic in [false, true] {
            let mut app = ReviewApp::default();
            let repainted = Arc::new(AtomicBool::new(false));
            let observed = repainted.clone();
            let ui_thread = std::thread::current().id();
            app.context.set_repaint_observer(move |_| {
                if std::thread::current().id() != ui_thread {
                    observed.store(true, Ordering::SeqCst);
                }
            });
            app.busy = true;
            app.spawn_worker(move || {
                assert!(!panic, "synthetic wake-up panic");
                Event::Loaded(Err(anyhow::anyhow!("synthetic result")), false, None)
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !app.worker.as_ref().unwrap().is_finished() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }

            assert!(
                repainted.load(Ordering::SeqCst),
                "worker did not wake the UI"
            );
            app.poll();
            assert!(!app.busy);
        }
    }

    #[test]
    fn unreadable_scan_status_revokes_verification_and_shows_an_error() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("process-exit.json"), b"{broken").unwrap();
        let mut app = ReviewApp {
            scan: Some(ScanHandle {
                directory: directory.path().to_path_buf(),
            }),
            scan_verified: true,
            ..Default::default()
        };

        app.refresh_scan();

        assert!(app.message_is_error);
        assert!(!app.scan_verified);
        assert!(app.scan.is_some());
    }

    #[test]
    fn successful_status_refresh_clears_the_error_without_reverifying_coverage() {
        let directory = tempfile::tempdir().unwrap();
        let receipt = directory.path().join("process-exit.json");
        std::fs::write(&receipt, b"{broken").unwrap();
        let mut app = ReviewApp {
            scan: Some(ScanHandle {
                directory: directory.path().to_path_buf(),
            }),
            scan_verified: true,
            ..Default::default()
        };
        app.refresh_scan();
        assert!(app.message_is_error);
        std::fs::write(&receipt, b"{}").unwrap();

        app.refresh_scan();

        assert!(!app.message_is_error);
        assert!(!app.scan_verified);
    }

    #[test]
    fn import_deduplicates_only_later_exact_urls_and_can_be_undone() {
        let mut app = ReviewApp::default();
        let doc = fixture();
        let duplicates = duplicate_ids(&doc);
        assert!(!duplicates.is_empty());
        app.accept_document(doc, false);
        assert_eq!(
            app.decisions.keys().copied().collect::<BTreeSet<_>>(),
            duplicates
        );
        app.undo();
        assert!(app.decisions.is_empty());
    }

    #[test]
    fn filtered_selection_and_bulk_undo_preserve_other_occurrences() {
        let mut app = ReviewApp {
            doc: Some(Arc::new(fixture())),
            search: "duplicate".into(),
            ..Default::default()
        };
        app.rebuild_filter();
        assert_eq!(app.filtered, [1, 2]);
        app.decide(app.filtered.clone(), Some(Decision::Remove));
        assert_eq!(app.decisions.len(), 2);
        app.undo();
        assert!(app.decisions.is_empty());
    }

    #[test]
    fn evidence_reload_preserves_source_coverage_and_current_decisions() {
        let mut app = ReviewApp::default();
        let mut doc = fixture();
        doc.coverage = Some(serde_json::json!({"collection_status":"partial"}));
        app.accept_document(doc, false);
        let decisions = app.decisions.clone();
        app.sender
            .send(Event::EvidenceLoaded(Ok((fixture(), false))))
            .unwrap();
        app.poll();
        assert_eq!(app.decisions, decisions);
        assert_eq!(
            app.doc.as_ref().unwrap().coverage.as_ref().unwrap()["collection_status"],
            "partial"
        );
        assert!(!app.scan_verified);
    }

    #[test]
    fn document_counts_refresh_on_evidence_reload_and_replacement_not_review_decisions() {
        let mut app = ReviewApp::default();
        app.accept_document(fixture(), false);
        assert_eq!(app.verdict_counts, BTreeMap::from([("untested".into(), 5)]));
        let mut checked = fixture();
        checked.items[0].verdict = "confirmed_dead".into();
        checked.items[1].verdict = "protected".into();
        app.sender
            .send(Event::EvidenceLoaded(Ok((checked, true))))
            .unwrap();
        app.poll();
        assert_eq!(
            app.verdict_counts,
            BTreeMap::from([
                ("untested".into(), 3),
                ("confirmed_dead".into(), 1),
                ("protected".into(), 1),
            ])
        );
        app.decide(vec![0], Some(Decision::Keep));
        app.undo();
        assert_eq!(app.verdict_counts.get("confirmed_dead"), Some(&1));
        app.accept_document(fixture(), false);
        assert_eq!(app.verdict_counts, BTreeMap::from([("untested".into(), 5)]));
    }

    #[test]
    fn synthetic_large_document_counter_benchmark() {
        let mut doc = fixture();
        let item = doc.items[0].clone();
        doc.items = (0..100_000)
            .map(|id| {
                let mut item = item.clone();
                item.id = id;
                item
            })
            .collect();
        let cached = count_verdicts(&doc);
        assert_eq!(cached.get("untested"), Some(&100_000));
        let started = std::time::Instant::now();
        for _ in 0..100 {
            std::hint::black_box(
                std::hint::black_box(&doc.items)
                    .iter()
                    .filter(|item| item.verdict == "untested")
                    .count(),
            );
            std::hint::black_box(
                std::hint::black_box(&doc.items)
                    .iter()
                    .filter(|item| item.verdict == "confirmed_dead")
                    .count(),
            );
        }
        let rescanned = started.elapsed();
        let started = std::time::Instant::now();
        for _ in 0..100 {
            std::hint::black_box(std::hint::black_box(&cached).get("untested"));
            std::hint::black_box(std::hint::black_box(&cached).get("confirmed_dead"));
        }
        println!("synthetic 100000 entries, 100 counter reads: rescanned={rescanned:?}, cached={:?}; count construction is excluded", started.elapsed());
    }

    #[test]
    fn primary_action_fills_keep_white_text_readable() {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        for accent in [
            AccentChoice::Blue,
            AccentChoice::Teal,
            AccentChoice::Violet,
            AccentChoice::Monochrome,
        ] {
            let color = action_fill(accent);
            let luminance = 0.2126 * channel(color.r())
                + 0.7152 * channel(color.g())
                + 0.0722 * channel(color.b());
            assert!(
                1.05 / (luminance + 0.05) >= 4.5,
                "primary fill must preserve readable white button text"
            );
        }
    }

    #[test]
    fn virtual_bookmark_rows_match_the_scroll_stride() {
        for (density, developer_mode, row_height) in [
            (DisplayDensity::Compact, false, 36.0),
            (DisplayDensity::Comfortable, false, 40.0),
            (DisplayDensity::Compact, true, 68.0),
            (DisplayDensity::Comfortable, true, 68.0),
        ] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            install_chinese_font(&ctx);
            let mut app = ReviewApp::default();
            app.preferences.density = density;
            app.preferences.developer_mode = developer_mode;
            app.preferences.language = LanguageChoice::English;
            app.preferences.grouping = GroupChoice::None;
            app.load_demo().unwrap();
            app.rebuild_filter();
            apply_style(&ctx, app.preferences);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 1800.0),
                    )),
                    ..Default::default()
                },
                |ui| app.list(ui),
            );
            output.textures_delta.clear();
            let mut tops: Vec<_> = output
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .iter()
                .filter(|(_, node)| {
                    node.role() == egui::accesskit::Role::CheckBox
                        && node
                            .label()
                            .is_some_and(|label| label.starts_with("Select bookmark:"))
                })
                .map(|(_, node)| node.bounds().unwrap().y0)
                .collect();
            tops.sort_by(f64::total_cmp);
            assert_eq!(tops.len(), 20, "all synthetic rows must be laid out");
            let expected =
                f64::from(row_height + ctx.style_of(egui::Theme::Dark).spacing.item_spacing.y);
            for pair in tops.windows(2) {
                assert!(
                    (pair[1] - pair[0] - expected).abs() < 1.0,
                    "row stride must match virtual scrolling: {pair:?}, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn native_review_layout_keeps_the_list_visible_and_selection_targets_large_enough() {
        for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
            for (language, scale, width) in [
                (LanguageChoice::English, 1.0, 1180.0),
                (LanguageChoice::SimplifiedChinese, 1.3, 800.0),
            ] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                install_chinese_font(&ctx);
                let mut app = ReviewApp::default();
                app.preferences.theme = theme;
                app.preferences.language = language;
                app.preferences.text_scale = scale;
                app.load_demo().unwrap();
                app.rebuild_filter();
                apply_style(&ctx, app.preferences);
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 780.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        app.summary(ui);
                        app.list(ui);
                    },
                );
                output.textures_delta.clear();
                let nodes = output.platform_output.accesskit_update.unwrap().nodes;
                let selection: Vec<_> = nodes
                    .iter()
                    .filter(|(_, node)| node.role() == egui::accesskit::Role::CheckBox)
                    .map(|(_, node)| node.bounds().unwrap())
                    .collect();
                assert!(!selection.is_empty());
                assert!(
                    selection
                        .iter()
                        .all(|rect| rect.width() >= 24.0 && rect.height() >= 24.0),
                    "selection controls must have usable targets"
                );
                let first = selection
                    .iter()
                    .map(|rect| rect.y0)
                    .min_by(f64::total_cmp)
                    .unwrap();
                assert!(
                    first < 780.0 * 0.5,
                    "the list must start in the upper half: {first}"
                );
            }
        }
    }

    #[test]
    fn synthetic_demo_group_selection_undo_export_and_developer_gating() {
        let mut app = ReviewApp::default();
        app.load_demo().unwrap();
        app.rebuild_filter();
        assert_eq!(app.doc.as_ref().unwrap().items.len(), 20);
        assert_eq!(app.duplicates.len(), 2);
        assert!(app.developer_flags().is_none());
        app.preferences.developer_mode = true;
        assert!(app.developer_flags().unwrap().contains("flags:"));
        app.preferences.developer_mode = false;
        assert!(app.developer_flags().is_none());
        app.search = "Read later".into();
        app.filter_dirty = true;
        app.rebuild_filter();
        assert_eq!(app.filtered.len(), 3);
        let ids = app
            .groups
            .iter()
            .flat_map(|g| g.ids.iter().copied())
            .collect::<Vec<_>>();
        native_groups::select_group(&mut app.selected, &ids, true);
        let before = app.decisions.clone();
        app.decide(ids.clone(), Some(Decision::Remove));
        app.undo();
        assert_eq!(app.decisions, before);
        app.collapsed_groups = app.groups.iter().map(|g| g.key.clone()).collect();
        let rows = native_groups::rows(&app.groups, &app.collapsed_groups, app.grouping);
        assert!(rows.iter().all(|row| matches!(row, GroupRow::Header(_))));
        assert_eq!(app.selected.len(), 3);
        let output = app
            .import_directory
            .as_ref()
            .unwrap()
            .path()
            .join("cleaned-synthetic.html");
        let audit = app
            .doc
            .as_ref()
            .unwrap()
            .export_copy(&app.decisions, &output, true)
            .unwrap();
        assert_eq!((audit.retained_count, audit.removed_count), (18, 2));
        assert!(app.doc.as_ref().unwrap().input_path.exists());
    }

    #[test]
    fn reopening_settings_does_not_discard_an_active_network_draft() {
        let mut app = ReviewApp::default();
        app.open_settings();
        app.settings.as_mut().unwrap().network.route = NetworkRoute::Direct;
        app.settings.as_mut().unwrap().network.timeout_secs = 30;
        app.open_settings();
        assert_eq!(app.settings.unwrap().network.route, NetworkRoute::Direct);
        assert_eq!(app.settings.unwrap().network.timeout_secs, 30);
        assert_eq!(app.preferences.network, NetworkSettings::default());
    }

    #[test]
    fn proxy_test_result_does_not_clear_a_parallel_import_busy_state() {
        let mut app = ReviewApp {
            busy: true,
            proxy_test_running: true,
            ..Default::default()
        };
        let network = NetworkSettings::default();
        app.sender
            .send(Event::ProxyChecked(network, Ok(())))
            .unwrap();
        app.poll();
        assert!(app.busy);
        assert!(!app.proxy_test_running);
        assert_eq!(app.proxy_test_result, Some((network, Ok(()))));
    }

    #[test]
    fn first_browser_import_failure_reports_no_new_content() {
        let mut app = ReviewApp {
            busy: true,
            ..Default::default()
        };
        app.preferences.language = LanguageChoice::English;
        app.sender
            .send(Event::Imported(Err(anyhow::anyhow!(
                "Operation not permitted (os error 1)"
            ))))
            .unwrap();
        app.poll();
        assert!(app.doc.is_none());
        assert!(!app.busy);
        assert!(app.message_is_error);
        assert_eq!(app.message, "Browser import failed; no new content was imported.\nOperation not permitted (os error 1)");
    }

    #[test]
    fn browser_import_failure_preserves_document_decisions_sources_and_snapshot() {
        let mut app = ReviewApp::default();
        app.preferences.language = LanguageChoice::SimplifiedChinese;
        let mut doc = fixture();
        doc.coverage = Some(serde_json::json!({"collection_status":"partial"}));
        app.accept_document(doc, false);
        app.decide(vec![0], Some(Decision::Keep));
        app.import_directory = Some(tempfile::tempdir().unwrap());
        let document = app.doc.clone().unwrap();
        let decisions = app.decisions.clone();
        let sources = app.sources.clone();
        let snapshot = app.import_directory.as_ref().unwrap().path().to_path_buf();
        app.busy = true;
        app.sender
            .send(Event::Imported(Err(anyhow::anyhow!("Profile read failed"))))
            .unwrap();
        app.poll();
        assert!(Arc::ptr_eq(&document, app.doc.as_ref().unwrap()));
        assert_eq!(app.decisions, decisions);
        assert_eq!(app.sources, sources);
        assert_eq!(app.import_directory.as_ref().unwrap().path(), snapshot);
        assert!(snapshot.exists());
        assert!(!app.busy);
        assert!(app.message_is_error);
        assert!(app
            .message
            .starts_with("浏览器导入失败；原有内容和清理决定未改变。\n"));
    }

    #[test]
    fn partial_browser_import_reports_limitations_and_opens_source_details() {
        let report = serde_json::json!({"sources":{"external_files_and_directories":[
            {"browser":"Chrome", "profile":"Default", "source_type":"reading_list", "status":"complete", "bookmark_count":3},
            {"browser":"Chrome", "profile":"Empty", "source_type":"reading_list", "status":"complete", "bookmark_count":0},
            {"browser":"Safari", "profile":"Denied", "source_type":"reading_list", "status":"permission_denied", "bookmark_count":0},
            {"browser":"Firefox", "profile":"Default", "source_type":"reading_list", "status":"unsupported", "bookmark_count":0}
        ]}});
        let sources = native_sources::rows_from_report(&report);
        let mut doc = fixture();
        doc.coverage = Some(report);
        let mut app = ReviewApp {
            busy: true,
            message_is_error: true,
            ..Default::default()
        };
        app.preferences.language = LanguageChoice::English;
        app.sender
            .send(Event::Imported(Ok(BrowserImport {
                document: doc,
                directory: tempfile::tempdir().unwrap(),
                sources,
            })))
            .unwrap();
        app.poll();
        assert!(app.doc.is_some());
        assert!(!app.busy);
        assert!(!app.message_is_error);
        assert_eq!(source_limitation_count(&app.sources), 2);
        assert_eq!(app.source_details_open, Some(true));
        assert_eq!(app.message, "Readable browser content imported with 2 source read issues or coverage limitations; see source details.");
    }
}
