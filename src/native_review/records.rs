//! Quiet, read-only history presentation. Loading never changes review decisions.

use super::{tr, verdict_label};
use crate::native_history::{self, Activity, HistoryPage, HistoryRecord, PAGE_SIZE};
use chrono::{DateTime, Local};
use eframe::egui::{self, RichText};
use serde_json::Value;
use std::{path::PathBuf, thread::JoinHandle};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    db: PathBuf,
    url: String,
    offset: usize,
    generation: u64,
}

#[derive(Default)]
pub(super) struct RecordsPanel {
    desired: Option<Request>,
    loaded: Option<Request>,
    worker: Option<JoinHandle<(Request, Result<HistoryPage, String>)>>,
    running: Option<Request>,
    page: Option<HistoryPage>,
    error: Option<String>,
    generation: u64,
}

impl RecordsPanel {
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.desired = None;
        self.loaded = None;
        self.page = None;
        self.error = None;
    }

    fn poll(&mut self) {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            if let Some(worker) = self.worker.take() {
                let running = self.running.take();
                match worker.join() {
                    Ok((request, result)) if self.desired.as_ref() == Some(&request) => {
                        self.loaded = Some(request);
                        match result {
                            Ok(page) => {
                                self.page = Some(page);
                                self.error = None;
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                    Ok(_) => {}
                    Err(_) if running == self.desired => {
                        self.loaded = self.desired.clone();
                        self.error = Some("history worker ended unexpectedly".into());
                    }
                    Err(_) => {}
                }
            }
        }
        if self.worker.is_none() && self.loaded != self.desired {
            if let Some(request) = self.desired.clone() {
                let running = request.clone();
                match std::thread::Builder::new()
                    .name("bookmark-history".into())
                    .spawn(move || {
                        let result =
                            native_history::load_page(&request.db, &request.url, request.offset);
                        (request, result)
                    }) {
                    Ok(worker) => {
                        self.worker = Some(worker);
                        self.running = Some(running);
                    }
                    Err(error) => {
                        self.loaded = self.desired.clone();
                        self.error = Some(error.to_string());
                    }
                }
            }
        }
    }

    fn request(&mut self, db: PathBuf, url: &str) {
        if self
            .desired
            .as_ref()
            .is_none_or(|request| request.db != db || request.url != url)
        {
            self.invalidate();
            self.desired = Some(Request {
                db,
                url: url.into(),
                offset: 0,
                generation: self.generation,
            });
        }
    }

    fn change_page(&mut self, offset: usize) {
        if let Some(request) = &mut self.desired {
            self.generation = self.generation.wrapping_add(1);
            request.offset = offset;
            request.generation = self.generation;
            self.loaded = None;
            self.page = None;
            self.error = None;
        }
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        db: Option<PathBuf>,
        url: &str,
        zh: bool,
        developer: bool,
    ) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr(zh, "检测记录", "Check history"))
                    .size(16.0)
                    .strong(),
            );
            if ui
                .add(egui::Button::new(tr(zh, "刷新", "Refresh")).min_size(egui::vec2(24.0, 24.0)))
                .on_hover_text(tr(
                    zh,
                    "只重新读取已保存的记录，不会请求网站。",
                    "Reload saved records only. No website requests.",
                ))
                .clicked()
            {
                self.generation = self.generation.wrapping_add(1);
                if let Some(request) = &mut self.desired {
                    request.generation = self.generation;
                }
            }
        });
        ui.label(
            RichText::new(tr(
                zh,
                "记录反映检测时的情况，不代表网址现在的状态。",
                "Results reflect the time of each check, not the current state.",
            ))
            .small()
            .weak(),
        );
        let Some(db) = db else {
            self.invalidate();
            ui.add_space(8.0);
            ui.label(tr(
                zh,
                "还没有关联检测记录。载入已保存的检测结果后，可在这里回查。",
                "No saved checks are attached. Load check results to see their history here.",
            ));
            return;
        };
        self.request(db, url);
        self.poll();
        let loading = self.loaded != self.desired;
        if loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak(tr(zh, "正在读取记录…", "Reading records…"));
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(80));
        }
        if let Some(error) = &self.error {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                tr(
                    zh,
                    "记录未能刷新。当前书签和清理决定没有改变。",
                    "Could not refresh history. Bookmarks and review decisions are unchanged.",
                ),
            );
            if self.page.is_some() {
                ui.weak(tr(
                    zh,
                    "下方仍显示上次读取的记录。",
                    "Showing the last successfully loaded records.",
                ));
            }
            egui::CollapsingHeader::new(tr(zh, "查看错误原因", "Error details"))
                .id_salt("history-error")
                .show(ui, |ui| {
                    ui.label(error);
                });
        }
        let Some(page) = &self.page else {
            let offset = self.desired.as_ref().map_or(0, |request| request.offset);
            if offset > 0
                && ui
                    .button(tr(zh, "返回较新记录", "Back to newer records"))
                    .clicked()
            {
                self.change_page(offset.saturating_sub(PAGE_SIZE));
            }
            return;
        };
        if page.legacy && !page.records.is_empty() {
            ui.add_space(6.0);
            ui.weak(tr(zh, "未找到批次分组，按独立检查展示。每条记录内可能有多种检测方式。", "No batch groups were found. Showing saved checks, each of which may use several verifiers."));
        }
        if page.records.is_empty() {
            ui.add_space(8.0);
            ui.label(tr(
                zh,
                "这个网址还没有已保存的记录。不代表链接失效。",
                "No saved records for this URL. This does not mean it is dead.",
            ));
        }
        for record in &page.records {
            ui.add_space(8.0);
            ui.push_id(url, |ui| show_record(ui, record, zh, developer));
            ui.separator();
        }
        let offset = self.desired.as_ref().map_or(0, |request| request.offset);
        let mut page_offset = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !loading && offset > 0,
                    egui::Button::new(tr(zh, "较新记录", "Newer")),
                )
                .clicked()
            {
                page_offset = Some(offset.saturating_sub(PAGE_SIZE));
            }
            ui.weak(if zh {
                format!("第 {} 页", offset / PAGE_SIZE + 1)
            } else {
                format!("Page {}", offset / PAGE_SIZE + 1)
            });
            if ui
                .add_enabled(
                    !loading && page.has_more,
                    egui::Button::new(tr(zh, "更早记录", "Older")),
                )
                .clicked()
            {
                page_offset = Some(offset.saturating_add(PAGE_SIZE));
            }
        });
        if let Some(offset) = page_offset {
            self.change_page(offset);
        }
    }
}

fn time_label(value: Option<&str>, zh: bool) -> String {
    match value {
        Some(value) => DateTime::parse_from_rfc3339(value)
            .map(|time| {
                let mut label = time
                    .with_timezone(&Local)
                    .format("%Y/%m/%d  %H:%M")
                    .to_string();
                if time.with_timezone(&chrono::Utc) > chrono::Utc::now() {
                    label.push_str(tr(zh, "（时间需核对）", " (check time)"));
                }
                label
            })
            .unwrap_or_else(|_| tr(zh, "时间格式异常", "Invalid recorded time").into()),
        None => tr(zh, "时间未记录", "Time not recorded").into(),
    }
}

fn activity_label(activity: Activity, zh: bool) -> &'static str {
    match activity {
        Activity::Fresh => tr(zh, "本轮已发起新请求", "New requests in this check"),
        Activity::Reused => tr(
            zh,
            "沿用已有结果，未重新检测",
            "Reused prior evidence, not a new check",
        ),
        Activity::Pending => tr(zh, "尚无最终结果", "No final result saved"),
        Activity::Unknown => tr(
            zh,
            "请求情况未完整记录",
            "Request activity is not fully recorded",
        ),
    }
}

fn activity_short_label(activity: Activity, zh: bool) -> &'static str {
    match activity {
        Activity::Fresh => tr(zh, "本轮检测", "New check"),
        Activity::Reused => tr(zh, "沿用结果", "Reused result"),
        Activity::Pending => tr(zh, "尚无结果", "No result yet"),
        Activity::Unknown => tr(zh, "信息未完整", "Details incomplete"),
    }
}

fn record_title(record: &HistoryRecord, zh: bool) -> String {
    if record.conflicting {
        return tr(
            zh,
            "结果不一致，需要核对",
            "Conflicting results, review needed",
        )
        .into();
    }
    if record.completed.is_none() {
        return tr(zh, "完成情况未记录", "Completion not recorded").into();
    }
    if record.completed == Some(false) {
        return tr(zh, "未完成", "Incomplete").into();
    }
    if record.rule_version.is_none() {
        return tr(
            zh,
            "检测规则未记录，需要核对",
            "Check rules not recorded, review needed",
        )
        .into();
    }
    if record.rule_version.as_deref()
        != Some(crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION)
    {
        return tr(
            zh,
            "旧规则记录，建议重新检测",
            "Older rules, recheck recommended",
        )
        .into();
    }
    record.verdict.as_deref().map_or_else(
        || tr(zh, "结果未记录", "Result not recorded").into(),
        |verdict| verdict_label(zh, verdict).to_owned(),
    )
}

fn show_record(
    ui: &mut egui::Ui,
    record: &HistoryRecord,
    zh: bool,
    developer: bool,
) -> egui::Response {
    let mut date = time_label(record.recorded_at.as_deref(), zh);
    if record.scope.as_deref() == Some("site") {
        date.push_str(tr(zh, "  站点预检", "  Site preflight"));
    }
    let title = record_title(record, zh);
    let mut text = egui::text::LayoutJob::default();
    text.append(
        &date,
        0.0,
        egui::TextFormat {
            font_id: egui::TextStyle::Small.resolve(ui.style()),
            color: ui.visuals().weak_text_color(),
            ..Default::default()
        },
    );
    text.append(
        &format!("\n{title}"),
        0.0,
        egui::TextFormat {
            font_id: egui::TextStyle::Body.resolve(ui.style()),
            color: ui.visuals().text_color(),
            ..Default::default()
        },
    );
    text.append(
        &format!("   {}", activity_short_label(record.activity, zh)),
        0.0,
        egui::TextFormat {
            font_id: egui::TextStyle::Small.resolve(ui.style()),
            color: ui.visuals().weak_text_color(),
            ..Default::default()
        },
    );
    let response = egui::CollapsingHeader::new(text)
        .id_salt(("history-record", &record.id))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.weak(activity_label(record.activity, zh));
            if record.completed != Some(true) { ui.label(tr(zh, "这条记录未标记完成，不能据此判断任务仍在运行、已取消或失败。", "This record is unfinished. It does not identify whether the task is running, cancelled or failed.")); }
            if record.partial { ui.label(tr(zh, "本批只检查了限定范围，不是全部网址。", "This batch checked a limited set, not all URLs.")); }
            if record.scope.as_deref() == Some("site") { ui.label(tr(zh, "这是站点预检，不能替代收藏页面的检查。", "This is a site preflight, not a check of the bookmarked page.")); }
            if record.raw.len() > 1 { ui.label(format!("{} {}", record.raw.len(), tr(zh, "个收藏位置使用了这条记录", "bookmark occurrences share this record"))); }
            if let Some(verdict) = &record.verdict { ui.label(format!("{}{}", tr(zh, "保存时的结果：", "Stored result: "), verdict_label(zh, verdict))); }
            if record.observations.is_empty() { ui.weak(tr(zh, "没有已保存的检测依据。", "No verifier details were saved.")); }
            for observation in &record.observations { show_observation(ui, observation, zh); }
            if developer {
                egui::CollapsingHeader::new(tr(zh, "开发者明细", "Developer details"))
                    .id_salt(("history-raw", &record.id)).show(ui, |ui| {
                        for raw in &record.raw {
                            match serde_json::to_string_pretty(raw) {
                                Ok(raw) => { ui.label(RichText::new(raw).monospace().small()); }
                                Err(error) => { ui.label(error.to_string()); }
                            }
                        }
                    });
            }
        });
    response
        .header_response
        .on_hover_text(activity_label(record.activity, zh))
}

fn verifier_label(backend: &str, zh: bool) -> &'static str {
    match backend {
        "native_reqwest" => tr(zh, "网页检测", "Page check"),
        "curl" | "curl_cli" => tr(zh, "备用复核", "Secondary verifier"),
        "same_day_resume" => tr(zh, "已有结果", "Prior result"),
        "native_dns" | "dnsx" => tr(zh, "域名检查", "Domain check"),
        "rule_policy" => tr(zh, "检测规则", "Check rules"),
        _ => tr(zh, "补充检测", "Additional verifier"),
    }
}

fn response_label(code: Option<i64>, error: Option<&str>, zh: bool) -> String {
    if error.is_some_and(|error| error.contains("SecurityBlocked")) {
        return tr(
            zh,
            "为安全起见，访问已被阻止",
            "A request was blocked for safety.",
        )
        .into();
    }
    if error.is_some_and(|error| error.contains("ContentMissing") || error.contains("Soft404")) {
        return tr(
            zh,
            "网站有响应，但目标内容可能已移除，需要核对",
            "The website responded, but the saved content may be missing. Review needed.",
        )
        .into();
    }
    if error.is_some() && code.is_some_and(|code| (200..300).contains(&code)) {
        return tr(
            zh,
            "已收到响应，但检查未能完整完成",
            "A response arrived, but the check did not complete.",
        )
        .into();
    }
    if error.is_some_and(|error| error.contains("RuleVersionMismatch")) {
        return tr(
            zh,
            "检测规则已更新，需要重新检测",
            "Check rules changed. Recheck this link.",
        )
        .into();
    }
    if let Some(code) = code {
        let meaning = match code {
            200..=299 => tr(zh, "网站有响应", "Website responded"),
            401 | 403 => tr(
                zh,
                "访问受限，暂时保留",
                "Access restricted, retain for review",
            ),
            404 | 410 => tr(
                zh,
                "本次未找到页面，需结合其他记录",
                "Page not found in this check. Compare other records.",
            ),
            429 => tr(zh, "网站要求稍后再试", "Website asked us to wait"),
            500..=599 => tr(zh, "网站暂时出错", "Temporary website error"),
            _ => tr(zh, "已保存网站响应", "Website response saved"),
        };
        return format!("{meaning} ({code})");
    }
    match error {
        Some(_) => tr(
            zh,
            "本次未能得到明确结果，暂时保留",
            "No clear result from this check. Retain for review.",
        )
        .into(),
        None => tr(zh, "未保存网站响应", "No website response saved").into(),
    }
}

fn show_observation(ui: &mut egui::Ui, observation: &Value, zh: bool) {
    ui.strong(verifier_label(
        observation["backend"].as_str().unwrap_or(""),
        zh,
    ));
    ui.label(response_label(
        observation["status_code"].as_i64(),
        observation["error_kind"].as_str(),
        zh,
    ));
    if observation["cache_hit"].as_bool() == Some(true)
        || observation["request_method"].as_str() == Some("REUSE")
    {
        ui.weak(tr(
            zh,
            "这是沿用的证据，不增加独立检测次数。",
            "This reused evidence is not another independent check.",
        ));
    }
    if let Some(time) = observation["finished_at"].as_str() {
        ui.small(format!(
            "{}{}",
            tr(zh, "证据时间：", "Evidence time: "),
            time_label(Some(time), zh)
        ));
    }
}

pub(super) fn latest_evidence(
    ui: &mut egui::Ui,
    evidence: &[crate::health::review::ReviewEvidence],
    zh: bool,
) {
    egui::CollapsingHeader::new(tr(zh, "最近一次检查的依据", "Details of the latest check"))
        .id_salt("latest-evidence")
        .show(ui, |ui| {
            for evidence in evidence {
                ui.strong(verifier_label(&evidence.backend, zh));
                ui.label(response_label(
                    evidence.status_code,
                    evidence.error_kind.as_deref(),
                    zh,
                ));
                if let Some(time) = &evidence.finished_at {
                    ui.small(time_label(Some(time), zh));
                }
                ui.add_space(6.0);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::state_machine::CURRENT_HEALTH_RULE_VERSION;

    fn record() -> HistoryRecord {
        HistoryRecord {
            id: "internal-record-should-stay-hidden".into(),
            recorded_at: Some("2026-10-29T00:12:00Z".into()),
            completed: Some(true),
            verdict: Some("live".into()),
            rule_version: Some(CURRENT_HEALTH_RULE_VERSION.into()),
            partial: false,
            scope: Some("content".into()),
            activity: Activity::Reused,
            observations: vec![
                serde_json::json!({"backend": "same_day_resume", "status_code": 200}),
            ],
            raw: vec![serde_json::json!({"scan_id": "internal-record-should-stay-hidden"})],
            conflicting: false,
        }
    }

    #[test]
    fn records_are_plain_language_collapsed_and_fit_both_themes_and_languages() {
        for dark in [false, true] {
            for zh in [false, true] {
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(300.0, 650.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        show_record(ui, &record(), zh, false);
                    },
                );
                output.textures_delta.clear();
                let nodes = output.platform_output.accesskit_update.unwrap().nodes;
                let values: Vec<_> = nodes
                    .iter()
                    .flat_map(|(_, node)| [node.value(), node.label()].into_iter().flatten())
                    .collect();
                assert!(
                    values
                        .iter()
                        .any(|value| value.contains(activity_short_label(Activity::Reused, zh))),
                    "{values:?}"
                );
                assert!(
                    !values.iter().any(|value| value.contains("internal-record-")
                        || value.contains("same_day_resume")
                        || value.contains("HTTP"))
                );
                assert!(nodes
                    .iter()
                    .filter_map(|(_, node)| node.bounds())
                    .all(|rect| rect.width() <= 300.0));
            }
        }
    }

    #[test]
    fn unfinished_old_rules_and_conflicts_do_not_present_as_current_dead_confirmation() {
        let mut item = record();
        item.verdict = Some("confirmed_dead".into());
        item.completed = Some(false);
        assert_eq!(record_title(&item, true), "未完成");
        item.completed = Some(true);
        item.rule_version = Some("old-policy".into());
        assert_eq!(record_title(&item, true), "旧规则记录，建议重新检测");
        item.conflicting = true;
        assert_eq!(
            record_title(&item, false),
            "Conflicting results, review needed"
        );
        assert_eq!(time_label(None, true), "时间未记录");
        assert_eq!(time_label(Some("broken"), false), "Invalid recorded time");
        item.conflicting = false;
        item.completed = None;
        assert_eq!(record_title(&item, false), "Completion not recorded");
        item.completed = Some(true);
        item.rule_version = None;
        assert_eq!(
            record_title(&item, false),
            "Check rules not recorded, review needed"
        );
    }

    #[test]
    fn future_times_and_incomplete_success_responses_are_not_presented_as_plain_success() {
        let future = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
        assert!(time_label(Some(&future), true).contains("时间需核对"));
        assert!(
            !response_label(Some(200), Some("BodyTruncated"), false).contains("Website responded")
        );
        assert!(response_label(Some(200), Some("ContentMissing"), true).contains("需要核对"));
    }

    fn wait_for_worker(panel: &RecordsPanel) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while panel
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }

    #[test]
    fn old_url_results_and_panics_cannot_replace_or_fail_the_new_selection() {
        for panic in [false, true] {
            let old = Request {
                db: PathBuf::from("unused"),
                url: "https://old.example.test".into(),
                offset: 0,
                generation: 1,
            };
            let new = Request {
                url: "https://new.example.test".into(),
                generation: 2,
                ..old.clone()
            };
            let mut panel = RecordsPanel {
                desired: Some(new.clone()),
                loaded: Some(new.clone()),
                page: Some(HistoryPage {
                    records: vec![record()],
                    legacy: false,
                    has_more: false,
                }),
                running: Some(old.clone()),
                ..Default::default()
            };
            panel.worker = Some(std::thread::spawn(move || {
                if panic {
                    panic!("synthetic stale history worker");
                }
                (
                    old,
                    Ok(HistoryPage {
                        records: vec![],
                        legacy: false,
                        has_more: false,
                    }),
                )
            }));
            wait_for_worker(&panel);
            panel.poll();
            assert_eq!(panel.loaded, Some(new));
            assert!(panel.error.is_none());
            assert_eq!(panel.page.as_ref().unwrap().records.len(), 1);
        }
    }

    #[test]
    fn failed_refresh_keeps_the_last_successful_page_visible() {
        let request = Request {
            db: PathBuf::from("unused"),
            url: "https://synthetic.example.test".into(),
            offset: 0,
            generation: 2,
        };
        let mut panel = RecordsPanel {
            desired: Some(request.clone()),
            page: Some(HistoryPage {
                records: vec![record()],
                legacy: false,
                has_more: false,
            }),
            running: Some(request.clone()),
            ..Default::default()
        };
        panel.worker = Some(std::thread::spawn(move || {
            (request, Err("synthetic unreadable history".into()))
        }));
        wait_for_worker(&panel);
        panel.poll();
        assert!(panel.error.is_some());
        assert_eq!(
            panel.page.as_ref().unwrap().records[0].id,
            "internal-record-should-stay-hidden"
        );
    }

    #[test]
    fn returning_to_a_previous_page_during_loading_starts_a_new_read() {
        let temp = tempfile::tempdir().unwrap();
        let first = Request {
            db: temp.path().join("not-a-database.sqlite"),
            url: "https://synthetic.example.test".into(),
            offset: 0,
            generation: 1,
        };
        let older = Request {
            offset: PAGE_SIZE,
            ..first.clone()
        };
        let mut panel = RecordsPanel {
            desired: Some(older.clone()),
            loaded: Some(first),
            running: Some(older.clone()),
            generation: 1,
            ..Default::default()
        };
        panel.worker = Some(std::thread::spawn(move || {
            (older, Err("stale older page".into()))
        }));
        panel.change_page(0);
        let desired = panel.desired.clone();
        assert_ne!(panel.loaded, desired);
        wait_for_worker(&panel);
        panel.poll();
        assert_eq!(panel.running, desired);
        assert!(panel.loaded.is_none());
        assert!(panel.error.is_none());
        // The new reader may fail, but must actually run rather than leaving an empty page.
        let (_, result) = panel.worker.take().unwrap().join().unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn expanded_record_keeps_internal_fields_behind_developer_mode() {
        for developer in [false, true] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            ctx.all_styles_mut(|style| style.animation_time = 0.0);
            let item = record();
            let mut first = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(390.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let response = show_record(ui, &item, false, developer);
                    let mut state =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            response.id,
                            true,
                        );
                    state.set_open(true);
                    state.store(ui.ctx());
                },
            );
            first.textures_delta.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(390.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    show_record(ui, &item, false, developer);
                },
            );
            output.textures_delta.clear();
            let nodes = output.platform_output.accesskit_update.unwrap().nodes;
            let values: Vec<_> = nodes
                .iter()
                .flat_map(|(_, node)| [node.value(), node.label()].into_iter().flatten())
                .collect();
            assert!(
                values
                    .iter()
                    .any(|value| value.contains("Stored result: Reachable")),
                "{values:?}"
            );
            assert_eq!(
                values
                    .iter()
                    .any(|value| value.contains("Developer details")),
                developer
            );
            assert!(!values
                .iter()
                .any(|value| value.contains("internal-record-should-stay-hidden")));
        }
    }
}
