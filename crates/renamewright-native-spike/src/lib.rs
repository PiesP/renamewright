#![forbid(unsafe_code)]

// Hallmark · pre-emit critique: P5 H5 E4 S5 R5 V4
// Hallmark · macrostructure: workbench · theme: Cobalt · slop: pass (native-app scope)

use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Layout, RichText, ScrollArea, Stroke,
};

const SAMPLE_COUNT: usize = 10_000;
const PREVIEW_ROW_HEIGHT: f32 = 28.0;

const PAPER: Color32 = Color32::from_rgb(247, 248, 252);
const PAPER_RAISED: Color32 = Color32::from_rgb(253, 253, 254);
const PAPER_SOFT: Color32 = Color32::from_rgb(234, 238, 248);
const INK: Color32 = Color32::from_rgb(28, 35, 55);
const INK_SOFT: Color32 = Color32::from_rgb(72, 82, 108);
const RULE: Color32 = Color32::from_rgb(199, 207, 226);
const ACCENT: Color32 = Color32::from_rgb(42, 75, 183);
const ACCENT_SOFT: Color32 = Color32::from_rgb(222, 230, 255);
const BLOCKED: Color32 = Color32::from_rgb(166, 45, 48);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanFilter {
    All,
    Changed,
    Blocked,
}

impl PlanFilter {
    const ALL: [Self; 3] = [Self::All, Self::Changed, Self::Blocked];

    const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Changed => "Changed",
            Self::Blocked => "Blocked",
        }
    }
}

#[derive(Debug)]
pub struct NativeSpikeApp {
    prefix: String,
    source_query: String,
    filter: PlanFilter,
    selected_rule: usize,
    status: String,
    #[cfg(feature = "automation")]
    automation_mode: bool,
}

impl NativeSpikeApp {
    #[must_use]
    pub fn new(_automation_mode: bool) -> Self {
        Self {
            prefix: "정리_".to_owned(),
            source_query: String::new(),
            filter: PlanFilter::All,
            selected_rule: 0,
            status: format!("{SAMPLE_COUNT} sample entries ready"),
            #[cfg(feature = "automation")]
            automation_mode: _automation_mode,
        }
    }

    fn row_is_blocked(index: usize) -> bool {
        index > 0 && index.is_multiple_of(997)
    }

    fn row_is_visible(&self, index: usize) -> bool {
        let blocked = Self::row_is_blocked(index);
        let matches_filter = match self.filter {
            PlanFilter::All | PlanFilter::Changed => true,
            PlanFilter::Blocked => blocked,
        };
        let matches_query = self.source_query.trim().is_empty()
            || format!("IMG_{index:05}.jpg")
                .to_ascii_lowercase()
                .contains(&self.source_query.trim().to_ascii_lowercase());
        matches_filter && matches_query
    }

    fn visible_indices(&self) -> Vec<usize> {
        (0..SAMPLE_COUNT)
            .filter(|index| self.row_is_visible(*index))
            .collect()
    }

    fn show_source_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Renamewright").color(INK));
            ui.label(RichText::new("Plan every rename.").color(INK_SOFT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Add folder").clicked() {
                    self.status = match rfd::FileDialog::new()
                        .set_title("Add a directory entry to Renamewright")
                        .pick_folder()
                    {
                        Some(_) => "One directory entry selected for the spike".to_owned(),
                        None => "Directory selection cancelled".to_owned(),
                    };
                }
                if ui.button("Add files").clicked() {
                    self.status = match rfd::FileDialog::new()
                        .set_title("Add files to Renamewright")
                        .pick_files()
                    {
                        Some(paths) => {
                            format!("{} file entries selected for the spike", paths.len())
                        }
                        None => "File selection cancelled".to_owned(),
                    };
                }
            });
        });
    }

    fn show_rule_rail(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Rules").color(INK));
        ui.label(RichText::new("Applied in order").color(INK_SOFT));
        ui.add_space(8.0);

        for (index, label) in ["Prefix", "Sequence", "Extension"].iter().enumerate() {
            let selected = self.selected_rule == index;
            if ui.selectable_label(selected, *label).clicked() {
                self.selected_rule = index;
            }
        }

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(12.0);
        ui.label(RichText::new("Prefix text").strong().color(INK));
        ui.add(
            egui::TextEdit::singleline(&mut self.prefix)
                .id_salt("rule.prefix.value")
                .hint_text("Prefix"),
        );
        ui.label(RichText::new("한글 IME 입력 확인").color(INK_SOFT));
    }

    fn show_preview(&mut self, ui: &mut egui::Ui) {
        let visible = self.visible_indices();
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Preview").color(INK));
            ui.label(RichText::new(format!("{} shown", visible.len())).color(INK_SOFT));
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for candidate in PlanFilter::ALL {
                if ui
                    .selectable_label(self.filter == candidate, candidate.label())
                    .clicked()
                {
                    self.filter = candidate;
                }
            }
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut self.source_query)
                    .id_salt("preview.source-query")
                    .hint_text("Filter names"),
            );
        });
        ui.add_space(8.0);

        egui::Frame::new()
            .fill(PAPER_RAISED)
            .stroke(Stroke::new(1.0, RULE))
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [210.0, 20.0],
                        egui::Label::new(RichText::new("Source").strong().color(INK)),
                    );
                    ui.add_sized(
                        [250.0, 20.0],
                        egui::Label::new(RichText::new("Proposed").strong().color(INK)),
                    );
                    ui.label(RichText::new("Status").strong().color(INK));
                });
                ui.separator();
                ScrollArea::vertical()
                    .id_salt("preview.rows")
                    .auto_shrink([false, false])
                    .show_rows(ui, PREVIEW_ROW_HEIGHT, visible.len(), |ui, row_range| {
                        for visible_row in row_range {
                            let index = visible[visible_row];
                            let source = format!("IMG_{index:05}.jpg");
                            let proposed = format!("{}{source}", self.prefix);
                            let blocked = Self::row_is_blocked(index);
                            ui.push_id(index, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_sized([210.0, 20.0], egui::Label::new(source));
                                    ui.add_sized([250.0, 20.0], egui::Label::new(proposed));
                                    let status = if blocked { "Blocked" } else { "Changed" };
                                    let color = if blocked { BLOCKED } else { ACCENT };
                                    ui.label(RichText::new(status).color(color).strong());
                                });
                            });
                        }
                    });
            });
    }

    fn show_review_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.status).color(INK_SOFT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled(false, egui::Button::new("Apply"))
                    .on_disabled_hover_text("The native spike never mutates the filesystem");
                ui.label(RichText::new("Apply locked").color(BLOCKED).strong());
            });
        });
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let dropped_count = ui.ctx().input(|input| input.raw.dropped_files.len());
        if dropped_count > 0 {
            self.status = format!("{dropped_count} dropped entries observed by the native shell");
        }

        #[cfg(feature = "automation")]
        if self.automation_mode {
            egui::Panel::top("automation-banner")
                .frame(egui::Frame::new().fill(BLOCKED).inner_margin(6.0))
                .show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("AUTOMATION TEST MODE")
                                .color(Color32::WHITE)
                                .strong(),
                        );
                    });
                });
        }

        egui::Panel::top("source-bar")
            .frame(egui::Frame::new().fill(PAPER_RAISED).inner_margin(12.0))
            .show(ui, |ui| self.show_source_bar(ui));

        egui::Panel::left("rule-rail")
            .resizable(true)
            .default_size(220.0)
            .min_size(180.0)
            .frame(egui::Frame::new().fill(PAPER_SOFT).inner_margin(12.0))
            .show(ui, |ui| self.show_rule_rail(ui));

        egui::Panel::bottom("review-bar")
            .frame(
                egui::Frame::new()
                    .fill(PAPER_RAISED)
                    .stroke(Stroke::new(1.0, RULE))
                    .inner_margin(12.0),
            )
            .show(ui, |ui| self.show_review_bar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PAPER).inner_margin(12.0))
            .show(ui, |ui| self.show_preview(ui));
    }
}

impl eframe::App for NativeSpikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

pub fn install_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = PAPER;
    style.visuals.window_fill = PAPER_RAISED;
    style.visuals.selection.bg_fill = ACCENT_SOFT;
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    style.visuals.widgets.inactive.fg_stroke.color = INK;
    style.visuals.widgets.hovered.bg_fill = ACCENT_SOFT;
    style.visuals.widgets.hovered.fg_stroke.color = INK;
    style.visuals.widgets.active.bg_fill = ACCENT;
    style.visuals.widgets.active.fg_stroke.color = Color32::WHITE;
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(22.0, FontFamily::Proportional),
    );
    ctx.set_style_of(egui::Theme::Light, style);
}

#[cfg(test)]
mod tests {
    use eframe::egui;
    use egui_kittest::Harness;
    use kittest::{NodeT as _, Queryable as _};

    use super::NativeSpikeApp;

    #[test]
    fn accesskit_exposes_primary_workbench_controls() {
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), NativeSpikeApp::new(false));

        harness.get_by_label("Add files");
        harness.get_by_label("Add folder");
        harness.get_by_label("Prefix text");
        harness.get_by_label("한글 IME 입력 확인");
        let apply = harness.get_by_label("Apply");
        assert!(apply.accesskit_node().is_disabled());
    }

    #[test]
    fn blocked_filter_updates_the_accessible_result_count() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), NativeSpikeApp::new(false));

        harness.get_by_label("Blocked").click();
        harness.run_ok();
        harness.get_by_label("10 shown");
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_build_has_a_visible_mode_banner() {
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), NativeSpikeApp::new(true));

        harness.get_by_label("AUTOMATION TEST MODE");
    }

    #[test]
    fn ten_thousand_entry_preview_keeps_the_accessibility_tree_bounded() {
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), NativeSpikeApp::new(false));

        assert!(harness.query_by_label("IMG_00000.jpg").is_some());
        assert!(harness.query_by_label("IMG_09999.jpg").is_none());
        assert!(harness.query_all_by(|_| true).count() < 500);
    }
}
