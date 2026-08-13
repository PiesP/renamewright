#![forbid(unsafe_code)]

// Hallmark · pre-emit critique: P5 H5 E4 S5 R5 V4
// Hallmark · macrostructure: workbench · theme: Cobalt · slop: pass (native-app scope)

use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Layout, RichText, ScrollArea, Stroke,
};

const SAMPLE_COUNT: usize = 10_000;
const PREVIEW_ROW_HEIGHT: f32 = 28.0;

pub mod semantics {
    pub const PRODUCT_NAME: &str = "Renamewright";
    pub const TAGLINE: &str = "Plan every rename.";
    pub const ADD_FOLDER: &str = "Add folder";
    pub const ADD_FILES: &str = "Add files";
    pub const RULES_HEADING: &str = "Rules";
    pub const RULES_ORDER_HELP: &str = "Applied in order";
    pub const RULE_PREFIX: &str = "Prefix";
    pub const RULE_SEQUENCE: &str = "Sequence";
    pub const RULE_EXTENSION: &str = "Extension";
    pub const PREFIX_LABEL: &str = "Prefix text";
    pub const HANGUL_IME_HELP: &str = "한글 IME 입력 확인";
    pub const PREVIEW_HEADING: &str = "Preview";
    pub const FILTER_ALL: &str = "All";
    pub const FILTER_CHANGED: &str = "Changed";
    pub const FILTER_BLOCKED: &str = "Blocked";
    pub const SOURCE_QUERY_LABEL: &str = "Filter names";
    pub const APPLY: &str = "Apply";
    pub const APPLY_LOCKED: &str = "Apply locked";
    pub const AUTOMATION_BANNER: &str = "AUTOMATION TEST MODE";
    pub const HIGH_CONTRAST_ACTIVE: &str = "Windows high contrast palette active";
}

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
pub struct NativePalette {
    paper: Color32,
    paper_raised: Color32,
    paper_soft: Color32,
    ink: Color32,
    ink_soft: Color32,
    rule: Color32,
    accent: Color32,
    accent_fill: Color32,
    accent_soft: Color32,
    accent_text: Color32,
    blocked: Color32,
    disabled: Color32,
    high_contrast: bool,
}

impl Default for NativePalette {
    fn default() -> Self {
        Self {
            paper: PAPER,
            paper_raised: PAPER_RAISED,
            paper_soft: PAPER_SOFT,
            ink: INK,
            ink_soft: INK_SOFT,
            rule: RULE,
            accent: ACCENT,
            accent_fill: ACCENT,
            accent_soft: ACCENT_SOFT,
            accent_text: Color32::WHITE,
            blocked: BLOCKED,
            disabled: INK_SOFT,
            high_contrast: false,
        }
    }
}

impl NativePalette {
    #[must_use]
    pub fn high_contrast(
        window: [u8; 3],
        window_text: [u8; 3],
        highlight: [u8; 3],
        highlight_text: [u8; 3],
        gray_text: [u8; 3],
    ) -> Self {
        let window = Color32::from_rgb(window[0], window[1], window[2]);
        let window_text = Color32::from_rgb(window_text[0], window_text[1], window_text[2]);
        let highlight = Color32::from_rgb(highlight[0], highlight[1], highlight[2]);
        let highlight_text =
            Color32::from_rgb(highlight_text[0], highlight_text[1], highlight_text[2]);
        let gray_text = Color32::from_rgb(gray_text[0], gray_text[1], gray_text[2]);
        Self {
            paper: window,
            paper_raised: window,
            paper_soft: window,
            ink: window_text,
            ink_soft: window_text,
            rule: window_text,
            accent: window_text,
            accent_fill: highlight,
            accent_soft: highlight,
            accent_text: highlight_text,
            blocked: window_text,
            disabled: gray_text,
            high_contrast: true,
        }
    }

    #[must_use]
    pub const fn is_high_contrast(self) -> bool {
        self.high_contrast
    }
}

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
            Self::All => semantics::FILTER_ALL,
            Self::Changed => semantics::FILTER_CHANGED,
            Self::Blocked => semantics::FILTER_BLOCKED,
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
    palette: NativePalette,
    #[cfg(feature = "automation")]
    automation_mode: bool,
}

impl NativeSpikeApp {
    #[must_use]
    pub fn new(_automation_mode: bool) -> Self {
        Self::new_with_palette(_automation_mode, NativePalette::default())
    }

    #[must_use]
    pub fn new_with_palette(_automation_mode: bool, palette: NativePalette) -> Self {
        Self {
            prefix: "정리_".to_owned(),
            source_query: String::new(),
            filter: PlanFilter::All,
            selected_rule: 0,
            status: format!("{SAMPLE_COUNT} sample entries ready"),
            palette,
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
            ui.heading(RichText::new(semantics::PRODUCT_NAME).color(self.palette.ink));
            ui.label(RichText::new(semantics::TAGLINE).color(self.palette.ink_soft));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(semantics::ADD_FOLDER).clicked() {
                    self.status = match rfd::FileDialog::new()
                        .set_title("Add a directory entry to Renamewright")
                        .pick_folder()
                    {
                        Some(_) => "One directory entry selected for the spike".to_owned(),
                        None => "Directory selection cancelled".to_owned(),
                    };
                }
                if ui.button(semantics::ADD_FILES).clicked() {
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
        ui.heading(RichText::new(semantics::RULES_HEADING).color(self.palette.ink));
        ui.label(RichText::new(semantics::RULES_ORDER_HELP).color(self.palette.ink_soft));
        ui.add_space(8.0);

        for (index, label) in [
            semantics::RULE_PREFIX,
            semantics::RULE_SEQUENCE,
            semantics::RULE_EXTENSION,
        ]
        .iter()
        .enumerate()
        {
            let selected = self.selected_rule == index;
            if ui.selectable_label(selected, *label).clicked() {
                self.selected_rule = index;
            }
        }

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(12.0);
        let prefix_label = ui.label(
            RichText::new(semantics::PREFIX_LABEL)
                .strong()
                .color(self.palette.ink),
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.prefix)
                .id_salt("rule.prefix.value")
                .hint_text(semantics::RULE_PREFIX),
        )
        .labelled_by(prefix_label.id);
        ui.label(RichText::new(semantics::HANGUL_IME_HELP).color(self.palette.ink_soft));
    }

    fn show_preview(&mut self, ui: &mut egui::Ui) {
        let visible = self.visible_indices();
        ui.horizontal(|ui| {
            ui.heading(RichText::new(semantics::PREVIEW_HEADING).color(self.palette.ink));
            ui.label(
                RichText::new(format!("{} shown", visible.len())).color(self.palette.ink_soft),
            );
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
            let source_query_label = ui.label(semantics::SOURCE_QUERY_LABEL);
            ui.add(
                egui::TextEdit::singleline(&mut self.source_query)
                    .id_salt("preview.source-query")
                    .hint_text("Name contains"),
            )
            .labelled_by(source_query_label.id);
        });
        ui.add_space(8.0);

        egui::Frame::new()
            .fill(self.palette.paper_raised)
            .stroke(Stroke::new(1.0, self.palette.rule))
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [210.0, 20.0],
                        egui::Label::new(RichText::new("Source").strong().color(self.palette.ink)),
                    );
                    ui.add_sized(
                        [250.0, 20.0],
                        egui::Label::new(
                            RichText::new("Proposed").strong().color(self.palette.ink),
                        ),
                    );
                    ui.label(RichText::new("Status").strong().color(self.palette.ink));
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
                                    let color = if blocked {
                                        self.palette.blocked
                                    } else {
                                        self.palette.accent
                                    };
                                    ui.label(RichText::new(status).color(color).strong());
                                });
                            });
                        }
                    });
            });
    }

    fn show_review_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.status).color(self.palette.ink_soft));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let apply_text = if self.palette.high_contrast {
                    RichText::new(semantics::APPLY).color(self.palette.disabled)
                } else {
                    RichText::new(semantics::APPLY)
                };
                ui.add_enabled(false, egui::Button::new(apply_text))
                    .on_disabled_hover_text("The native spike never mutates the filesystem");
                ui.label(
                    RichText::new(semantics::APPLY_LOCKED)
                        .color(self.palette.blocked)
                        .strong(),
                );
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
                .frame(
                    egui::Frame::new()
                        .fill(if self.palette.high_contrast {
                            self.palette.accent_fill
                        } else {
                            self.palette.blocked
                        })
                        .inner_margin(6.0),
                )
                .show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new(semantics::AUTOMATION_BANNER)
                                .color(self.palette.accent_text)
                                .strong(),
                        );
                    });
                });
        }

        if self.palette.high_contrast {
            egui::Panel::top("high-contrast-status")
                .frame(
                    egui::Frame::new()
                        .fill(self.palette.paper_raised)
                        .stroke(Stroke::new(2.0, self.palette.rule))
                        .inner_margin(6.0),
                )
                .show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new(semantics::HIGH_CONTRAST_ACTIVE)
                                .color(self.palette.ink)
                                .strong(),
                        );
                    });
                });
        }

        egui::Panel::top("source-bar")
            .frame(
                egui::Frame::new()
                    .fill(self.palette.paper_raised)
                    .inner_margin(12.0),
            )
            .show(ui, |ui| self.show_source_bar(ui));

        egui::Panel::left("rule-rail")
            .resizable(true)
            .default_size(220.0)
            .min_size(180.0)
            .frame(
                egui::Frame::new()
                    .fill(self.palette.paper_soft)
                    .inner_margin(12.0),
            )
            .show(ui, |ui| self.show_rule_rail(ui));

        egui::Panel::bottom("review-bar")
            .frame(
                egui::Frame::new()
                    .fill(self.palette.paper_raised)
                    .stroke(Stroke::new(1.0, self.palette.rule))
                    .inner_margin(12.0),
            )
            .show(ui, |ui| self.show_review_bar(ui));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(self.palette.paper)
                    .inner_margin(12.0),
            )
            .show(ui, |ui| self.show_preview(ui));
    }
}

impl eframe::App for NativeSpikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

pub fn install_theme(ctx: &egui::Context, palette: NativePalette) {
    if !palette.high_contrast {
        ctx.set_theme(egui::Theme::Light);
        let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
        style.visuals = egui::Visuals::light();
        style.visuals.panel_fill = palette.paper;
        style.visuals.window_fill = palette.paper_raised;
        style.visuals.selection.bg_fill = palette.accent_soft;
        style.visuals.selection.stroke = Stroke::new(1.0, palette.accent);
        style.visuals.widgets.inactive.fg_stroke.color = palette.ink;
        style.visuals.widgets.hovered.bg_fill = palette.accent_soft;
        style.visuals.widgets.hovered.fg_stroke.color = palette.ink;
        style.visuals.widgets.active.bg_fill = palette.accent_fill;
        style.visuals.widgets.active.fg_stroke.color = palette.accent_text;
        style.text_styles.insert(
            egui::TextStyle::Heading,
            FontId::new(22.0, FontFamily::Proportional),
        );
        ctx.set_style_of(egui::Theme::Light, style);
        return;
    }

    let theme = if u16::from(palette.paper.r())
        + u16::from(palette.paper.g())
        + u16::from(palette.paper.b())
        < 384
    {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(theme);
    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = if theme == egui::Theme::Dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    style.visuals.dark_mode = theme == egui::Theme::Dark;
    style.visuals.override_text_color = None;
    style.visuals.weak_text_color = Some(palette.ink_soft);
    style.visuals.panel_fill = palette.paper;
    style.visuals.window_fill = palette.paper_raised;
    style.visuals.window_stroke = Stroke::new(1.0, palette.rule);
    style.visuals.faint_bg_color = palette.paper;
    style.visuals.extreme_bg_color = palette.paper_raised;
    style.visuals.text_edit_bg_color = Some(palette.paper_raised);
    style.visuals.selection.bg_fill = palette.accent_soft;
    style.visuals.selection.stroke = Stroke::new(1.0, palette.accent_text);
    style.visuals.hyperlink_color = palette.accent;
    style.visuals.warn_fg_color = palette.blocked;
    style.visuals.error_fg_color = palette.blocked;
    style.visuals.widgets.noninteractive.bg_fill = palette.paper;
    style.visuals.widgets.noninteractive.weak_bg_fill = palette.paper;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.rule);
    style.visuals.widgets.noninteractive.fg_stroke.color = palette.ink;
    style.visuals.widgets.inactive.bg_fill = palette.paper_raised;
    style.visuals.widgets.inactive.weak_bg_fill = palette.paper_raised;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.rule);
    style.visuals.widgets.inactive.fg_stroke.color = palette.ink;
    style.visuals.widgets.hovered.bg_fill = palette.accent_soft;
    style.visuals.widgets.hovered.weak_bg_fill = palette.accent_soft;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(2.0, palette.accent_text);
    style.visuals.widgets.hovered.fg_stroke.color = palette.accent_text;
    style.visuals.widgets.active.bg_fill = palette.accent_fill;
    style.visuals.widgets.active.weak_bg_fill = palette.accent_fill;
    style.visuals.widgets.active.bg_stroke = Stroke::new(2.0, palette.accent_text);
    style.visuals.widgets.active.fg_stroke.color = palette.accent_text;
    style.visuals.widgets.open = style.visuals.widgets.active;
    style.visuals.disabled_alpha = 1.0;
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(22.0, FontFamily::Proportional),
    );
    ctx.set_style_of(theme, style);
}

#[cfg(test)]
mod tests {
    use eframe::egui;
    use egui_kittest::Harness;
    use kittest::{NodeT as _, Queryable as _};

    use super::{NativePalette, NativeSpikeApp, install_theme, semantics};

    #[test]
    fn accesskit_exposes_primary_workbench_controls() {
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), NativeSpikeApp::new(false));

        let add_files = harness.get_by_label(semantics::ADD_FILES);
        let add_folder = harness.get_by_label(semantics::ADD_FOLDER);
        let prefix = harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, semantics::PREFIX_LABEL);
        harness.get_by_label(semantics::HANGUL_IME_HELP);
        let source_query = harness.get_by_role_and_label(
            egui::accesskit::Role::TextInput,
            semantics::SOURCE_QUERY_LABEL,
        );
        let apply = harness.get_by_label(semantics::APPLY);
        assert_eq!(
            add_files.accesskit_node().role(),
            egui::accesskit::Role::Button
        );
        assert_eq!(
            add_folder.accesskit_node().role(),
            egui::accesskit::Role::Button
        );
        assert_eq!(
            prefix.accesskit_node().role(),
            egui::accesskit::Role::TextInput
        );
        assert_eq!(
            source_query.accesskit_node().role(),
            egui::accesskit::Role::TextInput
        );
        assert_eq!(apply.accesskit_node().role(), egui::accesskit::Role::Button);
        assert!(apply.accesskit_node().is_disabled());
    }

    #[test]
    fn blocked_filter_updates_the_accessible_result_count() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), NativeSpikeApp::new(false));

        harness.get_by_label(semantics::FILTER_BLOCKED).click();
        harness.run_ok();
        harness.get_by_label("10 shown");
    }

    #[test]
    fn high_contrast_palette_is_visible_and_accessible() {
        let palette = NativePalette::high_contrast(
            [0, 0, 0],
            [255, 255, 255],
            [255, 255, 0],
            [0, 0, 0],
            [0, 255, 0],
        );
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                NativeSpikeApp::new_with_palette(false, palette),
            );

        assert!(palette.is_high_contrast());
        harness.get_by_label(semantics::HIGH_CONTRAST_ACTIVE);
        let apply = harness.get_by_label(semantics::APPLY);
        assert!(apply.accesskit_node().is_disabled());
    }

    #[test]
    fn high_contrast_theme_uses_supplied_system_colors_without_fading_disabled_controls() {
        let context = egui::Context::default();
        let palette = NativePalette::high_contrast(
            [0, 0, 0],
            [255, 255, 255],
            [255, 255, 0],
            [0, 0, 0],
            [0, 255, 0],
        );
        install_theme(&context, palette);

        let style = context.style_of(egui::Theme::Dark);
        assert!(style.visuals.dark_mode);
        assert_eq!(style.visuals.panel_fill, egui::Color32::BLACK);
        assert_eq!(
            style.visuals.widgets.noninteractive.fg_stroke.color,
            egui::Color32::WHITE
        );
        assert_eq!(
            style.visuals.selection.bg_fill,
            egui::Color32::from_rgb(255, 255, 0)
        );
        assert_eq!(style.visuals.selection.stroke.color, egui::Color32::BLACK);
        assert_eq!(style.visuals.disabled_alpha, 1.0);
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_build_has_a_visible_mode_banner() {
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), NativeSpikeApp::new(true));

        harness.get_by_label(semantics::AUTOMATION_BANNER);
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
