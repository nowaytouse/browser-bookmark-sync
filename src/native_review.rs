//! Native, copy-only bookmark review window. No browser profile writes or scans.

use crate::health::review::{audit_path_for, Decision, ReviewDocument};
use chrono::{DateTime, Utc};
use eframe::egui::{self, Color32, RichText};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

enum Event {
    Loaded(anyhow::Result<ReviewDocument>, bool),
    Exported(anyhow::Result<(PathBuf, usize, usize)>),
}

pub fn start(
    input: Option<PathBuf>,
    db: Option<PathBuf>,
    source_review: Option<PathBuf>,
) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "书签副本审阅",
        options,
        Box::new(move |cc| {
            install_chinese_font(&cc.egui_ctx);
            let mut app = ReviewApp::default();
            app.db = db;
            app.source_review = source_review;
            if let Some(path) = input {
                app.load(path);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("启动原生窗口失败: {error}"))
}

fn install_chinese_font(ctx: &egui::Context) {
    #[cfg(target_os = "macos")]
    let candidates: &[&str] = &[
        "/System/Library/Fonts/STHeiti Medium.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    ];
    #[cfg(target_os = "windows")]
    let candidates: &[&str] = &[
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let candidates: &[&str] = &[
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("cjk".into(), Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, "cjk".into());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
    // ponytail: platform defaults may lack CJK glyphs; add another known system font when needed.
}

struct ReviewApp {
    doc: Option<Arc<ReviewDocument>>,
    db: Option<PathBuf>,
    source_review: Option<PathBuf>,
    decisions: BTreeMap<usize, Decision>,
    saved_decisions: BTreeMap<usize, Decision>,
    active_db: bool,
    selected: BTreeSet<usize>,
    focused: Option<usize>,
    history: Vec<Vec<(usize, Option<Decision>)>>,
    search: String,
    verdict_filter: String,
    decision_filter: u8,
    filtered: Vec<usize>,
    filter_dirty: bool,
    sender: mpsc::Sender<Event>,
    receiver: mpsc::Receiver<Event>,
    busy: bool,
    message: String,
    pending_input: Option<PathBuf>,
    confirm_export: Option<PathBuf>,
    close_after_confirm: bool,
}

impl Default for ReviewApp {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            doc: None,
            db: None,
            source_review: None,
            decisions: BTreeMap::new(),
            saved_decisions: BTreeMap::new(),
            active_db: false,
            selected: BTreeSet::new(),
            focused: None,
            history: Vec::new(),
            search: String::new(),
            verdict_filter: String::new(),
            decision_filter: 0,
            filtered: Vec::new(),
            filter_dirty: true,
            sender,
            receiver,
            busy: false,
            message: "选择导出的 HTML 副本开始；不会访问或修改浏览器资料。".into(),
            pending_input: None,
            confirm_export: None,
            close_after_confirm: false,
        }
    }
}

impl ReviewApp {
    fn load(&mut self, path: PathBuf) {
        if self.busy {
            return;
        }
        let db = self.db.clone();
        let source = self.source_review.clone();
        let sender = self.sender.clone();
        self.busy = true;
        self.message = format!("正在读取 {}", path.display());
        std::thread::spawn(move || {
            let result = ReviewDocument::load(&path, db.as_deref(), source.as_deref());
            let _ = sender.send(Event::Loaded(result, db.is_some()));
        });
    }

    fn request_load(&mut self, path: PathBuf) {
        if self.dirty() {
            self.pending_input = Some(path);
        } else {
            self.load(path);
        }
    }

    fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            self.busy = false;
            match event {
                Event::Loaded(Ok(doc), active_db) => {
                    let count = doc.items.len();
                    self.doc = Some(Arc::new(doc));
                    self.decisions.clear();
                    self.selected.clear();
                    self.history.clear();
                    self.focused = None;
                    self.saved_decisions.clear();
                    self.active_db = active_db;
                    self.filter_dirty = true;
                    self.message =
                        format!("已载入 {count} 个书签；默认全部保留。未检查不代表失效。");
                }
                Event::Loaded(Err(error), _) => self.message = format!("读取失败：{error:#}"),
                Event::Exported(Ok((path, retained, removed))) => {
                    self.saved_decisions = self.decisions.clone();
                    self.message = format!(
                        "已另存 {}；保留 {retained}，移除 {removed}。审计：{}",
                        path.display(),
                        audit_path_for(&path).display()
                    );
                }
                Event::Exported(Err(error)) => self.message = format!("导出失败：{error:#}"),
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
                if !self.verdict_filter.is_empty() && item.verdict != self.verdict_filter {
                    continue;
                }
                let decision = self.decisions.get(&item.id);
                if !matches!(self.decision_filter, 0)
                    && !match self.decision_filter {
                        1 => decision.is_none(),
                        2 => decision == Some(&Decision::Keep),
                        3 => decision == Some(&Decision::Remove),
                        _ => true,
                    }
                {
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
            match decision {
                Some(value) => {
                    self.decisions.insert(*id, value);
                }
                None => {
                    self.decisions.remove(id);
                }
            }
        }
        self.message = format!("已更新 {} 项人工决定。", prior.len());
        self.history.push(prior);
        self.filter_dirty = true;
    }

    fn undo(&mut self) {
        if let Some(prior) = self.history.pop() {
            for (id, old) in prior {
                match old {
                    Some(value) => {
                        self.decisions.insert(id, value);
                    }
                    None => {
                        self.decisions.remove(&id);
                    }
                }
            }
            self.filter_dirty = true;
            self.message = "已撤销上一步人工决定。".into();
        }
    }

    fn dirty(&self) -> bool {
        self.decisions != self.saved_decisions
    }

    fn export(&mut self, path: PathBuf) {
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let decisions = self.decisions.clone();
        let sender = self.sender.clone();
        self.busy = true;
        self.message = "正在写入新的 HTML 副本和审计文件…".into();
        std::thread::spawn(move || {
            let result = doc
                .export_copy(&decisions, &path, true)
                .map(|audit| (path, audit.retained_count, audit.removed_count));
            let _ = sender.send(Event::Exported(result));
        });
    }
}

impl eframe::App for ReviewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if ctx.input(|i| i.viewport().close_requested()) && (self.busy || self.dirty()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.busy {
                self.message = "正在读取或导出，请等待完成后再退出。".into();
            } else {
                self.close_after_confirm = true;
            }
        }
        self.top_bar(ctx);
        self.rebuild_filter();
        self.modal(ctx);
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.label(&self.message);
        });
        if self.doc.is_none() {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(70.0);
                    ui.heading("书签副本审阅");
                    ui.label(
                        "打开导出的 HTML 副本。健康证据与来源覆盖文件可选。不会扫描或修改浏览器。",
                    );
                    if ui.button("打开 HTML 副本…").clicked() {
                        if let Some(path) = pick_html() {
                            self.request_load(path);
                        }
                    }
                });
            });
            return;
        }
        egui::SidePanel::right("details")
            .resizable(true)
            .default_width(360.0)
            .min_width(270.0)
            .show(ctx, |ui| self.details(ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(!self.busy, |ui| self.list(ui));
        });
    }
}

impl ReviewApp {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!self.busy, egui::Button::new("打开 HTML…"))
                    .clicked()
                {
                    if let Some(path) = pick_html() {
                        self.request_load(path);
                    }
                }
                if ui
                    .add_enabled(!self.busy, egui::Button::new("健康数据库…"))
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        self.db = Some(path);
                    }
                    self.message = "数据库将在下次打开 HTML 时读取；当前结果不变。".into();
                }
                if ui
                    .add_enabled(!self.busy, egui::Button::new("来源覆盖 JSON…"))
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("JSON", &["json"])
                        .pick_file()
                    {
                        self.source_review = Some(path);
                    }
                    self.message = "覆盖文件将在下次打开 HTML 时读取；当前结果不变。".into();
                }
                if ui
                    .add_enabled(
                        self.doc.is_some() && !self.busy,
                        egui::Button::new("另存审阅副本…"),
                    )
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("HTML", &["html", "htm"])
                        .set_file_name("bookmarks-reviewed.html")
                        .save_file()
                    {
                        self.confirm_export = Some(path);
                    }
                }
            });
            if let Some(doc) = &self.doc {
                ui.label(format!("输入：{}", doc.input_path.display()));
                ui.horizontal_wrapped(|ui| {
                    let removed = self
                        .decisions
                        .values()
                        .filter(|&&d| d == Decision::Remove)
                        .count();
                    ui.label(format!(
                        "总计 {} · 筛选 {} · 已选 {} · 待移除 {} · 保留 {}",
                        doc.items.len(),
                        self.filtered.len(),
                        self.selected.len(),
                        removed,
                        doc.items.len() - removed
                    ));
                    ui.label(if self.active_db {
                        "当前副本已读取健康数据库（仅匹配证据）"
                    } else {
                        "当前副本无健康数据库：未检查状态不代表失效"
                    });
                });
                if let Some(coverage) = &doc.coverage {
                    let status = coverage
                        .get("collection_status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("未声明");
                    let universal = coverage
                        .get("universal_source_coverage_verified")
                        .and_then(|v| v.as_bool())
                        == Some(true);
                    ui.colored_label(
                        if universal {
                            Color32::LIGHT_GREEN
                        } else {
                            Color32::YELLOW
                        },
                        format!(
                            "来源覆盖：{status}；{}",
                            if universal {
                                "文件声明已验证全部来源"
                            } else {
                                "未验证全部来源，勿视为完整"
                            }
                        ),
                    );
                } else {
                    ui.colored_label(
                        Color32::YELLOW,
                        "未提供来源覆盖文件：无法确认所有浏览器/资料来源都已纳入",
                    );
                }
            }
        });
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let verdicts: BTreeSet<String> = doc.items.iter().map(|i| i.verdict.clone()).collect();
        let mut filter_changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("搜索标题、网址、路径：");
            filter_changed |= ui.text_edit_singleline(&mut self.search).changed();
            egui::ComboBox::from_id_salt("verdict")
                .selected_text(if self.verdict_filter.is_empty() {
                    "全部状态"
                } else {
                    &self.verdict_filter
                })
                .show_ui(ui, |ui| {
                    filter_changed |= ui
                        .selectable_value(&mut self.verdict_filter, String::new(), "全部状态")
                        .changed();
                    for verdict in &verdicts {
                        filter_changed |= ui
                            .selectable_value(&mut self.verdict_filter, verdict.clone(), verdict)
                            .changed();
                    }
                });
            egui::ComboBox::from_id_salt("decision")
                .selected_text(
                    ["全部决定", "未决定", "保留", "移除"][self.decision_filter as usize],
                )
                .show_ui(ui, |ui| {
                    for (n, label) in ["全部决定", "未决定", "保留", "移除"].iter().enumerate()
                    {
                        filter_changed |= ui
                            .selectable_value(&mut self.decision_filter, n as u8, *label)
                            .changed();
                    }
                });
        });
        if filter_changed {
            self.filter_dirty = true;
            self.rebuild_filter();
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(format!("仅全选当前筛选结果（{} 项）", self.filtered.len()))
                .clicked()
            {
                self.selected = self.filtered.iter().copied().collect();
            }
            let hidden = self
                .selected
                .iter()
                .filter(|id| self.filtered.binary_search(id).is_err())
                .count();
            if hidden > 0 {
                ui.colored_label(Color32::YELLOW, format!("含隐藏选择 {hidden} 项"));
            }
            if ui.button("清空选择").clicked() {
                self.selected.clear();
            }
            if ui
                .add_enabled(!self.selected.is_empty(), egui::Button::new("保留所选"))
                .clicked()
            {
                self.decide(
                    self.selected.iter().copied().collect(),
                    Some(Decision::Keep),
                );
            }
            if ui
                .add_enabled(!self.selected.is_empty(), egui::Button::new("移除所选"))
                .clicked()
            {
                self.decide(
                    self.selected.iter().copied().collect(),
                    Some(Decision::Remove),
                );
            }
            if ui
                .add_enabled(!self.selected.is_empty(), egui::Button::new("重置所选"))
                .clicked()
            {
                self.decide(self.selected.iter().copied().collect(), None);
            }
            if ui
                .add_enabled(!self.history.is_empty(), egui::Button::new("撤销"))
                .clicked()
            {
                self.undo();
            }
        });
        ui.separator();
        let Some(doc) = self.doc.clone() else {
            return;
        };
        let mut toggle = Vec::new();
        let mut focus = None;
        let mut actions = Vec::new();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 54.0, self.filtered.len(), |ui, range| {
                for row in range {
                    let id = self.filtered[row];
                    let item = &doc.items[id];
                    ui.horizontal(|ui| {
                        let mut checked = self.selected.contains(&id);
                        if ui.checkbox(&mut checked, "").changed() {
                            toggle.push((id, checked));
                        }
                        let title = if item.title.is_empty() {
                            "（无标题）"
                        } else {
                            &item.title
                        };
                        if ui
                            .add_sized(
                                [(ui.available_width() - 235.0).max(70.0), 22.0],
                                egui::Button::new(format!("{}  ·  {}", title, item.verdict))
                                    .selected(self.focused == Some(id))
                                    .truncate(),
                            )
                            .clicked()
                        {
                            focus = Some(id);
                        }
                        match self.decisions.get(&id) {
                            Some(Decision::Keep) => {
                                ui.colored_label(Color32::LIGHT_GREEN, "保留");
                            }
                            Some(Decision::Remove) => {
                                ui.colored_label(Color32::LIGHT_RED, "移除");
                            }
                            None => {
                                ui.label("未决定");
                            }
                        }
                        if ui.small_button("保留").clicked() {
                            actions.push((id, Some(Decision::Keep)));
                        }
                        if ui.small_button("移除").clicked() {
                            actions.push((id, Some(Decision::Remove)));
                        }
                        if ui.small_button("重置").clicked() {
                            actions.push((id, None));
                        }
                    });
                    ui.add(
                        egui::Label::new(RichText::new(&item.url).small().color(Color32::GRAY))
                            .truncate(),
                    );
                    ui.separator();
                }
            });
        for (id, checked) in toggle {
            if checked {
                self.selected.insert(id);
            } else {
                self.selected.remove(&id);
            }
        }
        if let Some(id) = focus {
            self.focused = Some(id);
        }
        for (id, decision) in actions {
            self.decide(vec![id], decision);
        }
    }

    fn details(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else {
            return;
        };
        ui.heading("证据与详情");
        let Some(id) = self.focused else {
            ui.label("点击列表项查看详情。");
            return;
        };
        let Some(item) = doc.items.get(id) else {
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(RichText::new(&item.title).strong());
            ui.label(&item.url);
            ui.label(format!("路径：{}", item.path.join(" / ")));
            ui.label(format!("健康状态：{}", item.verdict));
            ui.label(format!("检查次数：{}", item.total_checks));
            match &item.last_checked_at {
                Some(when) => {
                    ui.label(format!("最后检查：{when}"));
                    if DateTime::parse_from_rfc3339(when)
                        .map(|t| {
                            Utc::now().signed_duration_since(t.with_timezone(&Utc))
                                > chrono::Duration::hours(24)
                        })
                        .unwrap_or(false)
                    {
                        ui.colored_label(Color32::YELLOW, "证据超过 24 小时，可能已过期");
                    }
                }
                None => {
                    ui.colored_label(Color32::YELLOW, "无检查记录；不能据此判定失效");
                }
            }
            if (item.url.starts_with("https://") || item.url.starts_with("http://"))
                && ui.button("手动在浏览器打开 HTTP(S) 链接").clicked()
            {
                if let Err(error) = open::that(&item.url) {
                    self.message = format!("打开链接失败：{error}");
                }
            }
            ui.separator();
            ui.label("检查证据：");
            if item.evidence.is_empty() {
                ui.label("无证据");
            }
            for evidence in &item.evidence {
                ui.group(|ui| {
                    ui.label(format!("来源：{}", evidence.backend));
                    if let Some(code) = evidence.status_code {
                        ui.label(format!("HTTP 状态：{code}"));
                    }
                    if let Some(kind) = &evidence.error_kind {
                        ui.label(format!("错误：{kind}"));
                    }
                    if let Some(when) = &evidence.finished_at {
                        ui.label(format!("时间：{when}"));
                    }
                });
            }
        });
    }

    fn modal(&mut self, ctx: &egui::Context) {
        if let Some(path) = self.pending_input.clone() {
            egui::Modal::new(egui::Id::new("replace-confirm")).show(ctx, |ui| {
                ui.heading("尚有未保存的人工决定");
                ui.label("打开另一份 HTML 将清除当前人工决定。请先另存审阅副本，或明确放弃。");
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        self.pending_input = None;
                    }
                    if ui.button("放弃决定并打开").clicked() {
                        self.pending_input = None;
                        self.load(path);
                    }
                });
            });
            return;
        }
        if self.close_after_confirm {
            egui::Modal::new(egui::Id::new("close-confirm")).show(ctx, |ui| {
                ui.heading("退出确认");
                ui.label("当前人工决定尚未另存。确定放弃并退出？");
                if ui.button("继续审阅").clicked() {
                    self.close_after_confirm = false;
                }
                if ui.button("放弃并退出").clicked() {
                    self.decisions.clear();
                    self.saved_decisions.clear();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            return;
        }
        if let Some(path) = self.confirm_export.clone() {
            let removed = self
                .decisions
                .values()
                .filter(|&&d| d == Decision::Remove)
                .count();
            let total = self.doc.as_ref().map_or(0, |doc| doc.items.len());
            egui::Modal::new(egui::Id::new("export-confirm")).show(ctx, |ui| {
                ui.heading("确认另存审阅副本");
                ui.label(format!(
                    "将保留 {} 项、移除 {} 项。仅写入新 HTML 与审计文件，不修改原件。",
                    total - removed,
                    removed
                ));
                ui.label(format!("目标：{}", path.display()));
                ui.label(format!("审计：{}", audit_path_for(&path).display()));
                ui.label("若目标或审计文件已存在，导出会拒绝覆盖。");
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        self.confirm_export = None;
                    }
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("确认另存"))
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

fn pick_html() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("HTML 书签副本", &["html", "htm"])
        .pick_file()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn filtered_selection_and_bulk_undo_preserve_other_occurrences() -> anyhow::Result<()> {
        let mut app = ReviewApp::default();
        app.doc = Some(Arc::new(ReviewDocument::load(
            std::path::Path::new("tests/fixtures/review-bookmarks.html"),
            None,
            None,
        )?));
        app.search = "duplicate".into();
        app.rebuild_filter();
        assert_eq!(app.filtered, [1, 2]);
        app.selected = app.filtered.iter().copied().collect();
        app.decide(
            app.selected.iter().copied().collect(),
            Some(Decision::Remove),
        );
        assert_eq!(app.decisions.len(), 2);
        app.undo();
        assert!(app.decisions.is_empty());
        Ok(())
    }
}
