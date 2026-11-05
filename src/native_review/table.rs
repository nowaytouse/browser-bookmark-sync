use chrono::{DateTime, Local, Utc};
use eframe::egui;

use super::{tr, verdict_label};
use crate::health::{
    review::{Decision, ReviewItem},
    state_machine::StateMachine,
};

const COLUMN_WIDTHS: [f32; 7] = [32.0, 260.0, 150.0, 120.0, 320.0, 200.0, 160.0];
const GAP: f32 = 12.0;
const PADDING: f32 = 8.0;
pub(super) const WIDTH: f32 = 1330.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HealthTone {
    Good,
    Dead,
    Review,
    Untested,
}

fn health(item: &ReviewItem, now: DateTime<Utc>) -> (HealthTone, &'static str) {
    let current = item
        .last_checked_at
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .is_some_and(|at| StateMachine::is_current_evidence(at.with_timezone(&Utc), now));
    match item.verdict.as_str() {
        "live" if current => (HealthTone::Good, "live"),
        "recovered" if current => (HealthTone::Good, "recovered"),
        "confirmed_dead" if current => (HealthTone::Dead, "confirmed_dead"),
        "probable_dead" => (HealthTone::Review, "probable_dead"),
        "needs_review" => (HealthTone::Review, "needs_review"),
        "protected" => (HealthTone::Review, "protected"),
        "temporary_failure" => (HealthTone::Review, "temporary_failure"),
        "untested" => (HealthTone::Untested, "untested"),
        _ => (HealthTone::Review, "recheck"),
    }
}

pub(super) fn is_current_dead(item: &ReviewItem, now: DateTime<Utc>) -> bool {
    health(item, now).0 == HealthTone::Dead
}

fn health_label(zh: bool, verdict: &str) -> &str {
    if verdict == "recheck" {
        tr(zh, "需重新检测", "Recheck")
    } else {
        verdict_label(zh, verdict)
    }
}

fn health_color(visuals: &egui::Visuals, tone: HealthTone) -> egui::Color32 {
    match tone {
        HealthTone::Good if visuals.dark_mode => egui::Color32::from_rgb(85, 196, 125),
        HealthTone::Good => egui::Color32::from_rgb(31, 122, 67),
        HealthTone::Dead => visuals.error_fg_color,
        HealthTone::Review => visuals.warn_fg_color,
        HealthTone::Untested => visuals.weak_text_color(),
    }
}

fn decision_label(zh: bool, decision: Option<Decision>) -> &'static str {
    match decision {
        None => tr(zh, "默认保留", "Default keep"),
        Some(Decision::Keep) => tr(zh, "保留", "Keep"),
        Some(Decision::Remove) => tr(zh, "移除", "Remove"),
    }
}

fn decision_options(ui: &mut egui::Ui, decision: &mut Option<Decision>, zh: bool) {
    for value in [None, Some(Decision::Keep), Some(Decision::Remove)] {
        let mut label = egui::RichText::new(decision_label(zh, value));
        if value == Some(Decision::Remove) {
            label = label.color(ui.visuals().error_fg_color);
        }
        if ui.selectable_value(decision, value, label).clicked() {
            ui.close();
        }
    }
}

fn column_rects(rect: egui::Rect) -> [egui::Rect; 7] {
    let mut left = rect.left() + PADDING;
    COLUMN_WIDTHS.map(|width| {
        let cell = egui::Rect::from_min_size(
            egui::pos2(left, rect.top()),
            egui::vec2(width, rect.height()),
        );
        left += width + GAP;
        cell
    })
}

fn cell_ui(ui: &mut egui::Ui, rect: egui::Rect, index: usize) -> egui::Ui {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("review_table_cell", index))
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    child
}

fn text_cell(ui: &mut egui::Ui, rect: egui::Rect, index: usize, text: &str) {
    cell_ui(ui, rect, index)
        .add_sized(
            rect.size(),
            egui::Label::new(text).halign(egui::Align::Min).truncate(),
        )
        .on_hover_text(text);
}

pub(super) fn header(ui: &mut egui::Ui, zh: bool) {
    let (_, rect) = ui.allocate_space(egui::vec2(WIDTH, 32.0));
    let labels = [
        tr(zh, "选择", "Select"),
        tr(zh, "书签", "Bookmark"),
        tr(zh, "健康状态", "Health"),
        tr(zh, "清理决定", "Decision"),
        "URL",
        tr(zh, "文件夹", "Folder"),
        tr(zh, "上次检查", "Last checked"),
    ];
    ui.push_id("review_table_header", |ui| {
        for (index, (rect, label)) in column_rects(rect).into_iter().zip(labels).enumerate() {
            cell_ui(ui, rect, index)
                .add_sized(
                    rect.size(),
                    egui::Label::new(egui::RichText::new(label).strong())
                        .halign(egui::Align::Min)
                        .truncate(),
                )
                .on_hover_text(label);
        }
    });
}

pub(super) fn row(
    ui: &mut egui::Ui,
    item: &ReviewItem,
    selected: &mut bool,
    decision: &mut Option<Decision>,
    focused: bool,
    duplicate: bool,
    zh: bool,
    developer: bool,
    row_height: f32,
    now: DateTime<Utc>,
) -> egui::Response {
    let (_, rect) = ui.allocate_space(egui::vec2(WIDTH, row_height));
    let cells = column_rects(rect);
    if focused {
        ui.painter().rect_filled(
            rect,
            4.0,
            ui.visuals().selection.bg_fill.linear_multiply(0.18),
        );
    }
    cell_ui(ui, cells[0], 0)
        .add_sized([32.0, 28.0], egui::Checkbox::without_text(selected))
        .accessible_name(format!(
            "{}: {}",
            tr(zh, "选择书签", "Select bookmark"),
            item.title
        ))
        .on_hover_text(tr(
            zh,
            "选择此书签以进行批量操作",
            "Select this bookmark for batch actions",
        ));

    let title = if item.title.is_empty() {
        tr(zh, "无标题书签", "Untitled bookmark")
    } else {
        &item.title
    };
    let mut response = cell_ui(ui, cells[1], 1).add_sized(
        [cells[1].width(), row_height - 8.0],
        egui::Button::selectable(focused, ())
            .left_text(title)
            .truncate(),
    );
    response = response.on_hover_ui(|ui| {
        ui.label(title);
        if duplicate {
            ui.label(tr(
                zh,
                "精确重复书签；健康状态独立显示。",
                "Exact duplicate; health is shown independently.",
            ));
        }
        if developer {
            ui.label(format!(
                "ID: {} · {}: {}",
                item.id,
                tr(zh, "检查次数", "Checks"),
                item.total_checks
            ));
        }
    });
    response.context_menu(|ui| {
        ui.label(title);
        ui.separator();
        decision_options(ui, decision, zh);
    });

    let (tone, verdict) = health(item, now);
    ui.painter().circle_filled(
        egui::pos2(cells[2].left() + 4.0, cells[2].center().y),
        4.0,
        health_color(ui.visuals(), tone),
    );
    let mut health_rect = cells[2];
    health_rect.min.x += 16.0;
    cell_ui(ui, health_rect, 2)
        .add_sized(health_rect.size(), egui::Label::new(health_label(zh, verdict)).halign(egui::Align::Min).truncate())
        .on_hover_ui(|ui| {
            ui.label(health_label(zh, verdict));
            ui.label(tr(zh,
                "健康状态来自已存检查证据，与保留／移除决定相互独立。绿色和红色仅表示 24 小时内的有效证据；失效候选不会自动移除。",
                "Health comes from stored check evidence, independently of Keep/Remove decisions. Green and red require valid evidence from the last 24 hours; dead candidates are not removed automatically."));
            ui.label(format!("{}: {}", tr(zh, "已存状态", "Stored state"), verdict_label(zh, &item.verdict)));
        });

    let mut decision_text = egui::RichText::new(decision_label(zh, *decision));
    if *decision == Some(Decision::Remove) {
        decision_text = decision_text.color(ui.visuals().error_fg_color);
    }
    egui::ComboBox::from_id_salt("cleanup_decision")
        .width(cells[3].width())
        .selected_text(decision_text)
        .truncate()
        .show_ui(&mut cell_ui(ui, cells[3], 3), |ui| decision_options(ui, decision, zh))
        .response
        .accessible_name(format!("{}: {}", tr(zh, "清理决定", "Cleanup decision"), title))
        .on_hover_text(tr(zh,
            "仅改变清理决定，不改变健康状态。默认保留表示尚未明确选择；未检查或存疑的书签仍会保留。",
            "Changes only the cleanup decision, not health. Default keep means no explicit choice; untested or uncertain bookmarks remain retained."));
    text_cell(ui, cells[4], 4, &item.url);
    let folder = item.path.join(" / ");
    text_cell(ui, cells[5], 5, &folder);
    let checked = match item.last_checked_at.as_deref() {
        None => tr(zh, "未检查", "Not checked").to_string(),
        Some(at) => DateTime::parse_from_rfc3339(at).map_or_else(
            |_| tr(zh, "时间无效", "Invalid time").to_string(),
            |at| {
                at.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            },
        ),
    };
    cell_ui(ui, cells[6], 6)
        .add_sized(
            cells[6].size(),
            egui::Label::new(&checked)
                .halign(egui::Align::Min)
                .truncate(),
        )
        .on_hover_text(item.last_checked_at.as_deref().unwrap_or(&checked));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(verdict: &str, at: Option<String>) -> ReviewItem {
        ReviewItem {
            id: 7,
            title: "A long bookmark / 很长的书签标题".repeat(8),
            url: "https://example.invalid/a/long/path".repeat(8),
            path: vec!["Synthetic folder / 合成文件夹".repeat(8)],
            verdict: verdict.into(),
            last_checked_at: at,
            total_checks: 3,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn health_requires_current_evidence() {
        let now = DateTime::parse_from_rfc3339("2026-11-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for (verdict, tone) in [
            ("live", HealthTone::Good),
            ("recovered", HealthTone::Good),
            ("confirmed_dead", HealthTone::Dead),
        ] {
            for (age, current) in [(0, true), (24, true), (25, false), (-1, false)] {
                let entry = item(
                    verdict,
                    Some((now - chrono::Duration::hours(age)).to_rfc3339()),
                );
                let expected = if current {
                    (tone, verdict)
                } else {
                    (HealthTone::Review, "recheck")
                };
                assert_eq!(health(&entry, now), expected);
            }
            for at in [None, Some("invalid".into())] {
                assert_eq!(
                    health(&item(verdict, at), now),
                    (HealthTone::Review, "recheck")
                );
            }
        }
        for verdict in [
            "probable_dead",
            "needs_review",
            "protected",
            "temporary_failure",
        ] {
            assert_eq!(
                health(&item(verdict, None), now),
                (HealthTone::Review, verdict)
            );
        }
        assert_eq!(
            health(&item("untested", None), now),
            (HealthTone::Untested, "untested")
        );
        assert_eq!(
            health(&item("unknown", Some(now.to_rfc3339())), now),
            (HealthTone::Review, "recheck")
        );
    }

    #[test]
    fn table_geometry_and_accessible_targets_match_in_both_languages_and_themes() {
        for dark in [false, true] {
            for zh in [false, true] {
                for row_height in [40.0, 44.0] {
                    for initial_decision in [None, Some(Decision::Keep), Some(Decision::Remove)] {
                        let ctx = egui::Context::default();
                        ctx.enable_accesskit();
                        ctx.set_visuals(if dark {
                            egui::Visuals::dark()
                        } else {
                            egui::Visuals::light()
                        });
                        let now = Utc::now();
                        let entry = item("live", Some(now.to_rfc3339()));
                        let mut selected = false;
                        let mut decision = initial_decision;
                        let mut output = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(WIDTH + 40.0, 400.0),
                                )),
                                ..Default::default()
                            },
                            |ui| {
                                let header_top = ui.cursor().top();
                                let header_rects = column_rects(egui::Rect::from_min_size(
                                    ui.cursor().min,
                                    egui::vec2(WIDTH, 32.0),
                                ));
                                header(ui, zh);
                                assert!(ui.cursor().top() > header_top);
                                let before = ui.cursor().top();
                                let cells = column_rects(egui::Rect::from_min_size(
                                    ui.cursor().min,
                                    egui::vec2(WIDTH, row_height),
                                ));
                                let response = ui
                                    .push_id(entry.id, |ui| {
                                        row(
                                            ui,
                                            &entry,
                                            &mut selected,
                                            &mut decision,
                                            true,
                                            true,
                                            zh,
                                            true,
                                            row_height,
                                            now,
                                        )
                                    })
                                    .inner;
                                assert!((response.rect.left() - cells[1].left()).abs() < 0.5);
                                assert!(response.rect.right() <= cells[1].right() + 0.5);
                                assert!(response.rect.bottom() <= before + row_height);
                                assert!(
                                    (ui.cursor().top()
                                        - before
                                        - row_height
                                        - ui.spacing().item_spacing.y)
                                        .abs()
                                        < 0.5
                                );
                                for index in 0..COLUMN_WIDTHS.len() {
                                    assert_eq!(
                                        cells[index].x_range(),
                                        header_rects[index].x_range()
                                    );
                                    assert_eq!(cells[index].width(), COLUMN_WIDTHS[index]);
                                }
                                assert_eq!(
                                    cells[6].right() + PADDING - cells[0].left() + PADDING,
                                    WIDTH
                                );
                            },
                        );
                        output.textures_delta.clear();
                        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
                        let (_, checkbox) = nodes
                            .iter()
                            .find(|(_, node)| node.role() == egui::accesskit::Role::CheckBox)
                            .unwrap();
                        let bounds = checkbox.bounds().unwrap();
                        assert!(bounds.width() >= 24.0 && bounds.height() >= 24.0);
                        assert!(checkbox
                            .label()
                            .is_some_and(|label| label.contains(&entry.title)));
                        assert!(nodes
                            .iter()
                            .any(|(_, node)| node.label().is_some_and(
                                |label| label.starts_with(tr(zh, "清理决定", "Cleanup decision"))
                            )));
                        assert_eq!(decision, initial_decision);
                        assert!(!selected);
                        assert_eq!(entry.verdict, "live");
                        assert_eq!(health(&entry, now), (HealthTone::Good, "live"));
                    }
                }
            }
        }
    }

    #[test]
    fn scrolled_header_stays_aligned_with_virtual_rows_and_viewport_scrollbars() {
        let now = Utc::now();
        let entries: Vec<_> = (0..30)
            .map(|id| {
                let mut entry = item("live", Some(now.to_rfc3339()));
                entry.id = id;
                entry
            })
            .collect();
        let original_selection: Vec<_> = (0..entries.len()).map(|id| id == 1).collect();
        let original_decisions: Vec<_> = (0..entries.len())
            .map(|id| [None, Some(Decision::Keep), Some(Decision::Remove)][id % 3])
            .collect();
        let mut header_lefts = Vec::new();
        for offset in [0.0, 220.0] {
            let ctx = egui::Context::default();
            let mut selected = original_selection.clone();
            let mut decisions = original_decisions.clone();
            let mut geometry = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(700.0, 360.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let (_, header_slot) =
                        ui.allocate_space(egui::vec2(ui.available_width(), 32.0));
                    let viewport = ui.available_rect_before_wrap();
                    let spacing = ui.spacing().item_spacing.y;
                    let mut rows = Vec::new();
                    let mut content_left = header_slot.left();
                    let body = egui::ScrollArea::both()
                        .id_salt("table-scroll-regression")
                        .horizontal_scroll_offset(offset)
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                        )
                        .auto_shrink([false, false])
                        .show_rows(ui, 44.0, entries.len(), |ui, range| {
                            content_left = ui.max_rect().left();
                            ui.set_width(WIDTH);
                            for id in range {
                                let cells = column_rects(egui::Rect::from_min_size(
                                    ui.cursor().min,
                                    egui::vec2(WIDTH, 44.0),
                                ));
                                let response = ui
                                    .push_id(id, |ui| {
                                        row(
                                            ui,
                                            &entries[id],
                                            &mut selected[id],
                                            &mut decisions[id],
                                            false,
                                            false,
                                            false,
                                            false,
                                            44.0,
                                            now,
                                        )
                                    })
                                    .inner;
                                rows.push((cells, response.rect));
                            }
                        });
                    let header_rect = egui::Rect::from_min_size(
                        egui::pos2(content_left, header_slot.top()),
                        egui::vec2(WIDTH, header_slot.height()),
                    );
                    let header_cells = column_rects(header_rect);
                    let mut header_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt("fixed-table-header")
                            .max_rect(header_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    header_ui.set_clip_rect(ui.clip_rect().intersect(header_slot));
                    header(&mut header_ui, false);
                    geometry = Some((
                        header_cells,
                        rows,
                        body.inner_rect,
                        body.state.unclamped_offset().x,
                        body.content_size.y,
                        viewport,
                        spacing,
                    ));
                },
            );
            output.textures_delta.clear();
            let (header_cells, rows, body_rect, actual_offset, content_height, viewport, spacing) =
                geometry.unwrap();
            assert!((actual_offset - offset).abs() < 0.5);
            assert!(body_rect.right() <= viewport.right() + 0.5);
            assert!(body_rect.bottom() <= viewport.bottom() + 0.5);
            assert!(content_height > body_rect.height());
            assert!(rows.len() > 1 && rows.len() < entries.len());
            for (cells, title) in &rows {
                for index in 0..COLUMN_WIDTHS.len() {
                    assert!((cells[index].left() - header_cells[index].left()).abs() < 0.5);
                    assert_eq!(cells[index].width(), header_cells[index].width());
                }
                assert!((title.left() - header_cells[1].left()).abs() < 0.5);
            }
            for pair in rows.windows(2) {
                assert!((pair[1].1.top() - pair[0].1.top() - 44.0 - spacing).abs() < 0.5);
            }
            assert_eq!(selected, original_selection);
            assert_eq!(decisions, original_decisions);
            header_lefts.push(header_cells[0].left());
        }
        assert!((header_lefts[0] - header_lefts[1] - 220.0).abs() < 0.5);
    }
}
