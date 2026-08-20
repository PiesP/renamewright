#![forbid(unsafe_code)]

// Hallmark · pre-emit critique: P5 H5 E5 S5 R5 V4
// Hallmark · macrostructure: direct-command workbench · theme: Cobalt · slop: pass (native-app scope)

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use eframe::egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use eframe::egui::{
    self, Align, Color32, FontData, FontDefinitions, FontFamily, FontId, Layout, RichText,
    ScrollArea, Stroke,
};
use renamewright_application::{
    ApplicationService, ApplicationServiceErrorKind, ApplyCommandErrorDto, ApplyCommandResultDto,
    CaseModeDto, CharacterClassDto, CharacterClassOperationDto, ExtensionOperationDto,
    FilenamePartDto, LedgerEntryDto, PlanDto, PlanningCommandErrorDto, PresetDocumentDto,
    RangeOperationDto, RangeOriginDto, RecoveryCommandAction, RecoveryCommandErrorDto,
    RecoveryCommandResultDto, RecoveryInspectionDto, RecoveryRequestDto, RulePipelineRequestDto,
    RuleRequestDto, SequenceOrderDto, SequencePlacementDto, SequenceScopeDto, SourceOverrideDto,
    UndoCommandErrorDto, UndoCommandResultDto, UndoInspectionDto, UndoRequestDto,
    UnicodeNormalizationFormDto,
};
use renamewright_platform::{MAX_ADMITTED_SOURCES, NativeExecutionFileSystem};
use serde::{Deserialize, Serialize};

const SAMPLE_COUNT: usize = 10_000;
const SAMPLE_BLOCKED_COUNT: usize = (SAMPLE_COUNT - 1) / 997;
const PREVIEW_ROW_HEIGHT: f32 = 28.0;
const PREVIEW_CELL_HEIGHT: f32 = 20.0;
const PREVIEW_KIND_COLUMN_WIDTH: f32 = 70.0;
const PREVIEW_SOURCE_COLUMN_WIDTH: f32 = 130.0;
const PREVIEW_PROPOSED_COLUMN_WIDTH: f32 = 180.0;
const PREVIEW_STATUS_COLUMN_WIDTH: f32 = 80.0;
const PREVIEW_COLUMN_RESIZE_HANDLE_WIDTH: f32 = 8.0;
const APPEARANCE_STORAGE_KEY: &str = "renamewright.appearance.v1";
const MUTATION_POLL_INTERVAL: Duration = Duration::from_millis(50);
const PLANNING_DEBOUNCE: Duration = Duration::from_millis(100);
const PLANNING_POLL_INTERVAL: Duration = Duration::from_millis(16);
const LEDGER_POLL_INTERVAL: Duration = Duration::from_millis(25);
const FONT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const KOREAN_FONT_CANDIDATES: [&str; 4] = [
    "C:\\Windows\\Fonts\\malgun.ttf",
    "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
    "/usr/share/fonts/truetype/nanum/NanumGothicCoding.ttf",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
];
const EMOJI_FONT_CANDIDATES: [&str; 3] = [
    "C:\\Windows\\Fonts\\seguiemj.ttf",
    "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/opentype/noto/NotoColorEmoji.ttf",
];
const BASE_FONT_NAME: &str = "renamewright-base";

/// Installs the single embedded font needed to render the first application frame.
///
/// Korean text is added later from an operating-system font so the production
/// binary does not carry egui's complete default font bundle.
pub fn install_base_fonts(context: &egui::Context) {
    context.set_fonts(base_font_definitions());
}

fn base_font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    fonts.font_data.insert(
        BASE_FONT_NAME.to_owned(),
        Arc::new(FontData::from_static(epaint_default_fonts::UBUNTU_LIGHT)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .insert(family, vec![BASE_FONT_NAME.to_owned()]);
    }
    fonts
}

fn preview_column_label(
    ui: &mut egui::Ui,
    width: f32,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(width, PREVIEW_CELL_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            ui.add(egui::Label::new(text).truncate().halign(Align::Min))
        },
    )
    .inner
}

fn adjacent_selection_index(
    item_count: usize,
    current_index: Option<usize>,
    direction: i8,
) -> Option<usize> {
    if item_count == 0 {
        return None;
    }
    match (current_index, direction) {
        (Some(index), -1) => Some(index.saturating_sub(1)),
        (Some(index), 1) => Some(index.saturating_add(1).min(item_count - 1)),
        (None, -1 | 1) => Some(0),
        (Some(index), _) => Some(index.min(item_count - 1)),
        (None, _) => None,
    }
}

pub mod semantics {
    pub const PRODUCT_NAME: &str = "Renamewright";
    pub const TAGLINE: &str = "Plan every rename.";
    pub const ADD_FOLDER: &str = "Add folder name";
    pub const ADD_FILES: &str = "Add files";
    pub const RULES_HEADING: &str = "Rename rules";
    pub const RULES_ORDER_HELP: &str = "Earlier rules run first";
    pub const ACTIVE_RULES: &str = "Rule order";
    pub const MORE_RULES: &str = "More rules";
    pub const TOOLS: &str = "Advanced tools";
    pub const HISTORY: &str = "Rename history";
    pub const DONE_EDITING: &str = "Close editor";
    pub const CANCEL_EDITING: &str = "Discard new rule";
    pub const PREFIX_LABEL: &str = "Prefix text";
    pub const PREVIEW_HEADING: &str = "Preview";
    pub const FILTER_ALL: &str = "All";
    pub const FILTER_CHANGED: &str = "Will change";
    pub const FILTER_BLOCKED: &str = "Cannot apply";
    pub const SOURCE_QUERY_LABEL: &str = "Filter names";
    pub const APPLY: &str = "Rename entries";
    pub const MOVE_RULE_UP: &str = "Move rule up";
    pub const MOVE_RULE_DOWN: &str = "Move rule down";
    pub const DRAG_RULE: &str = "Drag rule";
    pub const REMOVE_RULE: &str = "Remove rule";
    pub const EXCLUDE_SOURCE: &str = "Leave out of plan";
    pub const ENABLE_RULE: &str = "Enable rule";
    pub const LANGUAGE: &str = "Language";
    pub const APPEARANCE: &str = "Display";
    pub const THEME_SYSTEM: &str = "System";
    pub const THEME_LIGHT: &str = "Light";
    pub const THEME_DARK: &str = "Dark";
    pub const ADVANCED_APPEARANCE: &str = "Advanced display settings";
    pub const CLOSE_APPEARANCE: &str = "Close display settings";
    pub const ACCENT_COLOR: &str = "Accent color";
    pub const DENSITY: &str = "Spacing";
    pub const DENSITY_STANDARD: &str = "Standard";
    pub const DENSITY_COMPACT: &str = "Compact";
    pub const PREVIEW_COLUMNS: &str = "Preview columns";
    pub const SHOW_KIND: &str = "Show entry kind";
    pub const SHOW_DIAGNOSTICS: &str = "Show all diagnostic details";
    pub const RESET_APPEARANCE: &str = "Reset advanced appearance";
    pub const HIGH_CONTRAST_OVERRIDES_APPEARANCE: &str =
        "Windows high contrast overrides appearance colors";
    pub const DIAGNOSTIC_FILTER: &str = "Diagnostic filter";
    pub const INSPECT_JSON: &str = "View plan JSON";
    pub const INSPECT_CSV: &str = "View plan CSV";
    pub const EXPORT_JSON: &str = "Export JSON";
    pub const EXPORT_CSV: &str = "Export CSV";
    pub const CLOSE_INSPECTOR: &str = "Close inspector";
    pub const SAVE_OVERRIDE: &str = "Save assigned name";
    pub const CANCEL_OVERRIDE: &str = "Cancel";
    pub const PRESETS: &str = "Saved presets";
    pub const PRESET_NAME: &str = "Preset name";
    pub const SAVE_PRESET: &str = "Save preset";
    pub const APPLY_PRESET: &str = "Apply preset";
    pub const DELETE_PRESET: &str = "Delete preset";
    pub const NO_SOURCES: &str = "No entries added yet";
    pub const AUTOMATION_BANNER: &str = "AUTOMATION TEST MODE";
    pub const HIGH_CONTRAST_ACTIVE: &str = "Windows high contrast palette active";
    pub const LEDGER: &str = "Rename history";
    pub const REFRESH_LEDGER: &str = "Refresh rename history";
    pub const INSPECT_RECOVERY: &str = "Review recovery options";
    pub const INSPECT_UNDO: &str = "Review Undo";
    pub const RESUME: &str = "Resume rename";
    pub const ROLLBACK: &str = "Restore previous names";
    pub const RECONCILE: &str = "Check current file state";
    pub const UNDO: &str = "Undo name change";
    pub const CANCEL_MUTATION: &str = "Cancel operation";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuleDragPayload {
    rule_id: u64,
}

#[cfg(feature = "automation")]
pub mod automation {
    use std::ffi::OsStr;
    use std::fmt::{Display, Formatter};
    use std::fs;
    use std::net::{TcpListener, TcpStream, ToSocketAddrs as _};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use eframe::egui;
    use egui_inspection::{InspectionPlugin, Request, Response};

    pub const AUTOMATION_BIND_ADDRESS: &str = "127.0.0.1:26191";
    pub const MAX_AUTOMATION_MESSAGE_BYTES: usize = 1024 * 1024;
    pub const MAX_AUTOMATION_EVENTS: usize = 256;
    pub const MAX_AUTOMATION_REQUESTS_PER_CONNECTION: usize = 128;
    pub const MAX_AUTOMATION_SETTLE_STEPS: u64 = 256;
    const MAX_AUTOMATION_CONNECTION_DURATION: Duration = Duration::from_secs(120);
    const AUTOMATION_IO_TIMEOUT: Duration = Duration::from_secs(5);
    const AUTOMATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
    const MAX_AUTOMATION_VIEWPORT_WIDTH: u32 = 3_840;
    const MAX_AUTOMATION_VIEWPORT_HEIGHT: u32 = 2_160;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum AutomationRootErrorKind {
        RootMustBeAbsolute,
        RootUnavailable,
        RootNotDirectory,
        ReparsePointRejected,
        InvalidProfile,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct AutomationRootError {
        kind: AutomationRootErrorKind,
    }

    impl AutomationRootError {
        const fn new(kind: AutomationRootErrorKind) -> Self {
            Self { kind }
        }

        #[must_use]
        pub const fn kind(self) -> AutomationRootErrorKind {
            self.kind
        }
    }

    impl Display for AutomationRootError {
        fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
            write!(
                formatter,
                "the automation root was rejected ({:?})",
                self.kind
            )
        }
    }

    impl std::error::Error for AutomationRootError {}

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum AutomationProfile {
        #[default]
        Empty,
        Performance,
    }

    impl AutomationProfile {
        pub fn parse(value: &OsStr) -> Result<Self, AutomationRootError> {
            match value.to_str() {
                Some("empty") => Ok(Self::Empty),
                Some("performance") => Ok(Self::Performance),
                _ => Err(AutomationRootError::new(
                    AutomationRootErrorKind::InvalidProfile,
                )),
            }
        }

        #[must_use]
        pub const fn synthetic_sample(self) -> bool {
            matches!(self, Self::Performance)
        }
    }

    #[derive(Debug)]
    pub struct AutomationRoot {
        canonical_root: PathBuf,
    }

    impl AutomationRoot {
        pub fn open(root: &Path) -> Result<Self, AutomationRootError> {
            if !root.is_absolute() {
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::RootMustBeAbsolute,
                ));
            }
            let metadata = fs::symlink_metadata(root)
                .map_err(|_| AutomationRootError::new(AutomationRootErrorKind::RootUnavailable))?;
            if !metadata.is_dir() {
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::RootNotDirectory,
                ));
            }
            if metadata_is_reparse_point(&metadata) {
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::ReparsePointRejected,
                ));
            }
            let canonical_root = fs::canonicalize(root)
                .map_err(|_| AutomationRootError::new(AutomationRootErrorKind::RootUnavailable))?;
            Ok(Self { canonical_root })
        }

        #[must_use]
        pub fn root(&self) -> &Path {
            &self.canonical_root
        }
    }

    pub fn serve_bounded(ctx: &egui::Context, address: &str) -> std::io::Result<()> {
        let resolved = address
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| std::io::Error::other("the automation address did not resolve"))?;
        if !resolved.ip().is_loopback() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the automation listener must use loopback",
            ));
        }
        let listener = TcpListener::bind(resolved)?;
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("renamewright_automation_accept".to_owned())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else {
                        continue;
                    };
                    let _ = serve_connection(stream, &ctx);
                }
            })?;
        Ok(())
    }

    struct DeadlineStream {
        stream: TcpStream,
        deadline: Instant,
    }

    impl DeadlineStream {
        const fn new(stream: TcpStream, deadline: Instant) -> Self {
            Self { stream, deadline }
        }

        fn timeout(&self) -> std::io::Result<Duration> {
            remaining_connection_duration(self.deadline).map(|remaining| {
                let timeout = remaining.min(AUTOMATION_IO_TIMEOUT);
                if timeout.is_zero() {
                    Duration::from_millis(1)
                } else {
                    timeout
                }
            })
        }

        fn normalize_timeout(&self, error: std::io::Error) -> std::io::Error {
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) && Instant::now() >= self.deadline
            {
                connection_timeout()
            } else {
                error
            }
        }
    }

    impl std::io::Read for DeadlineStream {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.stream.set_read_timeout(Some(self.timeout()?))?;
            self.stream
                .read(buffer)
                .map_err(|error| self.normalize_timeout(error))
        }
    }

    impl std::io::Write for DeadlineStream {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.stream.set_write_timeout(Some(self.timeout()?))?;
            self.stream
                .write(buffer)
                .map_err(|error| self.normalize_timeout(error))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.stream.set_write_timeout(Some(self.timeout()?))?;
            self.stream
                .flush()
                .map_err(|error| self.normalize_timeout(error))
        }
    }

    fn connection_timeout() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "the automation connection exceeded its runtime bound",
        )
    }

    fn remaining_connection_duration(deadline: Instant) -> std::io::Result<Duration> {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(connection_timeout)
    }

    fn serve_connection(stream: TcpStream, ctx: &egui::Context) -> std::io::Result<()> {
        let deadline = Instant::now() + MAX_AUTOMATION_CONNECTION_DURATION;
        let mut reader =
            std::io::BufReader::new(DeadlineStream::new(stream.try_clone()?, deadline));
        let mut writer = std::io::BufWriter::new(DeadlineStream::new(stream, deadline));
        egui_inspection::protocol::write_handshake(&mut writer)?;

        for _ in 0..MAX_AUTOMATION_REQUESTS_PER_CONNECTION {
            let request: Request = match read_bounded_message(&mut reader) {
                Ok(request) => request,
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(error) => return Err(error),
            };
            if !request_is_bounded(&request) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "the automation request exceeded a semantic bound",
                ));
            }

            let (sender, receiver) = mpsc::channel();
            let registered = ctx
                .with_plugin::<InspectionPlugin, _>(|plugin| {
                    plugin.submit(request, move |response| {
                        let _ = sender.send(response);
                    });
                })
                .is_some();
            if !registered {
                return egui_inspection::write_message(
                    &mut writer,
                    &Response::Error {
                        message: "the automation inspection plugin is unavailable".to_owned(),
                    },
                );
            }
            ctx.request_repaint();
            let response_timeout =
                remaining_connection_duration(deadline)?.min(AUTOMATION_REQUEST_TIMEOUT);
            let response =
                receiver
                    .recv_timeout(response_timeout)
                    .unwrap_or_else(|_| Response::Error {
                        message: "the automation request timed out".to_owned(),
                    });
            egui_inspection::write_message(&mut writer, &response)?;
        }
        Ok(())
    }

    fn read_bounded_message<R, T>(reader: &mut R) -> std::io::Result<T>
    where
        R: std::io::Read,
        T: for<'de> serde::Deserialize<'de>,
    {
        let mut header = [0_u8; 4];
        reader.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header) as usize;
        if length > MAX_AUTOMATION_MESSAGE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the automation message exceeded its byte bound",
            ));
        }
        let mut body = vec![0_u8; length];
        reader.read_exact(&mut body)?;
        egui_inspection::protocol::decode_frame_body(&body)
    }

    const fn request_is_bounded(request: &Request) -> bool {
        match request {
            Request::GetInfo | Request::GetTree => true,
            Request::GetScreenshot { pixels_per_point } => match pixels_per_point {
                Some(value) => value.is_finite() && *value > 0.0 && *value <= 4.0,
                None => true,
            },
            Request::ApplyEvents { events } => events.len() <= MAX_AUTOMATION_EVENTS,
            Request::Resize { width, height } => {
                *width > 0
                    && *height > 0
                    && *width <= MAX_AUTOMATION_VIEWPORT_WIDTH
                    && *height <= MAX_AUTOMATION_VIEWPORT_HEIGHT
            }
            Request::Settle { max_steps } => {
                *max_steps > 0 && *max_steps <= MAX_AUTOMATION_SETTLE_STEPS
            }
        }
    }

    #[cfg(windows)]
    fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
        use std::os::windows::fs::MetadataExt as _;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
        metadata.file_type().is_symlink()
    }

    #[cfg(test)]
    mod tests {
        use std::io::{Cursor, Write as _};
        use std::net::{TcpListener, TcpStream};
        use std::time::{Duration, Instant};

        use eframe::egui;
        use egui_inspection::Request;

        use super::{
            DeadlineStream, MAX_AUTOMATION_EVENTS, MAX_AUTOMATION_MESSAGE_BYTES,
            MAX_AUTOMATION_SETTLE_STEPS, read_bounded_message, request_is_bounded, serve_bounded,
        };

        #[test]
        fn framed_requests_are_rejected_before_oversized_allocation() {
            let declared = u32::try_from(MAX_AUTOMATION_MESSAGE_BYTES + 1)
                .unwrap_or(u32::MAX)
                .to_be_bytes();
            let mut input = Cursor::new(declared);

            let error = read_bounded_message::<_, Request>(&mut input).err();
            assert_eq!(
                error.map(|error| error.kind()),
                Some(std::io::ErrorKind::InvalidData)
            );
        }

        #[test]
        fn framed_requests_cannot_trickle_past_the_connection_deadline()
        -> Result<(), Box<dyn std::error::Error>> {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let address = listener.local_addr()?;
            let writer = std::thread::spawn(move || {
                let mut stream = TcpStream::connect(address)?;
                stream.write_all(&64_u32.to_be_bytes())?;
                for _ in 0..64 {
                    if stream.write_all(b"x").is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(15));
                }
                Ok::<(), std::io::Error>(())
            });
            let (stream, _) = listener.accept()?;
            let deadline = Instant::now() + Duration::from_millis(50);
            let mut reader = std::io::BufReader::new(DeadlineStream::new(stream, deadline));

            let error = read_bounded_message::<_, Request>(&mut reader).err();

            assert_eq!(
                error.map(|error| error.kind()),
                Some(std::io::ErrorKind::TimedOut)
            );
            drop(reader);
            writer.join().map_err(|_| "slow writer panicked")??;
            Ok(())
        }

        #[test]
        fn inspection_actions_have_event_viewport_and_scale_bounds() {
            assert!(request_is_bounded(&Request::ApplyEvents {
                events: vec![egui::Event::Copy; MAX_AUTOMATION_EVENTS],
            }));
            assert!(!request_is_bounded(&Request::ApplyEvents {
                events: vec![egui::Event::Copy; MAX_AUTOMATION_EVENTS + 1],
            }));
            assert!(request_is_bounded(&Request::Resize {
                width: 3_840,
                height: 2_160,
            }));
            assert!(!request_is_bounded(&Request::Resize {
                width: 3_841,
                height: 2_160,
            }));
            assert!(!request_is_bounded(&Request::GetScreenshot {
                pixels_per_point: Some(f32::NAN),
            }));
            assert!(request_is_bounded(&Request::Settle {
                max_steps: MAX_AUTOMATION_SETTLE_STEPS,
            }));
            assert!(!request_is_bounded(&Request::Settle {
                max_steps: MAX_AUTOMATION_SETTLE_STEPS + 1,
            }));
            assert!(!request_is_bounded(&Request::Settle { max_steps: 0 }));
        }

        #[test]
        fn inspection_listener_rejects_non_loopback_binding() {
            let error = serve_bounded(&egui::Context::default(), "0.0.0.0:0").err();
            assert_eq!(
                error.map(|error| error.kind()),
                Some(std::io::ErrorKind::PermissionDenied)
            );
        }
    }
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
const DARK_PAPER: Color32 = Color32::from_rgb(18, 22, 31);
const DARK_PAPER_RAISED: Color32 = Color32::from_rgb(25, 30, 42);
const DARK_PAPER_SOFT: Color32 = Color32::from_rgb(34, 41, 57);
const DARK_INK: Color32 = Color32::from_rgb(236, 239, 248);
const DARK_INK_SOFT: Color32 = Color32::from_rgb(172, 181, 202);
const DARK_RULE: Color32 = Color32::from_rgb(68, 78, 103);
const DARK_BLOCKED: Color32 = Color32::from_rgb(255, 154, 157);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum AppearanceTheme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum AccentChoice {
    #[default]
    Cobalt,
    Teal,
    Violet,
    Amber,
}

impl AccentChoice {
    const ALL: [Self; 4] = [Self::Cobalt, Self::Teal, Self::Violet, Self::Amber];

    const fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::Cobalt => locale.text("Cobalt", "코발트"),
            Self::Teal => locale.text("Teal", "틸"),
            Self::Violet => locale.text("Violet", "바이올렛"),
            Self::Amber => locale.text("Amber", "앰버"),
        }
    }

    const fn tokens(self, theme: egui::Theme) -> AccentTokens {
        match (theme, self) {
            (egui::Theme::Light, Self::Cobalt) => AccentTokens {
                foreground: Color32::from_rgb(42, 75, 183),
                fill: Color32::from_rgb(42, 75, 183),
                soft: Color32::from_rgb(222, 230, 255),
            },
            (egui::Theme::Dark, Self::Cobalt) => AccentTokens {
                foreground: Color32::from_rgb(153, 174, 255),
                fill: Color32::from_rgb(72, 99, 201),
                soft: Color32::from_rgb(43, 54, 88),
            },
            (egui::Theme::Light, Self::Teal) => AccentTokens {
                foreground: Color32::from_rgb(0, 105, 99),
                fill: Color32::from_rgb(0, 105, 99),
                soft: Color32::from_rgb(211, 242, 238),
            },
            (egui::Theme::Dark, Self::Teal) => AccentTokens {
                foreground: Color32::from_rgb(94, 214, 202),
                fill: Color32::from_rgb(0, 111, 104),
                soft: Color32::from_rgb(24, 67, 64),
            },
            (egui::Theme::Light, Self::Violet) => AccentTokens {
                foreground: Color32::from_rgb(99, 63, 174),
                fill: Color32::from_rgb(99, 63, 174),
                soft: Color32::from_rgb(235, 226, 255),
            },
            (egui::Theme::Dark, Self::Violet) => AccentTokens {
                foreground: Color32::from_rgb(199, 170, 255),
                fill: Color32::from_rgb(112, 72, 181),
                soft: Color32::from_rgb(62, 43, 88),
            },
            (egui::Theme::Light, Self::Amber) => AccentTokens {
                foreground: Color32::from_rgb(137, 84, 0),
                fill: Color32::from_rgb(137, 84, 0),
                soft: Color32::from_rgb(255, 235, 190),
            },
            (egui::Theme::Dark, Self::Amber) => AccentTokens {
                foreground: Color32::from_rgb(255, 194, 91),
                fill: Color32::from_rgb(142, 88, 0),
                soft: Color32::from_rgb(76, 53, 22),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AccentTokens {
    foreground: Color32,
    fill: Color32,
    soft: Color32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum InterfaceDensity {
    #[default]
    Standard,
    Compact,
}

impl InterfaceDensity {
    const ALL: [Self; 2] = [Self::Standard, Self::Compact];

    const fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::Standard => locale.text(semantics::DENSITY_STANDARD, "보통"),
            Self::Compact => locale.text(semantics::DENSITY_COMPACT, "좁게"),
        }
    }

    const fn preview_row_height(self) -> f32 {
        match self {
            Self::Standard => PREVIEW_ROW_HEIGHT,
            Self::Compact => 24.0,
        }
    }
}

impl AppearanceTheme {
    const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    const fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::System => locale.text(semantics::THEME_SYSTEM, "시스템 설정"),
            Self::Light => locale.text(semantics::THEME_LIGHT, "밝게"),
            Self::Dark => locale.text(semantics::THEME_DARK, "어둡게"),
        }
    }

    const fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }

    fn effective(self, context: &egui::Context) -> egui::Theme {
        match self {
            Self::System => context.system_theme().unwrap_or(egui::Theme::Light),
            Self::Light => egui::Theme::Light,
            Self::Dark => egui::Theme::Dark,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct AppearancePreferences {
    schema_version: u8,
    theme: AppearanceTheme,
    accent: AccentChoice,
    density: InterfaceDensity,
    show_kind: bool,
    show_diagnostics: bool,
    preview_source_column_width: Option<u16>,
}

impl Default for AppearancePreferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: AppearanceTheme::System,
            accent: AccentChoice::Cobalt,
            density: InterfaceDensity::Standard,
            show_kind: true,
            show_diagnostics: true,
            preview_source_column_width: None,
        }
    }
}

impl AppearancePreferences {
    fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| eframe::get_value(storage, APPEARANCE_STORAGE_KEY))
            .filter(|preferences: &Self| preferences.schema_version == 1)
            .unwrap_or_default()
    }

    fn reset_advanced(&mut self) {
        let theme = self.theme;
        *self = Self::default();
        self.theme = theme;
    }
}

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
    fn for_theme(theme: egui::Theme, accent: AccentChoice) -> Self {
        let accent = accent.tokens(theme);
        match theme {
            egui::Theme::Light => Self {
                paper: PAPER,
                paper_raised: PAPER_RAISED,
                paper_soft: PAPER_SOFT,
                ink: INK,
                ink_soft: INK_SOFT,
                rule: RULE,
                accent: accent.foreground,
                accent_fill: accent.fill,
                accent_soft: accent.soft,
                accent_text: Color32::WHITE,
                blocked: BLOCKED,
                disabled: INK_SOFT,
                high_contrast: false,
            },
            egui::Theme::Dark => Self {
                paper: DARK_PAPER,
                paper_raised: DARK_PAPER_RAISED,
                paper_soft: DARK_PAPER_SOFT,
                ink: DARK_INK,
                ink_soft: DARK_INK_SOFT,
                rule: DARK_RULE,
                accent: accent.foreground,
                accent_fill: accent.fill,
                accent_soft: accent.soft,
                accent_text: Color32::WHITE,
                blocked: DARK_BLOCKED,
                disabled: DARK_INK_SOFT,
                high_contrast: false,
            },
        }
    }

    fn theme(self) -> egui::Theme {
        if u16::from(self.paper.r()) + u16::from(self.paper.g()) + u16::from(self.paper.b()) < 384 {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        }
    }

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Locale {
    #[default]
    English,
    Korean,
}

impl Locale {
    const ALL: [Self; 2] = [Self::English, Self::Korean];

    const fn picker_label(self, korean_font_state: KoreanFontState) -> &'static str {
        match (self, korean_font_state) {
            (Self::English, _) => "English",
            (Self::Korean, KoreanFontState::Ready) => "한국어",
            (Self::Korean, KoreanFontState::Unavailable) => "Korean (font unavailable)",
            (Self::Korean, KoreanFontState::Idle | KoreanFontState::Loading) => "Korean (loading)",
        }
    }

    const fn picker_enabled(self, korean_font_state: KoreanFontState) -> bool {
        matches!(
            (self, korean_font_state),
            (Self::English, _) | (Self::Korean, KoreanFontState::Ready)
        )
    }

    const fn text(self, english: &'static str, korean: &'static str) -> &'static str {
        match self {
            Self::English => english,
            Self::Korean => korean,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuleKind {
    Prefix,
    Suffix,
    LiteralReplace,
    RegexReplace,
    Sequence,
    Extension,
    Case,
    WhitespaceCleanup,
    UnicodeNormalization,
    Range,
    CharacterClass,
}

impl RuleKind {
    const PRIMARY: [Self; 7] = [
        Self::LiteralReplace,
        Self::Prefix,
        Self::Suffix,
        Self::Sequence,
        Self::Range,
        Self::Extension,
        Self::Case,
    ];

    const SECONDARY: [Self; 4] = [
        Self::RegexReplace,
        Self::WhitespaceCleanup,
        Self::UnicodeNormalization,
        Self::CharacterClass,
    ];

    #[cfg(test)]
    const ALL: [Self; 11] = [
        Self::Prefix,
        Self::Suffix,
        Self::LiteralReplace,
        Self::RegexReplace,
        Self::Sequence,
        Self::Extension,
        Self::Case,
        Self::WhitespaceCleanup,
        Self::UnicodeNormalization,
        Self::Range,
        Self::CharacterClass,
    ];

    const fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::Prefix => locale.text("Prefix", "앞에 붙이기"),
            Self::Suffix => locale.text("Suffix", "뒤에 붙이기"),
            Self::LiteralReplace => locale.text("Replace", "찾아 바꾸기"),
            Self::RegexReplace => locale.text("Replace with pattern", "패턴으로 바꾸기"),
            Self::Sequence => locale.text("Number", "번호 붙이기"),
            Self::Extension => locale.text("Change extension", "확장자 바꾸기"),
            Self::Case => locale.text("Change case", "대/소문자 바꾸기"),
            Self::WhitespaceCleanup => locale.text("Clean whitespace", "공백 정리"),
            Self::UnicodeNormalization => locale.text("Normalize Unicode", "유니코드 정규화"),
            Self::Range => locale.text("Edit range", "구간 편집"),
            Self::CharacterClass => locale.text("Keep or remove characters", "문자 종류 정리"),
        }
    }

    fn create(self, rule_id: u64) -> RuleRequestDto {
        match self {
            Self::Prefix => RuleRequestDto::Prefix {
                rule_id,
                enabled: true,
                value: String::new(),
            },
            Self::Suffix => RuleRequestDto::Suffix {
                rule_id,
                enabled: true,
                value: String::new(),
            },
            Self::LiteralReplace => RuleRequestDto::LiteralReplace {
                rule_id,
                enabled: true,
                search: "old".to_owned(),
                replacement: String::new(),
            },
            Self::RegexReplace => RuleRequestDto::RegexReplace {
                rule_id,
                enabled: true,
                pattern: ".".to_owned(),
                replacement: String::new(),
            },
            Self::Sequence => RuleRequestDto::Sequence {
                rule_id,
                enabled: true,
                scope: SequenceScopeDto::AllSources,
                order: SequenceOrderDto::SourceOrder,
                start: 1,
                step: 1,
                padding: 3,
                placement: SequencePlacementDto::Prefix,
                separator: "_".to_owned(),
            },
            Self::Extension => RuleRequestDto::Extension {
                rule_id,
                enabled: true,
                operation: ExtensionOperationDto::Replace,
                value: "txt".to_owned(),
            },
            Self::Case => RuleRequestDto::Case {
                rule_id,
                enabled: true,
                target: FilenamePartDto::Stem,
                mode: CaseModeDto::Lowercase,
            },
            Self::WhitespaceCleanup => RuleRequestDto::WhitespaceCleanup {
                rule_id,
                enabled: true,
                target: FilenamePartDto::Stem,
                replacement: "_".to_owned(),
            },
            Self::UnicodeNormalization => RuleRequestDto::UnicodeNormalization {
                rule_id,
                enabled: true,
                target: FilenamePartDto::WholeName,
                form: UnicodeNormalizationFormDto::Nfc,
            },
            Self::Range => RuleRequestDto::Range {
                rule_id,
                enabled: true,
                target: FilenamePartDto::Stem,
                operation: RangeOperationDto::Keep,
                origin: RangeOriginDto::Start,
                offset: 0,
                length: None,
            },
            Self::CharacterClass => RuleRequestDto::CharacterClass {
                rule_id,
                enabled: true,
                target: FilenamePartDto::Stem,
                operation: CharacterClassOperationDto::Remove,
                class: CharacterClassDto::Whitespace,
            },
        }
    }
}

fn rule_kind(rule: &RuleRequestDto) -> RuleKind {
    match rule {
        RuleRequestDto::Prefix { .. } => RuleKind::Prefix,
        RuleRequestDto::Suffix { .. } => RuleKind::Suffix,
        RuleRequestDto::LiteralReplace { .. } => RuleKind::LiteralReplace,
        RuleRequestDto::RegexReplace { .. } => RuleKind::RegexReplace,
        RuleRequestDto::Sequence { .. } => RuleKind::Sequence,
        RuleRequestDto::Extension { .. } => RuleKind::Extension,
        RuleRequestDto::Case { .. } => RuleKind::Case,
        RuleRequestDto::WhitespaceCleanup { .. } => RuleKind::WhitespaceCleanup,
        RuleRequestDto::UnicodeNormalization { .. } => RuleKind::UnicodeNormalization,
        RuleRequestDto::Range { .. } => RuleKind::Range,
        RuleRequestDto::CharacterClass { .. } => RuleKind::CharacterClass,
    }
}

fn concise_rule_text(value: &str) -> String {
    const LIMIT: usize = 18;
    if value.is_empty() {
        return "empty".to_owned();
    }
    let mut characters = value.chars();
    let start = characters.by_ref().take(LIMIT).collect::<String>();
    if characters.next().is_some() {
        format!("{start}…")
    } else {
        start
    }
}

fn rule_has_non_ascii_text(rule: &RuleRequestDto) -> bool {
    match rule {
        RuleRequestDto::Prefix { value, .. }
        | RuleRequestDto::Suffix { value, .. }
        | RuleRequestDto::Extension { value, .. } => !value.is_ascii(),
        RuleRequestDto::LiteralReplace {
            search,
            replacement,
            ..
        }
        | RuleRequestDto::RegexReplace {
            pattern: search,
            replacement,
            ..
        } => !search.is_ascii() || !replacement.is_ascii(),
        RuleRequestDto::Sequence { separator, .. } => !separator.is_ascii(),
        RuleRequestDto::WhitespaceCleanup { replacement, .. } => !replacement.is_ascii(),
        RuleRequestDto::Case { .. }
        | RuleRequestDto::UnicodeNormalization { .. }
        | RuleRequestDto::Range { .. }
        | RuleRequestDto::CharacterClass { .. } => false,
    }
}

fn rule_summary(rule: &RuleRequestDto, locale: Locale) -> String {
    match rule {
        RuleRequestDto::Prefix { value, .. } => format!(
            "{} “{}”",
            RuleKind::Prefix.label(locale),
            concise_rule_text(value)
        ),
        RuleRequestDto::Suffix { value, .. } => format!(
            "{} “{}”",
            RuleKind::Suffix.label(locale),
            concise_rule_text(value)
        ),
        RuleRequestDto::LiteralReplace {
            search,
            replacement,
            ..
        } => format!(
            "{} “{}” -> “{}”",
            RuleKind::LiteralReplace.label(locale),
            concise_rule_text(search),
            concise_rule_text(replacement)
        ),
        RuleRequestDto::RegexReplace { pattern, .. } => format!(
            "{} /{}/",
            RuleKind::RegexReplace.label(locale),
            concise_rule_text(pattern)
        ),
        RuleRequestDto::Sequence { start, padding, .. } => format!(
            "{} {:0width$}",
            RuleKind::Sequence.label(locale),
            start,
            width = usize::try_from(*padding).unwrap_or(20)
        ),
        RuleRequestDto::Extension {
            operation, value, ..
        } => match operation {
            ExtensionOperationDto::Remove => {
                locale.text("Remove extension", "확장자 제거").to_owned()
            }
            ExtensionOperationDto::Replace => format!(
                "{} .{}",
                RuleKind::Extension.label(locale),
                concise_rule_text(value)
            ),
        },
        RuleRequestDto::Case { mode, .. } => match mode {
            CaseModeDto::Lowercase => {
                locale.text("Change case · lowercase", "대/소문자 · 소문자로")
            }
            CaseModeDto::Uppercase => {
                locale.text("Change case · uppercase", "대/소문자 · 대문자로")
            }
        }
        .to_owned(),
        RuleRequestDto::WhitespaceCleanup { .. }
        | RuleRequestDto::UnicodeNormalization { .. }
        | RuleRequestDto::Range { .. }
        | RuleRequestDto::CharacterClass { .. } => rule_kind(rule).label(locale).to_owned(),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DiagnosticFilter {
    #[default]
    All,
    Unchanged,
    EmptyName,
    IllegalCharacter,
    TrailingDotOrSpace,
    ReservedName,
    NameTooLong,
    DuplicateDestination,
    UnsupportedEncoding,
    OccupiedDestination,
    StaleSource,
    ParentUnavailable,
    InvalidRule,
    SequenceOverflow,
    AncestorDescendantConflict,
}

impl DiagnosticFilter {
    const ALL: [Self; 15] = [
        Self::All,
        Self::Unchanged,
        Self::EmptyName,
        Self::IllegalCharacter,
        Self::TrailingDotOrSpace,
        Self::ReservedName,
        Self::NameTooLong,
        Self::DuplicateDestination,
        Self::UnsupportedEncoding,
        Self::OccupiedDestination,
        Self::StaleSource,
        Self::ParentUnavailable,
        Self::InvalidRule,
        Self::SequenceOverflow,
        Self::AncestorDescendantConflict,
    ];

    const fn code(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Unchanged => Some("unchanged"),
            Self::EmptyName => Some("emptyName"),
            Self::IllegalCharacter => Some("illegalCharacter"),
            Self::TrailingDotOrSpace => Some("trailingDotOrSpace"),
            Self::ReservedName => Some("reservedName"),
            Self::NameTooLong => Some("nameTooLong"),
            Self::DuplicateDestination => Some("duplicateDestination"),
            Self::UnsupportedEncoding => Some("unsupportedEncoding"),
            Self::OccupiedDestination => Some("occupiedDestination"),
            Self::StaleSource => Some("staleSource"),
            Self::ParentUnavailable => Some("parentUnavailable"),
            Self::InvalidRule => Some("invalidRule"),
            Self::SequenceOverflow => Some("sequenceOverflow"),
            Self::AncestorDescendantConflict => Some("ancestorDescendantConflict"),
        }
    }

    const fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::All => locale.text("All diagnostics", "모든 진단"),
            Self::Unchanged => locale.text("No change", "변경 없음"),
            Self::EmptyName => locale.text("Name would be empty", "이름이 비게 됨"),
            Self::IllegalCharacter => locale.text("Illegal Windows character", "Windows 금지 문자"),
            Self::TrailingDotOrSpace => locale.text(
                "Name ends with a dot or space",
                "이름 끝에 점 또는 공백이 있음",
            ),
            Self::ReservedName => locale.text("Reserved Windows name", "Windows 예약 이름"),
            Self::NameTooLong => locale.text("Name too long", "바뀔 이름이 너무 김"),
            Self::DuplicateDestination => locale.text(
                "Same new name used more than once",
                "같은 새 이름을 여러 항목에 사용함",
            ),
            Self::UnsupportedEncoding => locale.text(
                "Name cannot be changed safely",
                "이름을 안전하게 변경할 수 없음",
            ),
            Self::OccupiedDestination => {
                locale.text("New name already exists", "같은 이름의 항목이 이미 있음")
            }
            Self::StaleSource => {
                locale.text("Entry changed after preview", "미리보기 후 항목이 변경됨")
            }
            Self::ParentUnavailable => {
                locale.text("Folder cannot be checked", "상위 폴더를 확인할 수 없음")
            }
            Self::InvalidRule => locale.text("Rule cannot be applied", "규칙을 적용할 수 없음"),
            Self::SequenceOverflow => {
                locale.text("Number is out of range", "번호가 허용 범위를 넘음")
            }
            Self::AncestorDescendantConflict => locale.text(
                "Folder and an item inside it are both selected",
                "폴더와 그 안의 항목을 함께 선택함",
            ),
        }
    }
}

fn diagnostic_label(code: &str, locale: Locale) -> &str {
    DiagnosticFilter::ALL
        .into_iter()
        .find(|candidate| candidate.code() == Some(code))
        .map_or_else(
            || locale.text("Unknown problem", "알 수 없는 문제"),
            |candidate| candidate.label(locale),
        )
}

fn planning_error_message(code: &str, locale: Locale) -> &'static str {
    match code {
        "tooManySources" => locale.text(
            "Too many entries were selected. Add no more than 10,000 at a time.",
            "한 번에 10,000개 이하의 항목을 추가하세요.",
        ),
        "sourceAdmissionFailed" => locale.text(
            "Some selected entries could not be added. Check that they still exist and try again.",
            "선택한 항목을 추가하지 못했습니다. 항목이 그대로 있는지 확인한 뒤 다시 시도하세요.",
        ),
        "stateUnavailable" => locale.text(
            "Renamewright could not read the current workbench state. Try again.",
            "현재 작업 상태를 확인하지 못했습니다. 다시 시도하세요.",
        ),
        "stalePlanningSnapshot" => locale.text(
            "The selected entries changed while the preview was updating. Review the latest preview.",
            "미리보기를 만드는 동안 선택한 항목이 변경되었습니다. 최신 미리보기를 확인하세요.",
        ),
        "planSequenceExhausted" => locale.text(
            "A new rename plan could not be created. Restart Renamewright before trying again.",
            "새 이름 변경 계획을 만들지 못했습니다. Renamewright를 다시 시작한 뒤 시도하세요.",
        ),
        "tooManyRules" => locale.text(
            "The rule limit has been reached. Remove a rule before adding another.",
            "규칙 개수 한도에 도달했습니다. 다른 규칙을 추가하기 전에 하나를 제거하세요.",
        ),
        "ruleTextTooLong" => locale.text(
            "This rule contains too much text. Shorten the value and try again.",
            "규칙에 입력한 내용이 너무 깁니다. 내용을 줄인 뒤 다시 시도하세요.",
        ),
        "emptyLiteralSearch" => {
            locale.text("Enter the text to find.", "찾을 내용을 입력하세요.")
        }
        "invalidRegex" => locale.text(
            "Check the pattern syntax.",
            "패턴 문법을 확인하세요.",
        ),
        "invalidSequenceStart" => locale.text(
            "Enter a valid starting number.",
            "올바른 시작 번호를 입력하세요.",
        ),
        "invalidSequenceStep" => locale.text(
            "The increment must be at least 1.",
            "증가값은 1 이상이어야 합니다.",
        ),
        "invalidSequencePadding" => locale.text(
            "The number of digits must be between 1 and 32.",
            "자릿수는 1에서 32 사이여야 합니다.",
        ),
        "invalidExtensionReplacement" => locale.text(
            "Check the new extension. Enter it without a leading dot.",
            "새 확장자를 확인하세요. 앞의 점(.)은 빼고 입력하세요.",
        ),
        "invalidRangeOffset" => locale.text(
            "Enter a valid starting position.",
            "올바른 시작 위치를 입력하세요.",
        ),
        "invalidRangeLength" => locale.text(
            "The selected length must be at least 1.",
            "선택할 길이는 1 이상이어야 합니다.",
        ),
        "tooManyOverrides" => locale.text(
            "Too many entries have directly assigned names. Remove an override before adding another.",
            "직접 이름을 지정한 항목이 너무 많습니다. 기존 직접 지정을 제거한 뒤 다시 시도하세요.",
        ),
        "overrideTextTooLong" => locale.text(
            "The directly assigned name is too long.",
            "직접 지정한 이름이 너무 깁니다.",
        ),
        "unknownSourceId" | "excludedOverrideSource" | "invalidOverrideSourceId"
        | "duplicateOverrideSourceId" | "unknownOverrideSource" => locale.text(
            "The selected entry is no longer in the current plan. Review the latest preview.",
            "선택한 항목이 현재 계획에 없습니다. 최신 미리보기를 확인하세요.",
        ),
        "unsupportedRuleSchema" | "invalidRuleId" | "duplicateRuleId" => locale.text(
            "This rule could not be read. Remove it and create the rule again.",
            "이 규칙을 읽지 못했습니다. 규칙을 제거한 뒤 다시 만드세요.",
        ),
        _ => locale.text(
            "The preview could not be updated. Review the rule and try again.",
            "미리보기를 갱신하지 못했습니다. 규칙을 확인한 뒤 다시 시도하세요.",
        ),
    }
}

fn preset_error_message(code: &str, locale: Locale) -> &'static str {
    match code {
        "invalidPresetName" => locale.text("Enter a preset name.", "프리셋 이름을 입력하세요."),
        "duplicatePresetName" => locale.text(
            "A preset with this name already exists. Choose another name.",
            "같은 이름의 프리셋이 이미 있습니다. 다른 이름을 입력하세요.",
        ),
        "tooManyPresets" => locale.text(
            "The preset limit has been reached. Delete a preset before saving another.",
            "프리셋 개수 한도에 도달했습니다. 기존 프리셋을 삭제한 뒤 저장하세요.",
        ),
        "presetDocumentTooLarge" => locale.text(
            "The preset file is too large to open.",
            "프리셋 파일이 너무 커서 열 수 없습니다.",
        ),
        "unsupportedPresetSchema" | "invalidPresetDocument" => locale.text(
            "The preset file could not be read.",
            "프리셋 파일을 읽을 수 없습니다.",
        ),
        "presetStorageUnavailable" => locale.text(
            "Presets could not be saved. Check the storage location and try again.",
            "프리셋을 저장하지 못했습니다. 저장 위치를 확인한 뒤 다시 시도하세요.",
        ),
        _ => locale.text("Presets are unavailable.", "프리셋을 사용할 수 없습니다."),
    }
}

fn application_error_message(kind: ApplicationServiceErrorKind, locale: Locale) -> &'static str {
    match kind {
        ApplicationServiceErrorKind::JournalPreparationFailed
        | ApplicationServiceErrorKind::JournalResolutionFailed
        | ApplicationServiceErrorKind::LedgerLoadFailed
        | ApplicationServiceErrorKind::LedgerRefreshFailed => locale.text(
            "Rename history could not be read. Try refreshing it.",
            "이름 변경 기록을 읽지 못했습니다. 기록을 새로 고쳐 보세요.",
        ),
        ApplicationServiceErrorKind::PlanSequenceExhausted => locale.text(
            "A new rename plan could not be created. Restart Renamewright before trying again.",
            "새 이름 변경 계획을 만들지 못했습니다. Renamewright를 다시 시작한 뒤 시도하세요.",
        ),
        ApplicationServiceErrorKind::StateUnavailable => locale.text(
            "Renamewright could not read the current workbench state. Try again.",
            "현재 작업 상태를 확인하지 못했습니다. 다시 시도하세요.",
        ),
        ApplicationServiceErrorKind::RecoveryInspectionFailed => locale.text(
            "Recovery options could not be checked. Refresh rename history and try again.",
            "복구 방법을 확인하지 못했습니다. 이름 변경 기록을 새로 고친 뒤 다시 시도하세요.",
        ),
        ApplicationServiceErrorKind::PlanInspectionFailed => locale.text(
            "The plan document could not be prepared.",
            "계획 문서를 준비하지 못했습니다.",
        ),
        ApplicationServiceErrorKind::PlanExportFailed => locale.text(
            "The plan could not be exported. Choose another file name or location and try again.",
            "계획을 내보내지 못했습니다. 다른 파일 이름이나 저장 위치를 선택해 다시 시도하세요.",
        ),
    }
}

fn apply_error_message(code: &str, locale: Locale) -> &'static str {
    match code {
        "busy" => locale.text(
            "Another file operation is already running.",
            "다른 파일 작업이 이미 실행 중입니다.",
        ),
        "planUnavailable" | "planChanged" => locale.text(
            "The plan changed after the preview. Review the latest preview and try again.",
            "미리보기 이후 계획이 변경되었습니다. 최신 미리보기를 확인한 뒤 다시 시도하세요.",
        ),
        "journalUnavailable" | "ledgerRefreshFailed" => locale.text(
            "Rename history is unavailable, so the names were not changed.",
            "이름 변경 기록을 사용할 수 없어 이름을 바꾸지 않았습니다.",
        ),
        "executionFailed" => locale.text(
            "The rename did not finish. Open rename history to review recovery options.",
            "이름 변경을 마치지 못했습니다. 이름 변경 기록에서 복구 방법을 확인하세요.",
        ),
        _ => locale.text(
            "The names could not be changed. Review the latest preview and try again.",
            "이름을 바꾸지 못했습니다. 최신 미리보기를 확인한 뒤 다시 시도하세요.",
        ),
    }
}

fn recovery_error_message(code: &str, locale: Locale) -> &'static str {
    match code {
        "busy" => locale.text(
            "Another file operation is already running.",
            "다른 파일 작업이 이미 실행 중입니다.",
        ),
        "inspectionChanged" => locale.text(
            "The files changed after recovery was reviewed. Check recovery options again.",
            "복구 방법을 확인한 뒤 파일 상태가 변경되었습니다. 복구 방법을 다시 확인하세요.",
        ),
        "actionUnavailable" => locale.text(
            "This recovery option is no longer available. Check the current file state again.",
            "이 복구 방법을 더 이상 사용할 수 없습니다. 현재 파일 상태를 다시 확인하세요.",
        ),
        _ => locale.text(
            "Recovery did not finish. Refresh rename history and review the available options.",
            "복구를 마치지 못했습니다. 이름 변경 기록을 새로 고친 뒤 가능한 방법을 확인하세요.",
        ),
    }
}

fn undo_error_message(code: &str, locale: Locale) -> &'static str {
    match code {
        "busy" => locale.text(
            "Another file operation is already running.",
            "다른 파일 작업이 이미 실행 중입니다.",
        ),
        "inspectionChanged" | "actionUnavailable" => locale.text(
            "The files changed after Undo was reviewed. Check Undo again.",
            "되돌리기를 확인한 뒤 파일 상태가 변경되었습니다. 되돌리기 가능 여부를 다시 확인하세요.",
        ),
        "planSequenceExhausted" => locale.text(
            "Undo could not start. Restart Renamewright before trying again.",
            "되돌리기를 시작하지 못했습니다. Renamewright를 다시 시작한 뒤 시도하세요.",
        ),
        _ => locale.text(
            "Undo did not finish. Refresh rename history and review the current state.",
            "되돌리기를 마치지 못했습니다. 이름 변경 기록을 새로 고친 뒤 현재 상태를 확인하세요.",
        ),
    }
}

fn outcome_label(outcome: &str, locale: Locale) -> &'static str {
    match outcome {
        "completed" => locale.text("Completed", "완료됨"),
        "rolledBack" => locale.text("Previous names restored", "변경 전 이름으로 되돌림"),
        "recoveryRequired" => locale.text("Recovery required", "복구 필요"),
        "cancelled" => locale.text("Cancelled", "취소됨"),
        _ => locale.text("Finished", "작업 종료"),
    }
}

fn ledger_status_label(status: &str, locale: Locale) -> &'static str {
    match status {
        "completed" => locale.text("Completed", "완료됨"),
        "rolledBack" => locale.text("Previous names restored", "변경 전 이름으로 되돌림"),
        "forwardPending" | "completionPending" => {
            locale.text("Rename interrupted", "이름 변경 중단됨")
        }
        "rollbackPending" | "rollbackCompletionPending" => {
            locale.text("Restore interrupted", "되돌리기 중단됨")
        }
        "reconciliationRequired" => locale.text("File state check required", "파일 상태 확인 필요"),
        "recoveryRequired" => locale.text("Recovery required", "복구 필요"),
        "legacyInspectionRequired" => locale.text("Manual review required", "직접 확인 필요"),
        "unsupportedVersion" => {
            locale.text("Unsupported history version", "지원하지 않는 기록 형식")
        }
        "tooLarge" => locale.text("History entry too large", "기록 항목이 너무 큼"),
        "discoveryLimitExceeded" => {
            locale.text("Too many history entries", "기록 항목이 너무 많음")
        }
        "torn" | "damaged" | "unreadable" => {
            locale.text("History entry unreadable", "기록 항목을 읽을 수 없음")
        }
        _ => locale.text("Status unavailable", "상태 확인 불가"),
    }
}

fn recovery_direction_label(direction: &str, locale: Locale) -> &'static str {
    match direction {
        "forward" => locale.text("Continue rename", "이름 변경 이어가기"),
        "rollback" => locale.text("Restore previous names", "변경 전 이름으로 되돌리기"),
        _ => locale.text("Recovery", "복구"),
    }
}

fn recovery_readiness_label(readiness: &str, locale: Locale) -> &'static str {
    match readiness {
        "ready" => locale.text("Ready", "진행 가능"),
        "reconciliationRequired" => locale.text(
            "Current file state must be checked",
            "현재 파일 상태 확인 필요",
        ),
        "blocked" => locale.text("Unavailable", "진행 불가"),
        _ => locale.text("Status unavailable", "상태 확인 불가"),
    }
}

fn undo_readiness_label(readiness: &str, locale: Locale) -> &'static str {
    match readiness {
        "ready" => locale.text("Ready to undo", "되돌리기 가능"),
        "blocked" => locale.text("Undo unavailable", "되돌리기 불가"),
        _ => locale.text("Status unavailable", "상태 확인 불가"),
    }
}

fn undo_block_reason_label(reason: &str, locale: Locale) -> &'static str {
    match reason {
        "sourceChanged" => locale.text(
            "A file or folder changed after the rename, so Undo is unavailable.",
            "이름을 바꾼 뒤 파일 또는 폴더가 변경되어 되돌릴 수 없습니다.",
        ),
        "destinationOccupied" => locale.text(
            "A previous name is already in use, so Undo is unavailable.",
            "이전 이름을 다른 항목이 사용하고 있어 되돌릴 수 없습니다.",
        ),
        _ => locale.text(
            "Undo is unavailable for the current file state.",
            "현재 파일 상태에서는 되돌릴 수 없습니다.",
        ),
    }
}

fn recovery_action_label(action: RecoveryCommandAction, locale: Locale) -> &'static str {
    match action {
        RecoveryCommandAction::Resume => locale.text(semantics::RESUME, "이어서 진행"),
        RecoveryCommandAction::Rollback => {
            locale.text(semantics::ROLLBACK, "변경 전 이름으로 되돌리기")
        }
        RecoveryCommandAction::Reconcile => {
            locale.text(semantics::RECONCILE, "현재 파일 상태 확인")
        }
    }
}

fn entry_count_label(count: usize, locale: Locale) -> String {
    match locale {
        Locale::English if count == 1 => "1 entry".to_owned(),
        Locale::English => format!("{count} entries"),
        Locale::Korean => format!("항목 {count}개"),
    }
}

fn rename_entries_label(count: usize, locale: Locale) -> String {
    match locale {
        Locale::English if count == 1 => "Rename 1 entry".to_owned(),
        Locale::English => format!("Rename {count} entries"),
        Locale::Korean => format!("{count}개 이름 바꾸기"),
    }
}

fn undo_entries_label(count: usize, locale: Locale) -> String {
    match locale {
        Locale::English if count == 1 => "Undo 1 name change".to_owned(),
        Locale::English => format!("Undo {count} name changes"),
        Locale::Korean => format!("{count}개 이름 되돌리기"),
    }
}

#[derive(Debug)]
struct OverrideEditor {
    source_id: u64,
    original_name: String,
    value: String,
}

#[derive(Debug)]
struct InspectionDocument {
    title: &'static str,
    content: String,
}

#[derive(Debug)]
enum PendingConfirmation {
    Apply {
        plan_id: u64,
        changed_count: usize,
    },
    Recovery {
        action: RecoveryCommandAction,
        inspection: RecoveryInspectionDto,
    },
    Undo {
        inspection: UndoInspectionDto,
    },
    Cancel,
}

#[derive(Debug)]
enum MutationMessage {
    Apply(Result<ApplyCommandResultDto, ApplyCommandErrorDto>),
    Recovery(Result<RecoveryCommandResultDto, RecoveryCommandErrorDto>),
    Undo(Result<UndoCommandResultDto, UndoCommandErrorDto>),
}

#[derive(Debug)]
struct MutationTask {
    receiver: Receiver<MutationMessage>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug)]
enum PlanningOutcome {
    Preview(Result<PlanDto, PlanningCommandErrorDto>),
    Validation(Result<(), PlanningCommandErrorDto>),
}

#[derive(Debug)]
struct PlanningMessage {
    revision: u64,
    outcome: PlanningOutcome,
}

#[derive(Debug)]
struct PlanningTask {
    receiver: Receiver<PlanningMessage>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug)]
struct AdmissionMessage {
    revision: u64,
    previous_source_count: usize,
    result: Result<PlanDto, PlanningCommandErrorDto>,
}

#[derive(Debug)]
struct AdmissionTask {
    receiver: Receiver<AdmissionMessage>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug)]
enum LedgerMessage {
    Snapshot {
        result: Result<Vec<LedgerEntryDto>, ApplicationServiceErrorKind>,
        announce: bool,
    },
    Recovery(Result<RecoveryInspectionDto, ApplicationServiceErrorKind>),
    Undo(Result<UndoInspectionDto, &'static str>),
}

#[derive(Debug)]
struct LedgerTask {
    receiver: Receiver<LedgerMessage>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug)]
enum DocumentMessage {
    Inspection {
        json: bool,
        result: Result<String, ApplicationServiceErrorKind>,
    },
    Export {
        extension: &'static str,
        result: Result<(), ApplicationServiceErrorKind>,
    },
}

#[derive(Debug)]
struct DocumentTask {
    receiver: Receiver<DocumentMessage>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum KoreanFontState {
    #[default]
    Idle,
    Loading,
    Ready,
    Unavailable,
}

#[derive(Debug)]
struct KoreanFontTask {
    receiver: Receiver<SystemFallbackFonts>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug)]
struct LoadedSystemFont {
    file_name: String,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct SystemFallbackFonts {
    korean: Option<LoadedSystemFont>,
    emoji: Option<LoadedSystemFont>,
}

fn read_first_system_font(candidates: &[&str]) -> Option<LoadedSystemFont> {
    candidates.iter().find_map(|candidate| {
        std::fs::read(candidate).ok().map(|bytes| LoadedSystemFont {
            file_name: std::path::Path::new(candidate).file_name().map_or_else(
                || (*candidate).to_owned(),
                |name| name.to_string_lossy().into_owned(),
            ),
            bytes,
        })
    })
}

fn add_system_fallback_font(context: &egui::Context, name: &'static str, font: LoadedSystemFont) {
    context.add_font(FontInsert::new(
        name,
        FontData::from_owned(font.bytes),
        [FontFamily::Proportional, FontFamily::Monospace]
            .into_iter()
            .map(|family| InsertFontFamily {
                family,
                priority: FontPriority::Lowest,
            })
            .collect(),
    ));
}

impl KoreanFontTask {
    fn finish(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl DocumentTask {
    fn finish(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl LedgerTask {
    fn finish(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl PlanningTask {
    fn finish(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl AdmissionTask {
    fn finish(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Debug)]
struct VisibleRowsKey {
    plan_id: Option<u64>,
    source_query: String,
    filter: PlanFilter,
    diagnostic_filter: DiagnosticFilter,
    synthetic_fixture: bool,
}

#[derive(Debug, Default)]
struct VisibleRowsCache {
    key: Option<VisibleRowsKey>,
    search_plan_id: Option<u64>,
    search_text: Vec<String>,
    indices: Arc<[usize]>,
}

impl MutationTask {
    fn finish(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn rule_enabled_mut(rule: &mut RuleRequestDto) -> &mut bool {
    match rule {
        RuleRequestDto::Prefix { enabled, .. }
        | RuleRequestDto::Suffix { enabled, .. }
        | RuleRequestDto::LiteralReplace { enabled, .. }
        | RuleRequestDto::RegexReplace { enabled, .. }
        | RuleRequestDto::Sequence { enabled, .. }
        | RuleRequestDto::Extension { enabled, .. }
        | RuleRequestDto::Case { enabled, .. }
        | RuleRequestDto::WhitespaceCleanup { enabled, .. }
        | RuleRequestDto::UnicodeNormalization { enabled, .. }
        | RuleRequestDto::Range { enabled, .. }
        | RuleRequestDto::CharacterClass { enabled, .. } => enabled,
    }
}

fn filename_part_label(part: FilenamePartDto, locale: Locale) -> &'static str {
    match part {
        FilenamePartDto::WholeName => locale.text("Whole name", "전체 이름"),
        FilenamePartDto::Stem => locale.text("Name", "이름"),
        FilenamePartDto::Extension => locale.text("Extension", "확장자"),
    }
}

fn focus_once(response: &egui::Response, request_focus: &mut bool) {
    if *request_focus {
        response.request_focus();
        *request_focus = false;
    }
}

fn filename_part_control(
    ui: &mut egui::Ui,
    target: &mut FilenamePartDto,
    locale: Locale,
    request_focus: &mut bool,
) -> bool {
    let before = *target;
    ui.label(locale.text("Part to change", "바꿀 부분"));
    let response = egui::ComboBox::from_id_salt("filename-part")
        .selected_text(filename_part_label(*target, locale))
        .show_ui(ui, |ui| {
            for candidate in [
                FilenamePartDto::WholeName,
                FilenamePartDto::Stem,
                FilenamePartDto::Extension,
            ] {
                ui.selectable_value(target, candidate, filename_part_label(candidate, locale));
            }
        })
        .response;
    focus_once(&response, request_focus);
    before != *target
}

fn rule_editor(
    ui: &mut egui::Ui,
    rule: &mut RuleRequestDto,
    locale: Locale,
    focus_first: bool,
) -> bool {
    let mut changed = false;
    let mut request_focus = focus_first;
    ui.horizontal_wrapped(|ui| {
        changed |= ui
            .checkbox(
                rule_enabled_mut(rule),
                locale.text(semantics::ENABLE_RULE, "규칙 사용"),
            )
            .changed();
        ui.separator();
        match rule {
            RuleRequestDto::Prefix { value, .. } => {
                let label = ui.label(locale.text(semantics::PREFIX_LABEL, "접두사 텍스트"));
                let response = ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .id_salt("rule.prefix.value")
                            .desired_width(180.0)
                            .hint_text(locale.text("Text", "텍스트")),
                    )
                    .labelled_by(label.id);
                focus_once(&response, &mut request_focus);
                changed |= response.changed();
            }
            RuleRequestDto::Suffix { value, .. } => {
                let label = ui.label(locale.text("Suffix text", "접미사 텍스트"));
                let response = ui
                    .add(
                        egui::TextEdit::singleline(value)
                            .id_salt("rule.suffix.value")
                            .desired_width(180.0)
                            .hint_text(locale.text("Text", "텍스트")),
                    )
                    .labelled_by(label.id);
                focus_once(&response, &mut request_focus);
                changed |= response.changed();
            }
            RuleRequestDto::LiteralReplace {
                search,
                replacement,
                ..
            } => {
                let find_label = ui.label(locale.text("Find", "찾기"));
                let find = ui
                    .add(egui::TextEdit::singleline(search).desired_width(160.0))
                    .labelled_by(find_label.id);
                focus_once(&find, &mut request_focus);
                changed |= find.changed();
                let replacement_label = ui.label(locale.text("Replace with", "바꿀 내용"));
                changed |= ui
                    .add(egui::TextEdit::singleline(replacement).desired_width(160.0))
                    .labelled_by(replacement_label.id)
                    .changed();
            }
            RuleRequestDto::RegexReplace {
                pattern,
                replacement,
                ..
            } => {
                let pattern_label = ui.label(locale.text("Pattern", "패턴"));
                let pattern_response = ui
                    .add(egui::TextEdit::singleline(pattern).desired_width(160.0))
                    .labelled_by(pattern_label.id);
                focus_once(&pattern_response, &mut request_focus);
                changed |= pattern_response.changed();
                let replacement_label = ui.label(locale.text("Replacement", "바꿀 내용"));
                changed |= ui
                    .add(egui::TextEdit::singleline(replacement).desired_width(160.0))
                    .labelled_by(replacement_label.id)
                    .changed();
            }
            RuleRequestDto::Sequence {
                scope,
                order,
                start,
                step,
                padding,
                placement,
                separator,
                ..
            } => {
                ui.label(locale.text("Numbering group", "번호 매기기 단위"));
                let before = *scope;
                egui::ComboBox::from_id_salt("sequence-scope")
                    .selected_text(match scope {
                        SequenceScopeDto::AllSources => locale.text("All entries", "전체 항목"),
                        SequenceScopeDto::PerParent => {
                            locale.text("Restart in each folder", "폴더마다 다시 시작")
                        }
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            scope,
                            SequenceScopeDto::AllSources,
                            locale.text("All entries", "전체 항목"),
                        );
                        ui.selectable_value(
                            scope,
                            SequenceScopeDto::PerParent,
                            locale.text("Restart in each folder", "폴더마다 다시 시작"),
                        );
                    });
                changed |= before != *scope;
                ui.label(locale.text("Number order", "번호 순서 기준"));
                let before = *order;
                egui::ComboBox::from_id_salt("sequence-order")
                    .selected_text(match order {
                        SequenceOrderDto::SourceOrder => locale.text("Order added", "추가한 순서"),
                        SequenceOrderDto::NameAscending => {
                            locale.text("Name ascending", "이름 오름차순")
                        }
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            order,
                            SequenceOrderDto::SourceOrder,
                            locale.text("Order added", "추가한 순서"),
                        );
                        ui.selectable_value(
                            order,
                            SequenceOrderDto::NameAscending,
                            locale.text("Name ascending", "이름 오름차순"),
                        );
                    });
                changed |= before != *order;
                ui.horizontal(|ui| {
                    ui.label(locale.text("First number", "시작 번호"));
                    let start_response = ui.add(egui::DragValue::new(start));
                    focus_once(&start_response, &mut request_focus);
                    changed |= start_response.changed();
                    ui.label(locale.text("Increment", "증가값"));
                    changed |= ui
                        .add(egui::DragValue::new(step).range(1..=u64::MAX))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label(locale.text("Number of digits", "자릿수"));
                    changed |= ui
                        .add(egui::DragValue::new(padding).range(1..=32))
                        .changed();
                    ui.label(locale.text("Between number and name", "번호와 이름 사이"));
                    changed |= ui.text_edit_singleline(separator).changed();
                });
                let before = *placement;
                ui.label(locale.text("Number position", "번호 위치"));
                egui::ComboBox::from_id_salt("sequence-placement")
                    .selected_text(match placement {
                        SequencePlacementDto::Prefix => locale.text("Before name", "이름 앞"),
                        SequencePlacementDto::Suffix => locale.text("After name", "이름 뒤"),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            placement,
                            SequencePlacementDto::Prefix,
                            locale.text("Before name", "이름 앞"),
                        );
                        ui.selectable_value(
                            placement,
                            SequencePlacementDto::Suffix,
                            locale.text("After name", "이름 뒤"),
                        );
                    });
                changed |= before != *placement;
            }
            RuleRequestDto::Extension {
                operation, value, ..
            } => {
                let before = *operation;
                ui.label(locale.text("Action", "처리 방법"));
                egui::ComboBox::from_id_salt("extension-operation")
                    .selected_text(match operation {
                        ExtensionOperationDto::Remove => locale.text("Remove", "제거"),
                        ExtensionOperationDto::Replace => locale.text("Replace", "바꾸기"),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            operation,
                            ExtensionOperationDto::Remove,
                            locale.text("Remove", "제거"),
                        );
                        ui.selectable_value(
                            operation,
                            ExtensionOperationDto::Replace,
                            locale.text("Replace", "바꾸기"),
                        );
                    });
                changed |= before != *operation;
                if *operation == ExtensionOperationDto::Replace {
                    let label = ui.label(locale.text("New extension", "새 확장자"));
                    let response = ui
                        .add(
                            egui::TextEdit::singleline(value)
                                .desired_width(140.0)
                                .hint_text("txt"),
                        )
                        .labelled_by(label.id);
                    focus_once(&response, &mut request_focus);
                    changed |= response.changed();
                }
            }
            RuleRequestDto::Case { target, mode, .. } => {
                changed |= filename_part_control(ui, target, locale, &mut request_focus);
                let before = *mode;
                ui.label(locale.text("Letter case", "바꿀 형태"));
                egui::ComboBox::from_id_salt("case-mode")
                    .selected_text(match mode {
                        CaseModeDto::Lowercase => locale.text("Lowercase", "소문자"),
                        CaseModeDto::Uppercase => locale.text("Uppercase", "대문자"),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            mode,
                            CaseModeDto::Lowercase,
                            locale.text("Lowercase", "소문자"),
                        );
                        ui.selectable_value(
                            mode,
                            CaseModeDto::Uppercase,
                            locale.text("Uppercase", "대문자"),
                        );
                    });
                changed |= before != *mode;
            }
            RuleRequestDto::WhitespaceCleanup {
                target,
                replacement,
                ..
            } => {
                changed |= filename_part_control(ui, target, locale, &mut request_focus);
                ui.label(locale.text("Replace whitespace with", "공백을 바꿀 문자"));
                changed |= ui.text_edit_singleline(replacement).changed();
            }
            RuleRequestDto::UnicodeNormalization { target, form, .. } => {
                changed |= filename_part_control(ui, target, locale, &mut request_focus);
                let before = *form;
                ui.label(locale.text("Unicode form", "유니코드 형식"));
                egui::ComboBox::from_id_salt("normalization-form")
                    .selected_text(match form {
                        UnicodeNormalizationFormDto::Nfc => "NFC",
                        UnicodeNormalizationFormDto::Nfd => "NFD",
                        UnicodeNormalizationFormDto::Nfkc => "NFKC",
                        UnicodeNormalizationFormDto::Nfkd => "NFKD",
                    })
                    .show_ui(ui, |ui| {
                        for candidate in [
                            UnicodeNormalizationFormDto::Nfc,
                            UnicodeNormalizationFormDto::Nfd,
                            UnicodeNormalizationFormDto::Nfkc,
                            UnicodeNormalizationFormDto::Nfkd,
                        ] {
                            let label = match candidate {
                                UnicodeNormalizationFormDto::Nfc => "NFC",
                                UnicodeNormalizationFormDto::Nfd => "NFD",
                                UnicodeNormalizationFormDto::Nfkc => "NFKC",
                                UnicodeNormalizationFormDto::Nfkd => "NFKD",
                            };
                            ui.selectable_value(form, candidate, label);
                        }
                    });
                changed |= before != *form;
                ui.label(locale.text(
                    "NFC is the common choice for Windows filenames.",
                    "Windows 파일 이름에는 일반적으로 NFC를 사용합니다.",
                ));
            }
            RuleRequestDto::Range {
                target,
                operation,
                origin,
                offset,
                length,
                ..
            } => {
                changed |= filename_part_control(ui, target, locale, &mut request_focus);
                let before = *operation;
                ui.label(locale.text("Result", "처리 방법"));
                egui::ComboBox::from_id_salt("range-operation")
                    .selected_text(match operation {
                        RangeOperationDto::Keep => {
                            locale.text("Keep selected range", "선택 구간만 남기기")
                        }
                        RangeOperationDto::Remove => {
                            locale.text("Remove selected range", "선택 구간 지우기")
                        }
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            operation,
                            RangeOperationDto::Keep,
                            locale.text("Keep selected range", "선택 구간만 남기기"),
                        );
                        ui.selectable_value(
                            operation,
                            RangeOperationDto::Remove,
                            locale.text("Remove selected range", "선택 구간 지우기"),
                        );
                    });
                changed |= before != *operation;
                let before = *origin;
                ui.label(locale.text("Measure from", "위치 기준"));
                egui::ComboBox::from_id_salt("range-origin")
                    .selected_text(match origin {
                        RangeOriginDto::Start => locale.text("Start of name", "이름 앞에서"),
                        RangeOriginDto::End => locale.text("End of name", "이름 뒤에서"),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            origin,
                            RangeOriginDto::Start,
                            locale.text("Start of name", "이름 앞에서"),
                        );
                        ui.selectable_value(
                            origin,
                            RangeOriginDto::End,
                            locale.text("End of name", "이름 뒤에서"),
                        );
                    });
                changed |= before != *origin;
                ui.horizontal(|ui| {
                    ui.label(locale.text("Start position", "시작 위치"));
                    let response = ui.add(egui::DragValue::new(offset));
                    focus_once(&response, &mut request_focus);
                    changed |= response.changed();
                });
                let mut open_ended = length.is_none();
                if ui
                    .checkbox(&mut open_ended, locale.text("Through end", "끝까지"))
                    .changed()
                {
                    *length = if open_ended { None } else { Some(1) };
                    changed = true;
                }
                if let Some(length) = length {
                    ui.label(locale.text("Number of characters", "문자 수"));
                    changed |= ui
                        .add(egui::DragValue::new(length).range(1..=u64::MAX))
                        .changed();
                }
            }
            RuleRequestDto::CharacterClass {
                target,
                operation,
                class,
                ..
            } => {
                changed |= filename_part_control(ui, target, locale, &mut request_focus);
                let before = *operation;
                ui.label(locale.text("Result", "처리 방법"));
                egui::ComboBox::from_id_salt("class-operation")
                    .selected_text(match operation {
                        CharacterClassOperationDto::Keep => {
                            locale.text("Keep this type", "이 종류만 남기기")
                        }
                        CharacterClassOperationDto::Remove => {
                            locale.text("Remove this type", "이 종류 지우기")
                        }
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            operation,
                            CharacterClassOperationDto::Keep,
                            locale.text("Keep this type", "이 종류만 남기기"),
                        );
                        ui.selectable_value(
                            operation,
                            CharacterClassOperationDto::Remove,
                            locale.text("Remove this type", "이 종류 지우기"),
                        );
                    });
                changed |= before != *operation;
                let before = *class;
                let class_label = |candidate| match candidate {
                    CharacterClassDto::DecimalNumber => locale.text("Numbers", "숫자"),
                    CharacterClassDto::Letter => locale.text("Letters", "문자"),
                    CharacterClassDto::Whitespace => locale.text("Whitespace", "공백"),
                    CharacterClassDto::Punctuation => locale.text("Punctuation", "문장 부호"),
                    CharacterClassDto::Symbol => locale.text("Symbols", "기호"),
                };
                ui.label(locale.text("Character type", "문자 종류"));
                egui::ComboBox::from_id_salt("character-class")
                    .selected_text(class_label(*class))
                    .show_ui(ui, |ui| {
                        for candidate in [
                            CharacterClassDto::DecimalNumber,
                            CharacterClassDto::Letter,
                            CharacterClassDto::Whitespace,
                            CharacterClassDto::Punctuation,
                            CharacterClassDto::Symbol,
                        ] {
                            ui.selectable_value(class, candidate, class_label(candidate));
                        }
                    });
                changed |= before != *class;
            }
        }
    });
    changed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanFilter {
    All,
    Changed,
    Unchanged,
    Blocked,
}

impl PlanFilter {
    const ALL: [Self; 4] = [Self::All, Self::Changed, Self::Unchanged, Self::Blocked];

    const fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::All => locale.text(semantics::FILTER_ALL, "전체"),
            Self::Changed => locale.text(semantics::FILTER_CHANGED, "변경 예정"),
            Self::Unchanged => locale.text("Unchanged", "변경 없음"),
            Self::Blocked => locale.text(semantics::FILTER_BLOCKED, "적용 불가"),
        }
    }
}

#[derive(Debug)]
pub struct RenamewrightApp {
    application: Arc<ApplicationService>,
    plan: Option<PlanDto>,
    rules: Vec<RuleRequestDto>,
    overrides: BTreeMap<u64, String>,
    next_rule_id: u64,
    source_query: String,
    filter: PlanFilter,
    diagnostic_filter: DiagnosticFilter,
    selected_preview_row: Option<usize>,
    preview_focus_requested: bool,
    source_query_focus_requested: bool,
    selected_rule: usize,
    rule_editor_open: bool,
    focused_rule_id: Option<u64>,
    draft_rule_id: Option<u64>,
    draft_rule_changed: bool,
    rule_error: Option<String>,
    ledger_open: bool,
    tools_open: bool,
    locale: Locale,
    override_editor: Option<OverrideEditor>,
    inspection: Option<InspectionDocument>,
    document_task: Option<DocumentTask>,
    synthetic_fixture: bool,
    preset_path: Option<PathBuf>,
    journal_root: Option<PathBuf>,
    presets: PresetDocumentDto,
    preset_name: String,
    status: String,
    ledger: Vec<LedgerEntryDto>,
    ledger_ready: bool,
    ledger_task: Option<LedgerTask>,
    selected_ledger_id: Option<u64>,
    recovery_inspection: Option<RecoveryInspectionDto>,
    undo_inspection: Option<UndoInspectionDto>,
    pending_confirmation: Option<PendingConfirmation>,
    mutation_task: Option<MutationTask>,
    planning_revision: u64,
    planning_due: Option<Instant>,
    planning_task: Option<PlanningTask>,
    admission_task: Option<AdmissionTask>,
    pending_admission_paths: Vec<PathBuf>,
    plan_is_current: bool,
    visible_rows: VisibleRowsCache,
    native_palette: NativePalette,
    palette: NativePalette,
    appearance: AppearancePreferences,
    appearance_applied: bool,
    appearance_advanced_open: bool,
    korean_font_state: KoreanFontState,
    korean_font_task: Option<KoreanFontTask>,
    #[cfg(feature = "automation")]
    automation_mode: bool,
    #[cfg(feature = "automation")]
    _automation_root: Option<automation::AutomationRoot>,
}

impl RenamewrightApp {
    #[must_use]
    pub fn new(_automation_mode: bool) -> Self {
        Self::new_with_palette(_automation_mode, NativePalette::default())
    }

    #[must_use]
    pub fn new_with_palette(_automation_mode: bool, palette: NativePalette) -> Self {
        Self::new_with_storage(_automation_mode, palette, None)
    }

    #[must_use]
    pub fn new_with_storage(
        _automation_mode: bool,
        palette: NativePalette,
        preset_path: Option<PathBuf>,
    ) -> Self {
        Self::new_configured(
            _automation_mode,
            palette,
            preset_path,
            None,
            true,
            AppearancePreferences::default(),
        )
    }

    #[must_use]
    pub fn new_product(palette: NativePalette, preset_path: Option<PathBuf>) -> Self {
        Self::new_configured(
            false,
            palette,
            preset_path,
            None,
            false,
            AppearancePreferences::default(),
        )
    }

    #[must_use]
    pub fn new_product_with_data(
        palette: NativePalette,
        preset_path: Option<PathBuf>,
        journal_root: Option<PathBuf>,
    ) -> Self {
        Self::new_configured(
            false,
            palette,
            preset_path,
            journal_root,
            false,
            AppearancePreferences::default(),
        )
    }

    #[must_use]
    pub fn new_product_with_persistence(
        palette: NativePalette,
        preset_path: Option<PathBuf>,
        journal_root: Option<PathBuf>,
        storage: Option<&dyn eframe::Storage>,
    ) -> Self {
        Self::new_configured(
            false,
            palette,
            preset_path,
            journal_root,
            false,
            AppearancePreferences::load(storage),
        )
    }

    fn new_configured(
        _automation_mode: bool,
        palette: NativePalette,
        preset_path: Option<PathBuf>,
        journal_root: Option<PathBuf>,
        synthetic_fixture: bool,
        appearance: AppearancePreferences,
    ) -> Self {
        let (presets, preset_status) = preset_path.as_ref().map_or_else(
            || (PresetDocumentDto::default(), None),
            |path| match PresetDocumentDto::load(path) {
                Ok(document) => (document, None),
                Err(error) => (
                    PresetDocumentDto::default(),
                    Some(preset_error_message(error.code(), Locale::English).to_owned()),
                ),
            },
        );
        let application = Arc::new(ApplicationService::default());
        let ledger_task = journal_root.as_ref().map(|root| {
            let application = Arc::clone(&application);
            let root = root.clone();
            let (sender, receiver) = mpsc::channel();
            let handle = thread::spawn(move || {
                let result = application
                    .initialize(&root)
                    .and_then(|()| application.ledger_snapshot())
                    .map_err(|error| error.kind());
                let _ = sender.send(LedgerMessage::Snapshot {
                    result,
                    announce: false,
                });
            });
            LedgerTask {
                receiver,
                handle: Some(handle),
            }
        });
        let rules = if synthetic_fixture {
            vec![RuleRequestDto::Prefix {
                rule_id: 1,
                enabled: true,
                value: "정리_".to_owned(),
            }]
        } else {
            Vec::new()
        };
        let has_initial_rule = !rules.is_empty();
        Self {
            application,
            plan: None,
            rules,
            overrides: BTreeMap::new(),
            next_rule_id: if has_initial_rule { 2 } else { 1 },
            source_query: String::new(),
            filter: PlanFilter::All,
            diagnostic_filter: DiagnosticFilter::All,
            selected_preview_row: None,
            preview_focus_requested: false,
            source_query_focus_requested: false,
            selected_rule: 0,
            rule_editor_open: has_initial_rule,
            focused_rule_id: None,
            draft_rule_id: None,
            draft_rule_changed: false,
            rule_error: None,
            ledger_open: false,
            tools_open: false,
            locale: Locale::English,
            override_editor: None,
            inspection: None,
            document_task: None,
            synthetic_fixture,
            preset_path,
            journal_root,
            presets,
            preset_name: String::new(),
            status: preset_status.unwrap_or_else(|| {
                if ledger_task.is_some() {
                    "Checking rename history".to_owned()
                } else if synthetic_fixture {
                    format!("{SAMPLE_COUNT} sample entries ready")
                } else {
                    semantics::NO_SOURCES.to_owned()
                }
            }),
            ledger: Vec::new(),
            ledger_ready: false,
            ledger_task,
            selected_ledger_id: None,
            recovery_inspection: None,
            undo_inspection: None,
            pending_confirmation: None,
            mutation_task: None,
            planning_revision: 0,
            planning_due: None,
            planning_task: None,
            admission_task: None,
            pending_admission_paths: Vec::new(),
            plan_is_current: true,
            visible_rows: VisibleRowsCache::default(),
            native_palette: palette,
            palette,
            appearance,
            appearance_applied: false,
            appearance_advanced_open: false,
            korean_font_state: KoreanFontState::Idle,
            korean_font_task: None,
            #[cfg(feature = "automation")]
            automation_mode: _automation_mode,
            #[cfg(feature = "automation")]
            _automation_root: None,
        }
    }

    #[cfg(feature = "automation")]
    #[must_use]
    pub fn new_automated(
        palette: NativePalette,
        automation_root: automation::AutomationRoot,
        profile: automation::AutomationProfile,
    ) -> Self {
        let synthetic_fixture = profile.synthetic_sample();
        let mut app = Self::new_configured(
            true,
            palette,
            None,
            None,
            synthetic_fixture,
            AppearancePreferences::default(),
        );
        if synthetic_fixture {
            app.status = "Automation performance profile loaded".to_owned();
        }
        app._automation_root = Some(automation_root);
        app
    }

    fn row_is_blocked(index: usize) -> bool {
        index > 0 && index.is_multiple_of(997)
    }

    fn visible_indices(&mut self) -> Arc<[usize]> {
        let plan_id = self.plan.as_ref().map(PlanDto::plan_id);
        if self.visible_rows.key.as_ref().is_some_and(|key| {
            key.plan_id == plan_id
                && key.source_query == self.source_query
                && key.filter == self.filter
                && key.diagnostic_filter == self.diagnostic_filter
                && key.synthetic_fixture == self.synthetic_fixture
        }) {
            return Arc::clone(&self.visible_rows.indices);
        }

        if self.visible_rows.search_plan_id != plan_id {
            self.visible_rows.search_text.clear();
            if let Some(plan) = &self.plan {
                self.visible_rows.search_text.reserve(plan.rows().len());
                self.visible_rows
                    .search_text
                    .extend(plan.rows().iter().map(|row| {
                        let mut projection = row.original_name().to_lowercase();
                        projection.push('\0');
                        projection.push_str(&row.proposed_name().to_lowercase());
                        projection
                    }));
            }
            self.visible_rows.search_plan_id = plan_id;
        }

        let row_count = self.plan.as_ref().map_or_else(
            || {
                if self.synthetic_fixture {
                    SAMPLE_COUNT
                } else {
                    0
                }
            },
            |plan| plan.rows().len(),
        );
        let query = self.source_query.trim().to_lowercase();
        let indices = (0..row_count)
            .filter(|index| {
                if let Some(plan) = &self.plan {
                    let Some(row) = plan.rows().get(*index) else {
                        return false;
                    };
                    let matches_filter = match self.filter {
                        PlanFilter::All => true,
                        PlanFilter::Changed => row.status() == "changed",
                        PlanFilter::Unchanged => row.status() == "unchanged",
                        PlanFilter::Blocked => row.status() == "blocked",
                    };
                    let matches_query = query.is_empty()
                        || self
                            .visible_rows
                            .search_text
                            .get(*index)
                            .is_some_and(|projection| projection.contains(&query));
                    let matches_diagnostic = self
                        .diagnostic_filter
                        .code()
                        .is_none_or(|code| row.diagnostics().contains(&code));
                    matches_filter && matches_query && matches_diagnostic
                } else if self.synthetic_fixture {
                    let blocked = Self::row_is_blocked(*index);
                    let matches_filter = match self.filter {
                        PlanFilter::All => true,
                        PlanFilter::Changed => !blocked,
                        PlanFilter::Unchanged => false,
                        PlanFilter::Blocked => blocked,
                    };
                    let matches_query = query.is_empty()
                        || format!("IMG_{index:05}.jpg")
                            .to_ascii_lowercase()
                            .contains(&query);
                    matches_filter && matches_query
                } else {
                    false
                }
            })
            .collect::<Arc<[_]>>();
        self.visible_rows.key = Some(VisibleRowsKey {
            plan_id,
            source_query: self.source_query.clone(),
            filter: self.filter,
            diagnostic_filter: self.diagnostic_filter,
            synthetic_fixture: self.synthetic_fixture,
        });
        self.visible_rows.indices = Arc::clone(&indices);
        indices
    }

    #[cfg(test)]
    fn admit_sources_immediately(&mut self, paths: Vec<PathBuf>) {
        self.supersede_pending_plan_refresh();
        let previous_source_count = self.plan.as_ref().map_or(0, |plan| plan.rows().len());
        let request = self.rule_request();
        let result = self.application.admit_sources_with_rules(paths, request);
        self.apply_admission_result(previous_source_count, result);
    }

    fn filesystem_authority_enabled(&self) -> bool {
        #[cfg(feature = "automation")]
        if self.automation_mode {
            return false;
        }
        true
    }

    fn start_source_admission(&mut self, paths: Vec<PathBuf>, context: &egui::Context) {
        if paths.is_empty() || !self.filesystem_authority_enabled() {
            return;
        }
        if self.admission_task.is_some() || self.planning_task.is_some() {
            self.supersede_pending_plan_refresh();
            let retained = MAX_ADMITTED_SOURCES
                .saturating_add(1)
                .saturating_sub(self.pending_admission_paths.len());
            self.pending_admission_paths
                .extend(paths.into_iter().take(retained));
            self.plan_is_current = false;
            self.status = self
                .locale
                .text(
                    "Adding entries · selection queued",
                    "항목 추가를 대기 중입니다",
                )
                .to_owned();
            context.request_repaint_after(PLANNING_POLL_INTERVAL);
            return;
        }
        self.launch_source_admission(paths, context);
    }

    fn launch_source_admission(&mut self, paths: Vec<PathBuf>, context: &egui::Context) {
        self.supersede_pending_plan_refresh();
        let revision = self.planning_revision;
        let previous_source_count = self.plan.as_ref().map_or(0, |plan| plan.rows().len());
        let request = self.rule_request();
        let application = Arc::clone(&self.application);
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = application.admit_sources_with_rules(paths, request);
            let _ = sender.send(AdmissionMessage {
                revision,
                previous_source_count,
                result,
            });
        });
        self.admission_task = Some(AdmissionTask {
            receiver,
            handle: Some(handle),
        });
        self.plan_is_current = false;
        self.rule_error = None;
        self.status = self
            .locale
            .text("Adding entries", "항목을 추가 중입니다")
            .to_owned();
        context.request_repaint_after(PLANNING_POLL_INTERVAL);
    }

    fn poll_admission(&mut self, context: &egui::Context) {
        let message = self
            .admission_task
            .as_ref()
            .map(|task| task.receiver.try_recv());
        match message {
            Some(Ok(message)) => {
                if let Some(task) = self.admission_task.take() {
                    task.finish();
                }
                let has_pending = !self.pending_admission_paths.is_empty();
                if message.revision == self.planning_revision && !has_pending {
                    self.apply_admission_result(message.previous_source_count, message.result);
                } else {
                    match message.result {
                        Ok(plan) => {
                            self.plan = Some(plan);
                            self.plan_is_current = false;
                            if !has_pending {
                                self.planning_due = Some(Instant::now());
                            }
                        }
                        Err(error) => self.apply_admission_error(&error),
                    }
                }
                if has_pending {
                    let paths = std::mem::take(&mut self.pending_admission_paths);
                    self.launch_source_admission(paths, context);
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                if let Some(task) = self.admission_task.take() {
                    task.finish();
                }
                self.plan_is_current = false;
                self.status = self
                    .locale
                    .text(
                        "Source admission ended unexpectedly",
                        "항목 추가가 예기치 않게 종료되었습니다",
                    )
                    .to_owned();
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
        if self.admission_task.is_some() {
            context.request_repaint_after(PLANNING_POLL_INTERVAL);
        }
    }

    fn apply_admission_result(
        &mut self,
        previous_source_count: usize,
        result: Result<PlanDto, PlanningCommandErrorDto>,
    ) {
        match result {
            Ok(plan) => {
                let admitted_count = plan.rows().len().saturating_sub(previous_source_count);
                self.status = match self.locale {
                    Locale::English => format!(
                        "Added {admitted_count} entries · {}",
                        self.plan_status(&plan)
                    ),
                    Locale::Korean => {
                        format!("항목 {admitted_count}개 추가 · {}", self.plan_status(&plan))
                    }
                };
                self.plan = Some(plan);
                self.plan_is_current = true;
            }
            Err(error) => self.apply_admission_error(&error),
        }
    }

    fn apply_admission_error(&mut self, error: &PlanningCommandErrorDto) {
        self.plan_is_current = false;
        self.status = planning_error_message(error.code(), self.locale).to_owned();
    }

    fn refresh_plan(&mut self) {
        self.supersede_pending_plan_refresh();
        let request = self.rule_request();
        if self.plan.is_none() {
            match self.application.validate_rule_request(&request) {
                Ok(()) => {
                    self.rule_error = None;
                    self.plan_is_current = true;
                }
                Err(error) => self.apply_planning_error(&error),
            }
            return;
        }
        match self.application.preview_rules(request) {
            Ok(plan) => {
                self.rule_error = None;
                self.status = self.plan_status(&plan);
                self.plan = Some(plan);
                self.plan_is_current = true;
            }
            Err(error) => self.apply_planning_error(&error),
        }
    }

    fn supersede_pending_plan_refresh(&mut self) {
        self.planning_revision = self.planning_revision.saturating_add(1);
        self.planning_due = None;
        if matches!(
            self.pending_confirmation.as_ref(),
            Some(PendingConfirmation::Apply { .. })
        ) {
            self.pending_confirmation = None;
        }
    }

    fn schedule_plan_refresh(&mut self, context: &egui::Context) {
        self.planning_revision = self.planning_revision.saturating_add(1);
        self.planning_due = Some(Instant::now() + PLANNING_DEBOUNCE);
        self.plan_is_current = false;
        self.rule_error = None;
        if matches!(
            self.pending_confirmation.as_ref(),
            Some(PendingConfirmation::Apply { .. })
        ) {
            self.pending_confirmation = None;
        }
        self.status = self
            .locale
            .text("Updating preview", "미리보기를 갱신 중입니다")
            .to_owned();
        context.request_repaint_after(PLANNING_DEBOUNCE);
    }

    fn poll_planning(&mut self, context: &egui::Context) {
        let message = self
            .planning_task
            .as_ref()
            .map(|task| task.receiver.try_recv());
        match message {
            Some(Ok(message)) => {
                if let Some(task) = self.planning_task.take() {
                    task.finish();
                }
                if message.revision == self.planning_revision {
                    match message.outcome {
                        PlanningOutcome::Preview(Ok(plan)) => {
                            self.rule_error = None;
                            self.status = self.plan_status(&plan);
                            self.plan = Some(plan);
                            self.plan_is_current = true;
                        }
                        PlanningOutcome::Validation(Ok(())) => {
                            self.rule_error = None;
                            self.plan_is_current = true;
                        }
                        PlanningOutcome::Preview(Err(error))
                        | PlanningOutcome::Validation(Err(error)) => {
                            self.apply_planning_error(&error);
                        }
                    }
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                if let Some(task) = self.planning_task.take() {
                    task.finish();
                }
                self.plan_is_current = false;
                self.status = self
                    .locale
                    .text(
                        "Preview calculation ended unexpectedly",
                        "미리보기 계산이 예기치 않게 종료되었습니다",
                    )
                    .to_owned();
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }

        if self.admission_task.is_none()
            && self.planning_task.is_none()
            && !self.pending_admission_paths.is_empty()
        {
            let paths = std::mem::take(&mut self.pending_admission_paths);
            self.launch_source_admission(paths, context);
        }

        if self.admission_task.is_none()
            && self.planning_task.is_none()
            && self
                .planning_due
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.planning_due = None;
            let revision = self.planning_revision;
            let request = self.rule_request();
            let has_plan = self.plan.is_some();
            let application = Arc::clone(&self.application);
            let (sender, receiver) = mpsc::channel();
            let handle = thread::spawn(move || {
                let outcome = if has_plan {
                    PlanningOutcome::Preview(application.preview_rules(request))
                } else {
                    PlanningOutcome::Validation(application.validate_rule_request(&request))
                };
                let _ = sender.send(PlanningMessage { revision, outcome });
            });
            self.planning_task = Some(PlanningTask {
                receiver,
                handle: Some(handle),
            });
        }

        if self.planning_task.is_some() {
            context.request_repaint_after(PLANNING_POLL_INTERVAL);
        } else if let Some(deadline) = self.planning_due {
            context.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
        }
    }

    fn apply_planning_error(&mut self, error: &PlanningCommandErrorDto) {
        let message = planning_error_message(error.code(), self.locale).to_owned();
        self.rule_error = Some(message.clone());
        self.plan_is_current = false;
        self.status = message;
    }

    fn plan_status(&self, plan: &PlanDto) -> String {
        match self.locale {
            Locale::English => format!(
                "{} entries · {} will change · {} cannot apply",
                plan.rows().len(),
                plan.changed_count(),
                plan.blocked_count()
            ),
            Locale::Korean => format!(
                "전체 {}개 · 변경 예정 {}개 · 적용 불가 {}개",
                plan.rows().len(),
                plan.changed_count(),
                plan.blocked_count()
            ),
        }
    }

    fn rule_request(&self) -> RulePipelineRequestDto {
        RulePipelineRequestDto::new(
            self.rules.clone(),
            self.overrides
                .iter()
                .map(|(source_id, value)| SourceOverrideDto::new(*source_id, value.clone()))
                .collect(),
        )
    }

    #[cfg(any(test, feature = "automation"))]
    #[cfg(test)]
    fn set_prefix(&mut self, prefix: &str) {
        if let Some(RuleRequestDto::Prefix { value, .. }) = self
            .rules
            .iter_mut()
            .find(|rule| matches!(rule, RuleRequestDto::Prefix { .. }))
        {
            prefix.clone_into(value);
            return;
        }
        self.rules.push(RuleRequestDto::Prefix {
            rule_id: self.next_rule_id,
            enabled: true,
            value: prefix.to_owned(),
        });
        self.next_rule_id = self.next_rule_id.saturating_add(1);
        self.selected_rule = self.rules.len().saturating_sub(1);
    }

    fn add_rule(&mut self, kind: RuleKind) {
        if self.rules.len() >= 32 {
            self.status = self
                .locale
                .text(
                    "The rule limit has been reached",
                    "규칙 개수 한도에 도달했습니다",
                )
                .to_owned();
            return;
        }
        let rule_id = self.next_rule_id;
        self.rules.push(kind.create(rule_id));
        self.next_rule_id = self.next_rule_id.saturating_add(1);
        self.selected_rule = self.rules.len().saturating_sub(1);
        self.rule_editor_open = true;
        self.focused_rule_id = Some(rule_id);
        self.draft_rule_id = Some(rule_id);
        self.draft_rule_changed = false;
        self.refresh_plan();
    }

    fn finish_rule_edit(&mut self) {
        self.rule_editor_open = false;
        self.focused_rule_id = None;
        self.draft_rule_id = None;
        self.draft_rule_changed = false;
    }

    fn cancel_rule_edit(&mut self) {
        if let Some(draft_rule_id) = self.draft_rule_id
            && !self.draft_rule_changed
            && let Some(index) = self
                .rules
                .iter()
                .position(|rule| rule.rule_id() == draft_rule_id)
        {
            self.rules.remove(index);
            self.selected_rule = self.selected_rule.min(self.rules.len().saturating_sub(1));
            self.refresh_plan();
        }
        self.finish_rule_edit();
    }

    fn remove_rule(&mut self, index: usize) {
        if index >= self.rules.len() {
            return;
        }
        let removed_rule_id = self.rules[index].rule_id();
        self.rules.remove(index);
        self.selected_rule = self.selected_rule.min(self.rules.len().saturating_sub(1));
        if self.rules.is_empty() || self.draft_rule_id == Some(removed_rule_id) {
            self.finish_rule_edit();
        }
        self.refresh_plan();
    }

    fn move_rule_to_insertion(&mut self, rule_id: u64, insertion_index: usize) -> bool {
        let Some(source_index) = self.rules.iter().position(|rule| rule.rule_id() == rule_id)
        else {
            return false;
        };
        let selected_rule_id = self
            .rules
            .get(self.selected_rule)
            .map(RuleRequestDto::rule_id);
        let bounded_insertion = insertion_index.min(self.rules.len());
        let destination_index = if source_index < bounded_insertion {
            bounded_insertion.saturating_sub(1)
        } else {
            bounded_insertion
        };
        if source_index == destination_index {
            return false;
        }

        let rule = self.rules.remove(source_index);
        self.rules.insert(destination_index, rule);
        if let Some(selected_rule_id) = selected_rule_id
            && let Some(selected_index) = self
                .rules
                .iter()
                .position(|rule| rule.rule_id() == selected_rule_id)
        {
            self.selected_rule = selected_index;
        }
        self.refresh_plan();
        true
    }

    fn move_selected_rule(&mut self, direction: i8) -> bool {
        let Some(rule_id) = self
            .rules
            .get(self.selected_rule)
            .map(RuleRequestDto::rule_id)
        else {
            return false;
        };
        let insertion_index = match direction {
            -1 if self.selected_rule > 0 => self.selected_rule - 1,
            1 if self.selected_rule + 1 < self.rules.len() => self.selected_rule + 2,
            _ => return false,
        };
        self.move_rule_to_insertion(rule_id, insertion_index)
    }

    fn focus_rule(&mut self, rule_id: u64) {
        let Some(index) = self.rules.iter().position(|rule| rule.rule_id() == rule_id) else {
            return;
        };
        self.selected_rule = index;
        self.rule_editor_open = true;
        self.focused_rule_id = Some(rule_id);
        self.draft_rule_id = None;
        self.draft_rule_changed = false;
    }

    fn open_override_for_row(&mut self, row_index: usize) {
        let Some((source_id, original_name)) = self.plan.as_ref().and_then(|plan| {
            plan.rows()
                .get(row_index)
                .map(|row| (row.source_id(), row.original_name().to_owned()))
        }) else {
            return;
        };
        let value = self
            .overrides
            .get(&source_id)
            .cloned()
            .unwrap_or_else(|| original_name.clone());
        self.override_editor = Some(OverrideEditor {
            source_id,
            original_name,
            value,
        });
    }

    fn exclude_source_for_row(&mut self, row_index: usize) {
        let Some(source_id) = self
            .plan
            .as_ref()
            .and_then(|plan| plan.rows().get(row_index))
            .map(|row| row.source_id())
        else {
            return;
        };
        self.supersede_pending_plan_refresh();
        let previous_override = self.overrides.remove(&source_id);
        let request = self.rule_request();
        match self
            .application
            .exclude_sources_with_rules(&[source_id], request)
        {
            Ok(plan) => {
                self.status = match self.locale {
                    Locale::English => format!(
                        "Removed 1 entry from the plan · {}",
                        self.plan_status(&plan)
                    ),
                    Locale::Korean => {
                        format!("계획에서 항목 1개 제거 · {}", self.plan_status(&plan))
                    }
                };
                self.plan = Some(plan);
                self.plan_is_current = true;
                self.selected_preview_row = None;
            }
            Err(error) => {
                if let Some(previous_override) = previous_override {
                    self.overrides.insert(source_id, previous_override);
                }
                self.plan_is_current = false;
                self.status = planning_error_message(error.code(), self.locale).to_owned();
            }
        }
    }

    fn select_preview_filter(&mut self, filter: PlanFilter) {
        self.filter = filter;
        self.diagnostic_filter = DiagnosticFilter::All;
        self.source_query.clear();
        self.selected_preview_row = None;
        self.preview_focus_requested = true;
    }

    fn synthetic_prefix(&self) -> &str {
        self.rules
            .iter()
            .find_map(|rule| match rule {
                RuleRequestDto::Prefix {
                    enabled: true,
                    value,
                    ..
                } => Some(value.as_str()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn save_presets(&mut self, next: PresetDocumentDto, success: String) {
        if !self.filesystem_authority_enabled() {
            return;
        }
        if let Some(path) = &self.preset_path
            && let Err(error) = next.save(path)
        {
            self.status = preset_error_message(error.code(), self.locale).to_owned();
            return;
        }
        self.presets = next;
        self.status = success;
    }

    fn save_current_preset(&mut self) {
        let mut next = self.presets.clone();
        match next.add(&self.preset_name, &self.rules) {
            Ok(_) => {
                self.preset_name.clear();
                self.save_presets(
                    next,
                    self.locale
                        .text("Preset saved", "프리셋을 저장했습니다")
                        .to_owned(),
                );
            }
            Err(error) => self.status = preset_error_message(error.code(), self.locale).to_owned(),
        }
    }

    fn apply_preset(&mut self, preset_id: u64) {
        if !self.filesystem_authority_enabled() {
            return;
        }
        let Some((name, rules)) = self
            .presets
            .presets()
            .iter()
            .find(|preset| preset.preset_id() == preset_id)
            .map(|preset| (preset.name().to_owned(), preset.rules().to_vec()))
        else {
            return;
        };
        self.next_rule_id = rules
            .iter()
            .map(RuleRequestDto::rule_id)
            .max()
            .unwrap_or_default()
            .saturating_add(1);
        self.rules = rules;
        self.selected_rule = 0;
        self.rule_editor_open = false;
        self.focused_rule_id = None;
        self.draft_rule_id = None;
        self.draft_rule_changed = false;
        self.refresh_plan();
        self.status = format!(
            "{}: {name}",
            self.locale.text("Preset applied", "프리셋 적용")
        );
    }

    fn delete_preset(&mut self, preset_id: u64) {
        if !self.filesystem_authority_enabled() {
            return;
        }
        let mut next = self.presets.clone();
        next.remove(preset_id);
        self.save_presets(
            next,
            self.locale
                .text("Preset deleted", "프리셋을 삭제했습니다")
                .to_owned(),
        );
    }

    fn refresh_ledger(&mut self) {
        if !self.filesystem_authority_enabled() || self.journal_root.is_none() {
            self.status = self
                .locale
                .text(
                    "The journal location is unavailable",
                    "저널 위치를 사용할 수 없습니다",
                )
                .to_owned();
            return;
        }
        if self.ledger_task.is_some() {
            return;
        }
        self.ledger_ready = false;
        self.start_ledger_task(
            self.locale
                .text(
                    "Refreshing rename history",
                    "이름 변경 기록을 새로 고치는 중입니다",
                )
                .to_owned(),
            |application| LedgerMessage::Snapshot {
                result: application.list_ledger().map_err(|error| error.kind()),
                announce: true,
            },
        );
    }

    fn inspect_selected_recovery(&mut self) {
        if !self.filesystem_authority_enabled() {
            return;
        }
        let Some(ledger_id) = self.selected_ledger_id else {
            return;
        };
        if self.ledger_task.is_some() {
            return;
        }
        self.start_ledger_task(
            self.locale
                .text("Inspecting recovery state", "복구 상태를 검사 중입니다")
                .to_owned(),
            move |application| {
                LedgerMessage::Recovery(
                    application
                        .inspect_recovery(ledger_id, &NativeExecutionFileSystem::new())
                        .map_err(|error| error.kind()),
                )
            },
        );
    }

    fn inspect_selected_undo(&mut self) {
        if !self.filesystem_authority_enabled() {
            return;
        }
        let Some(ledger_id) = self.selected_ledger_id else {
            return;
        };
        if self.ledger_task.is_some() {
            return;
        }
        self.start_ledger_task(
            self.locale
                .text("Inspecting undo state", "실행 취소 상태를 검사 중입니다")
                .to_owned(),
            move |application| {
                LedgerMessage::Undo(
                    application
                        .inspect_undo(ledger_id, &NativeExecutionFileSystem::new())
                        .map_err(|error| error.code()),
                )
            },
        );
    }

    fn start_ledger_task<F>(&mut self, status: String, work: F)
    where
        F: FnOnce(Arc<ApplicationService>) -> LedgerMessage + Send + 'static,
    {
        let application = Arc::clone(&self.application);
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = sender.send(work(application));
        });
        self.ledger_task = Some(LedgerTask {
            receiver,
            handle: Some(handle),
        });
        self.status = status;
    }

    fn poll_ledger(&mut self, context: &egui::Context) {
        let message = self
            .ledger_task
            .as_ref()
            .map(|task| task.receiver.try_recv());
        match message {
            Some(Ok(message)) => {
                if let Some(task) = self.ledger_task.take() {
                    task.finish();
                }
                match message {
                    LedgerMessage::Snapshot {
                        result: Ok(ledger),
                        announce,
                    } => {
                        if self.selected_ledger_id.is_some_and(|selected| {
                            !ledger.iter().any(|entry| entry.ledger_id() == selected)
                        }) {
                            self.selected_ledger_id = None;
                            self.recovery_inspection = None;
                            self.undo_inspection = None;
                        }
                        self.ledger = ledger;
                        self.ledger_ready = true;
                        if !announce && self.plan.is_some() {
                            let status = self.status.clone();
                            self.refresh_plan();
                            if self.plan_is_current {
                                self.status = status;
                            }
                        }
                        if announce {
                            self.status = self
                                .locale
                                .text("Rename history is ready", "이름 변경 기록을 확인했습니다")
                                .to_owned();
                        }
                    }
                    LedgerMessage::Snapshot {
                        result: Err(error), ..
                    } => {
                        self.ledger_ready = false;
                        self.status = application_error_message(error, self.locale).to_owned();
                    }
                    LedgerMessage::Recovery(Ok(inspection)) => {
                        self.recovery_inspection = Some(inspection);
                        self.undo_inspection = None;
                    }
                    LedgerMessage::Recovery(Err(error)) => {
                        self.status = application_error_message(error, self.locale).to_owned();
                    }
                    LedgerMessage::Undo(Err(error)) => {
                        self.status = undo_error_message(error, self.locale).to_owned();
                    }
                    LedgerMessage::Undo(Ok(inspection)) => {
                        self.undo_inspection = Some(inspection);
                        self.recovery_inspection = None;
                    }
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                if let Some(task) = self.ledger_task.take() {
                    task.finish();
                }
                self.ledger_ready = false;
                self.status = self
                    .locale
                    .text(
                        "Rename history task ended unexpectedly",
                        "이름 변경 기록 작업이 예기치 않게 종료되었습니다",
                    )
                    .to_owned();
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
        if self.ledger_task.is_some() {
            context.request_repaint_after(LEDGER_POLL_INTERVAL);
        }
    }

    fn start_confirmed_mutation(&mut self, confirmation: PendingConfirmation) {
        if !self.filesystem_authority_enabled() {
            self.status = self
                .locale
                .text(
                    "Automation mode is read-only",
                    "자동화 모드는 읽기 전용입니다",
                )
                .to_owned();
            return;
        }
        if self.mutation_task.is_some() && !matches!(&confirmation, PendingConfirmation::Cancel) {
            self.status = self
                .locale
                .text(
                    "Another operation is already running",
                    "다른 작업이 이미 실행 중입니다",
                )
                .to_owned();
            return;
        }
        match confirmation {
            PendingConfirmation::Apply {
                plan_id,
                changed_count: _,
            } => {
                let application = Arc::clone(&self.application);
                let (sender, receiver) = mpsc::channel();
                let handle = thread::spawn(move || {
                    let result = application.apply_latest_plan(
                        plan_id,
                        &NativeExecutionFileSystem::new(),
                        || false,
                    );
                    let _ = sender.send(MutationMessage::Apply(result));
                });
                self.mutation_task = Some(MutationTask {
                    receiver,
                    handle: Some(handle),
                });
                self.status = self
                    .locale
                    .text("Applying the current plan", "현재 계획을 적용 중입니다")
                    .to_owned();
            }
            PendingConfirmation::Recovery { action, inspection } => {
                let request = RecoveryRequestDto::new(action, &inspection);
                let application = Arc::clone(&self.application);
                let (sender, receiver) = mpsc::channel();
                let handle = thread::spawn(move || {
                    let result = application.apply_recovery_action(
                        &request,
                        &NativeExecutionFileSystem::new(),
                        |_, _| true,
                    );
                    let _ = sender.send(MutationMessage::Recovery(result));
                });
                self.mutation_task = Some(MutationTask {
                    receiver,
                    handle: Some(handle),
                });
                self.status = self
                    .locale
                    .text("Recovery operation running", "복구 작업을 실행 중입니다")
                    .to_owned();
            }
            PendingConfirmation::Undo { inspection } => {
                let request = UndoRequestDto::new(inspection);
                let application = Arc::clone(&self.application);
                let (sender, receiver) = mpsc::channel();
                let handle = thread::spawn(move || {
                    let result =
                        application
                            .apply_undo(&request, &NativeExecutionFileSystem::new(), |_| true);
                    let _ = sender.send(MutationMessage::Undo(result));
                });
                self.mutation_task = Some(MutationTask {
                    receiver,
                    handle: Some(handle),
                });
                self.status = self
                    .locale
                    .text("Undo operation running", "실행 취소 작업을 실행 중입니다")
                    .to_owned();
            }
            PendingConfirmation::Cancel => {
                match self.application.request_confirmed_cancellation(|| true) {
                    Ok(true) => {
                        self.status = self
                            .locale
                            .text("Cancellation requested", "취소를 요청했습니다")
                            .to_owned();
                    }
                    Ok(false) => {
                        self.status = self
                            .locale
                            .text("No cancellable operation", "취소 가능한 작업이 없습니다")
                            .to_owned();
                    }
                    Err(error) => {
                        self.status = recovery_error_message(error.code(), self.locale).to_owned()
                    }
                }
            }
        }
    }

    fn poll_mutation(&mut self, context: &egui::Context) {
        let Some(task) = self.mutation_task.as_ref() else {
            return;
        };
        match task.receiver.try_recv() {
            Ok(MutationMessage::Apply(result)) => {
                if let Some(task) = self.mutation_task.take() {
                    task.finish();
                }
                self.status = match result {
                    Ok(result) => {
                        self.plan = None;
                        self.overrides.clear();
                        format!(
                            "{}: {}",
                            self.locale.text("Rename finished", "이름 변경 완료"),
                            outcome_label(result.outcome(), self.locale)
                        )
                    }
                    Err(error) => apply_error_message(error.code(), self.locale).to_owned(),
                };
                self.refresh_ledger();
            }
            Ok(MutationMessage::Recovery(result)) => {
                if let Some(task) = self.mutation_task.take() {
                    task.finish();
                }
                self.status = match result {
                    Ok(result) => format!(
                        "{}: {}",
                        self.locale.text("Recovery finished", "복구 완료"),
                        outcome_label(result.outcome(), self.locale)
                    ),
                    Err(error) => recovery_error_message(error.code(), self.locale).to_owned(),
                };
                self.refresh_ledger();
            }
            Ok(MutationMessage::Undo(result)) => {
                if let Some(task) = self.mutation_task.take() {
                    task.finish();
                }
                self.status = match result {
                    Ok(result) => format!(
                        "{}: {}",
                        self.locale.text("Undo finished", "실행 취소 완료"),
                        outcome_label(result.outcome(), self.locale)
                    ),
                    Err(error) => undo_error_message(error.code(), self.locale).to_owned(),
                };
                self.refresh_ledger();
            }
            Err(TryRecvError::Empty) => context.request_repaint_after(MUTATION_POLL_INTERVAL),
            Err(TryRecvError::Disconnected) => {
                if let Some(task) = self.mutation_task.take() {
                    task.finish();
                }
                self.status = self
                    .locale
                    .text(
                        "The operation ended unexpectedly",
                        "작업이 예기치 않게 종료되었습니다",
                    )
                    .to_owned();
            }
        }
    }

    fn show_ledger(&mut self, ui: &mut egui::Ui) {
        let filesystem_authority = self.filesystem_authority_enabled();
        ui.horizontal(|ui| {
            ui.heading(self.locale.text(semantics::LEDGER, "이름 변경 기록"));
            if ui
                .add_enabled(
                    filesystem_authority && self.ledger_task.is_none(),
                    egui::Button::new(
                        self.locale
                            .text(semantics::REFRESH_LEDGER, "이름 변경 기록 새로 고침"),
                    ),
                )
                .clicked()
            {
                self.refresh_ledger();
            }
        });
        if self.ledger_task.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    self.locale
                        .text("Checking rename history", "이름 변경 기록을 확인 중입니다"),
                );
            });
        }
        if self.ledger.is_empty() {
            if self.ledger_task.is_none() {
                ui.label(self.locale.text(
                    "No name changes have been recorded yet.",
                    "아직 이름 변경 기록이 없습니다.",
                ));
            }
            return;
        }

        ui.label(
            RichText::new(self.locale.text(
                "Up/Down: select · Enter: inspect available action",
                "위/아래: 선택 · Enter: 가능한 작업 검사",
            ))
            .color(self.palette.ink_soft),
        );
        let move_selection_up = ui
            .ctx()
            .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        let move_selection_down = ui
            .ctx()
            .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let inspect_selection = ui
            .ctx()
            .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let mut selected = self.selected_ledger_id;
        let current_index = selected.and_then(|ledger_id| {
            self.ledger
                .iter()
                .position(|entry| entry.ledger_id() == ledger_id)
        });
        let direction = if move_selection_down {
            1
        } else if move_selection_up {
            -1
        } else {
            0
        };
        if let Some(next_index) =
            adjacent_selection_index(self.ledger.len(), current_index, direction)
            && direction != 0
        {
            selected = self.ledger.get(next_index).map(LedgerEntryDto::ledger_id);
        }
        ScrollArea::vertical().max_height(240.0).show_rows(
            ui,
            self.appearance.density.preview_row_height(),
            self.ledger.len(),
            |ui, row_range| {
                for index in row_range {
                    let entry = &self.ledger[index];
                    let label = match self.locale {
                        Locale::English => format!(
                            "Name change #{} · {} · {}",
                            entry.ledger_id(),
                            ledger_status_label(entry.status(), self.locale),
                            entry_count_label(entry.source_count(), self.locale)
                        ),
                        Locale::Korean => format!(
                            "이름 변경 #{} · {} · {}",
                            entry.ledger_id(),
                            ledger_status_label(entry.status(), self.locale),
                            entry_count_label(entry.source_count(), self.locale)
                        ),
                    };
                    ui.selectable_value(&mut selected, Some(entry.ledger_id()), label);
                }
            },
        );
        if selected != self.selected_ledger_id {
            self.selected_ledger_id = selected;
            self.recovery_inspection = None;
            self.undo_inspection = None;
        }

        let mutation_idle = filesystem_authority
            && self.mutation_task.is_none()
            && self.ledger_task.is_none()
            && self.ledger_ready;
        let (recovery_available, undo_available) = self
            .selected_ledger_id
            .and_then(|ledger_id| {
                self.ledger
                    .iter()
                    .find(|entry| entry.ledger_id() == ledger_id)
            })
            .map_or((false, false), |entry| {
                (entry.recovery_available(), entry.undo_available())
            });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    recovery_available && mutation_idle,
                    egui::Button::new(
                        self.locale
                            .text(semantics::INSPECT_RECOVERY, "복구 방법 확인"),
                    ),
                )
                .clicked()
            {
                self.inspect_selected_recovery();
            }
            if ui
                .add_enabled(
                    undo_available && mutation_idle,
                    egui::Button::new(self.locale.text(semantics::INSPECT_UNDO, "되돌리기 확인")),
                )
                .clicked()
            {
                self.inspect_selected_undo();
            }
        });
        if inspect_selection && mutation_idle {
            if undo_available {
                self.inspect_selected_undo();
            } else if recovery_available {
                self.inspect_selected_recovery();
            }
        }

        if let Some(inspection) = &self.recovery_inspection {
            ui.separator();
            ui.label(format!(
                "{} · {}",
                recovery_direction_label(inspection.direction(), self.locale),
                recovery_readiness_label(inspection.readiness(), self.locale),
            ));
            if let Some(step) = inspection.step_index() {
                ui.label(match self.locale {
                    Locale::English => {
                        format!("Interrupted at step {}", step.saturating_add(1))
                    }
                    Locale::Korean => format!("{}단계에서 중단됨", step.saturating_add(1)),
                });
            }
            let mut requested_action = None;
            ui.horizontal(|ui| {
                for (action, label, enabled) in [
                    (
                        RecoveryCommandAction::Resume,
                        recovery_action_label(RecoveryCommandAction::Resume, self.locale),
                        inspection.resume_available(),
                    ),
                    (
                        RecoveryCommandAction::Rollback,
                        recovery_action_label(RecoveryCommandAction::Rollback, self.locale),
                        inspection.rollback_available(),
                    ),
                    (
                        RecoveryCommandAction::Reconcile,
                        recovery_action_label(RecoveryCommandAction::Reconcile, self.locale),
                        inspection.reconcile_available(),
                    ),
                ] {
                    if ui
                        .add_enabled(enabled && mutation_idle, egui::Button::new(label))
                        .clicked()
                    {
                        requested_action = Some(action);
                    }
                }
            });
            if let Some(action) = requested_action {
                self.pending_confirmation = Some(PendingConfirmation::Recovery {
                    action,
                    inspection: inspection.clone(),
                });
            }
        }

        if let Some(inspection) = &self.undo_inspection {
            ui.separator();
            ui.label(format!(
                "{} · {}",
                undo_readiness_label(inspection.readiness(), self.locale),
                entry_count_label(inspection.source_count(), self.locale)
            ));
            if let Some(reason) = inspection.block_reason() {
                ui.label(undo_block_reason_label(reason, self.locale));
            }
            if ui
                .add_enabled(
                    inspection.undo_available() && mutation_idle,
                    egui::Button::new(self.locale.text(semantics::UNDO, "이 이름 변경 되돌리기")),
                )
                .clicked()
            {
                self.pending_confirmation = Some(PendingConfirmation::Undo {
                    inspection: inspection.clone(),
                });
            }
        }

        if self.mutation_task.is_some()
            && ui
                .button(self.locale.text(semantics::CANCEL_MUTATION, "작업 취소"))
                .clicked()
        {
            self.pending_confirmation = Some(PendingConfirmation::Cancel);
        }
    }

    fn choose_files(&mut self, context: &egui::Context) {
        if !self.filesystem_authority_enabled() {
            return;
        }
        match rfd::FileDialog::new()
            .set_title(
                self.locale
                    .text("Add files to Renamewright", "이름을 바꿀 파일 추가"),
            )
            .pick_files()
        {
            Some(paths) => self.start_source_admission(paths, context),
            None => {
                self.status = self
                    .locale
                    .text("File selection cancelled", "파일 선택을 취소했습니다")
                    .to_owned();
            }
        }
    }

    fn choose_folder_entry(&mut self, context: &egui::Context) {
        if !self.filesystem_authority_enabled() {
            return;
        }
        match rfd::FileDialog::new()
            .set_title(
                self.locale
                    .text("Add a folder name to Renamewright", "이름을 바꿀 폴더 추가"),
            )
            .pick_folder()
        {
            Some(path) => self.start_source_admission(vec![path], context),
            None => {
                self.status = self
                    .locale
                    .text("Folder selection cancelled", "폴더 선택을 취소했습니다")
                    .to_owned();
            }
        }
    }

    fn apply_appearance(&mut self, context: &egui::Context) {
        if self.native_palette.high_contrast {
            if !self.appearance_applied || self.palette != self.native_palette {
                self.palette = self.native_palette;
                install_theme_with_density(context, self.palette, self.appearance.density);
                self.appearance_applied = true;
            }
            return;
        }

        let theme = self.appearance.theme.effective(context);
        let palette = NativePalette::for_theme(theme, self.appearance.accent);
        if !self.appearance_applied || self.palette != palette {
            self.palette = palette;
            install_theme_with_density(context, palette, self.appearance.density);
            context.options_mut(|options| options.fallback_theme = egui::Theme::Light);
            context.set_theme(self.appearance.theme.preference());
            self.appearance_applied = true;
        }
    }

    fn ensure_korean_font(&mut self, context: &egui::Context) {
        if self.korean_font_state != KoreanFontState::Idle {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        match thread::Builder::new()
            .name("renamewright-font-loader".to_owned())
            .spawn(move || {
                let fonts = SystemFallbackFonts {
                    korean: read_first_system_font(&KOREAN_FONT_CANDIDATES),
                    emoji: read_first_system_font(&EMOJI_FONT_CANDIDATES),
                };
                let _ = sender.send(fonts);
            }) {
            Ok(handle) => {
                self.korean_font_state = KoreanFontState::Loading;
                self.korean_font_task = Some(KoreanFontTask {
                    receiver,
                    handle: Some(handle),
                });
                context.request_repaint_after(FONT_POLL_INTERVAL);
            }
            Err(error) => {
                self.korean_font_state = KoreanFontState::Unavailable;
                eprintln!("Korean fallback font loading was not started: {error}");
            }
        }
    }

    fn poll_korean_font(&mut self, context: &egui::Context) {
        let Some(task) = self.korean_font_task.as_ref() else {
            return;
        };
        match task.receiver.try_recv() {
            Ok(fonts) => {
                if let Some(font) = fonts.korean {
                    eprintln!("loaded Korean fallback font: {}", font.file_name);
                    add_system_fallback_font(context, "system-korean", font);
                    self.korean_font_state = KoreanFontState::Ready;
                } else {
                    self.korean_font_state = KoreanFontState::Unavailable;
                    eprintln!("no Korean fallback font was found; IME text remains inspectable");
                }
                if let Some(font) = fonts.emoji {
                    eprintln!("loaded emoji fallback font: {}", font.file_name);
                    add_system_fallback_font(context, "system-emoji", font);
                }
                if let Some(task) = self.korean_font_task.take() {
                    task.finish();
                }
                context.request_repaint();
            }
            Err(TryRecvError::Disconnected) => {
                self.korean_font_state = KoreanFontState::Unavailable;
                if let Some(task) = self.korean_font_task.take() {
                    task.finish();
                }
                eprintln!("system fallback font loading ended before producing a result");
            }
            Err(TryRecvError::Empty) => context.request_repaint_after(FONT_POLL_INTERVAL),
        }
    }

    fn show_appearance_menu(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.menu_button(
            self.locale.text(semantics::APPEARANCE, "화면 설정"),
            |ui| {
                ui.set_min_width(180.0);
                ui.label(
                    RichText::new(self.locale.text("Theme", "테마"))
                        .strong()
                        .color(self.palette.ink),
                );
                ui.add_enabled_ui(!self.native_palette.high_contrast, |ui| {
                    for theme in AppearanceTheme::ALL {
                        changed |= ui
                            .selectable_value(
                                &mut self.appearance.theme,
                                theme,
                                theme.label(self.locale),
                            )
                            .changed();
                    }
                });
                if self.native_palette.high_contrast {
                    ui.separator();
                    ui.label(
                        RichText::new(self.locale.text(
                            semantics::HIGH_CONTRAST_OVERRIDES_APPEARANCE,
                            "Windows 고대비 설정이 화면 색상보다 우선합니다",
                        ))
                        .color(self.palette.ink),
                    );
                }
                ui.separator();
                if ui
                    .button(
                        self.locale
                            .text(semantics::ADVANCED_APPEARANCE, "고급 화면 설정"),
                    )
                    .clicked()
                {
                    self.appearance_advanced_open = true;
                    self.ledger_open = false;
                    ui.close();
                }
            },
        );
        changed
    }

    fn show_advanced_appearance(&mut self, ui: &mut egui::Ui, scroll_id: &'static str) {
        ui.horizontal(|ui| {
            ui.heading(
                RichText::new(
                    self.locale
                        .text(semantics::ADVANCED_APPEARANCE, "고급 화면 설정"),
                )
                .color(self.palette.ink),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button(
                        self.locale
                            .text(semantics::CLOSE_APPEARANCE, "화면 설정 닫기"),
                    )
                    .clicked()
                {
                    self.appearance_advanced_open = false;
                }
            });
        });
        ui.label(
            RichText::new(self.locale.text(
                "These settings change the workbench view, never the rename plan.",
                "화면 표시 방식만 바뀌며 이름 변경 내용에는 영향을 주지 않습니다.",
            ))
            .color(self.palette.ink_soft),
        );

        ScrollArea::vertical()
            .id_salt(scroll_id)
            .auto_shrink([false, false])
            .show(ui, |ui| self.show_advanced_appearance_options(ui));
    }

    fn show_advanced_appearance_options(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;

        ui.add_space(8.0);
        ui.separator();
        ui.label(
            RichText::new(self.locale.text(semantics::ACCENT_COLOR, "강조 색상"))
                .strong()
                .color(self.palette.ink),
        );
        ui.add_enabled_ui(!self.native_palette.high_contrast, |ui| {
            ui.horizontal_wrapped(|ui| {
                for accent in AccentChoice::ALL {
                    changed |= ui
                        .selectable_value(
                            &mut self.appearance.accent,
                            accent,
                            accent.label(self.locale),
                        )
                        .changed();
                }
            });
        });
        if self.native_palette.high_contrast {
            ui.label(
                RichText::new(self.locale.text(
                    semantics::HIGH_CONTRAST_OVERRIDES_APPEARANCE,
                    "Windows 고대비 설정이 화면 색상보다 우선합니다",
                ))
                .color(self.palette.ink),
            );
        }

        ui.add_space(8.0);
        ui.label(
            RichText::new(self.locale.text(semantics::DENSITY, "간격"))
                .strong()
                .color(self.palette.ink),
        );
        ui.horizontal(|ui| {
            for density in InterfaceDensity::ALL {
                changed |= ui
                    .selectable_value(
                        &mut self.appearance.density,
                        density,
                        density.label(self.locale),
                    )
                    .changed();
            }
        });

        ui.add_space(8.0);
        ui.label(
            RichText::new(self.locale.text(semantics::PREVIEW_COLUMNS, "미리보기 열"))
                .strong()
                .color(self.palette.ink),
        );
        changed |= ui
            .checkbox(
                &mut self.appearance.show_kind,
                self.locale.text(semantics::SHOW_KIND, "항목 종류 표시"),
            )
            .changed();
        changed |= ui
            .checkbox(
                &mut self.appearance.show_diagnostics,
                self.locale
                    .text(semantics::SHOW_DIAGNOSTICS, "모든 진단 세부 정보 표시"),
            )
            .changed();
        ui.label(
            RichText::new(self.locale.text(
                "Source, proposed name, status, and blocker reasons always remain visible.",
                "현재 이름, 바뀔 이름, 결과와 적용할 수 없는 이유는 항상 표시됩니다.",
            ))
            .color(self.palette.ink_soft),
        );

        ui.add_space(10.0);
        egui::Frame::new()
            .fill(self.palette.paper_raised)
            .stroke(Stroke::new(1.0, self.palette.rule))
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(self.locale.text("Live preview", "실시간 미리보기"))
                        .strong()
                        .color(self.palette.ink),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("IMG_00001.jpg").color(self.palette.ink));
                    ui.label(RichText::new("->").color(self.palette.ink_soft));
                    ui.label(RichText::new("Trip_0001.jpg").color(self.palette.accent));
                    ui.label(
                        RichText::new(self.locale.text("Will change", "변경 예정"))
                            .color(self.palette.accent)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(self.locale.text("Cannot apply", "적용 불가"))
                            .color(self.palette.blocked)
                            .strong(),
                    );
                });
            });

        ui.add_space(10.0);
        if ui
            .button(
                self.locale
                    .text(semantics::RESET_APPEARANCE, "고급 화면 설정 초기화"),
            )
            .clicked()
        {
            self.appearance.reset_advanced();
            changed = true;
        }

        if changed {
            self.appearance_applied = false;
            self.apply_appearance(ui.ctx());
            ui.ctx().request_repaint();
        }
    }

    fn show_source_bar(&mut self, ui: &mut egui::Ui) {
        let mut appearance_changed = false;
        let compact = ui.available_width() < 980.0;
        ui.horizontal(|ui| {
            ui.heading(RichText::new(semantics::PRODUCT_NAME).color(self.palette.ink));
            if !compact {
                ui.label(
                    RichText::new(
                        self.locale
                            .text(semantics::TAGLINE, "모든 이름 변경을 계획하세요."),
                    )
                    .color(self.palette.ink_soft),
                );
                let source_count = self.plan.as_ref().map_or_else(
                    || {
                        if self.synthetic_fixture {
                            SAMPLE_COUNT
                        } else {
                            0
                        }
                    },
                    |plan| plan.rows().len(),
                );
                ui.label(
                    RichText::new(entry_count_label(source_count, self.locale))
                        .color(self.palette.ink_soft),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let korean_font_state = self.korean_font_state;
                let language_menu = egui::ComboBox::from_id_salt("locale")
                    .selected_text(self.locale.picker_label(korean_font_state))
                    .show_ui(ui, |ui| {
                        for locale in Locale::ALL {
                            ui.add_enabled_ui(locale.picker_enabled(korean_font_state), |ui| {
                                ui.selectable_value(
                                    &mut self.locale,
                                    locale,
                                    locale.picker_label(korean_font_state),
                                );
                            });
                        }
                    });
                let language_menu_open = language_menu.inner.is_some();
                language_menu
                    .response
                    .on_hover_text(self.locale.text(semantics::LANGUAGE, "언어"));
                if language_menu_open {
                    self.ensure_korean_font(ui.ctx());
                }
                match self.korean_font_state {
                    KoreanFontState::Loading => {
                        ui.spinner();
                        ui.label(
                            self.locale
                                .text("Loading Korean text", "한글 글꼴 불러오는 중"),
                        );
                    }
                    KoreanFontState::Unavailable => {
                        ui.label(
                            RichText::new(
                                self.locale
                                    .text("Korean font unavailable", "한글 글꼴을 찾을 수 없음"),
                            )
                            .color(self.palette.blocked),
                        );
                    }
                    KoreanFontState::Idle | KoreanFontState::Ready => {}
                }
                appearance_changed |= self.show_appearance_menu(ui);
                let history_label = self.locale.text(semantics::HISTORY, "이름 변경 기록");
                let history = ui.selectable_label(self.ledger_open, history_label);
                history.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, history_label)
                });
                if history.clicked() {
                    self.ledger_open = !self.ledger_open;
                    if self.ledger_open {
                        self.appearance_advanced_open = false;
                    }
                }
                let add_folder = ui
                    .add_enabled(
                        self.filesystem_authority_enabled(),
                        egui::Button::new(
                            self.locale.text(semantics::ADD_FOLDER, "폴더 이름 추가"),
                        ),
                    )
                    .on_hover_text(self.locale.text(
                        "Adds only the folder itself, not the items inside it",
                        "폴더 자체만 추가하며 안의 항목은 포함하지 않습니다",
                    ));
                if add_folder.clicked() {
                    self.choose_folder_entry(ui.ctx());
                }
                if ui
                    .add_enabled(
                        self.filesystem_authority_enabled(),
                        egui::Button::new(self.locale.text(semantics::ADD_FILES, "파일 추가")),
                    )
                    .clicked()
                {
                    self.choose_files(ui.ctx());
                }
            });
        });
        if appearance_changed {
            self.appearance_applied = false;
            self.apply_appearance(ui.ctx());
            ui.ctx().request_repaint();
        }
        if self.locale == Locale::Korean {
            self.ensure_korean_font(ui.ctx());
        }
    }

    fn show_source_drop_overlay(&self, context: &egui::Context, hovered_count: usize) {
        let detail = match self.locale {
            Locale::English => format!(
                "Release to add {hovered_count} {}",
                if hovered_count == 1 {
                    "entry"
                } else {
                    "entries"
                }
            ),
            Locale::Korean => format!("놓아서 항목 {hovered_count}개 추가"),
        };
        egui::Area::new(egui::Id::new("source-drop-overlay"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(context, |ui| {
                egui::Frame::new()
                    .fill(self.palette.accent_fill)
                    .stroke(Stroke::new(3.0, self.palette.accent))
                    .corner_radius(12.0)
                    .inner_margin(24.0)
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.heading(
                                RichText::new(
                                    self.locale.text(
                                        "Add files or folder names",
                                        "파일 또는 폴더 이름 추가",
                                    ),
                                )
                                .color(self.palette.accent_text),
                            );
                            ui.label(RichText::new(detail).color(self.palette.accent_text));
                        });
                    });
            });
    }

    fn show_rule_command_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(
                RichText::new(
                    self.locale
                        .text(semantics::RULES_HEADING, "이름 바꾸기 규칙"),
                )
                .color(self.palette.ink),
            );
            ui.label(
                RichText::new(
                    self.locale
                        .text(semantics::RULES_ORDER_HELP, "앞의 규칙부터 차례로 반영"),
                )
                .color(self.palette.ink_soft),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .selectable_label(
                        self.tools_open,
                        self.locale.text(semantics::TOOLS, "고급 도구"),
                    )
                    .clicked()
                {
                    self.tools_open = !self.tools_open;
                }
            });
        });

        let mut add_kind = None;
        let mut editor_opened_this_frame = false;
        ui.horizontal_wrapped(|ui| {
            for kind in RuleKind::PRIMARY {
                if ui.button(kind.label(self.locale)).clicked() {
                    add_kind = Some(kind);
                }
            }
            ui.menu_button(self.locale.text(semantics::MORE_RULES, "더보기"), |ui| {
                for kind in RuleKind::SECONDARY {
                    if ui.button(kind.label(self.locale)).clicked() {
                        add_kind = Some(kind);
                        ui.close();
                    }
                }
            });
        });
        if let Some(kind) = add_kind {
            self.add_rule(kind);
            editor_opened_this_frame = true;
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(self.locale.text(semantics::ACTIVE_RULES, "규칙 순서"))
                    .strong()
                    .color(self.palette.ink),
            );
            if self.rules.is_empty() {
                ui.label(
                    RichText::new(self.locale.text(
                        "Choose a button above to add a rule",
                        "위에서 원하는 규칙을 선택하세요",
                    ))
                    .color(self.palette.ink_soft),
                );
            }
        });

        let mut selected_rule = None;
        let mut move_rule = None;
        let mut dropped_rule = None;
        let mut remove_rule = None;
        ScrollArea::horizontal()
            .id_salt("active-rule-chain")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, rule) in self.rules.iter().enumerate() {
                        let rule_card = ui.push_id(rule.rule_id(), |ui| {
                            egui::Frame::new()
                                .fill(self.palette.paper_raised)
                                .stroke(Stroke::new(1.0, self.palette.rule))
                                .inner_margin(4.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let drag_label = match self.locale {
                                            Locale::English => {
                                                format!("{} {}", semantics::DRAG_RULE, index + 1)
                                            }
                                            Locale::Korean => {
                                                format!("규칙 {} 드래그", index + 1)
                                            }
                                        };
                                        ui.dnd_drag_source(
                                            ui.id().with("rule-drag-handle"),
                                            RuleDragPayload {
                                                rule_id: rule.rule_id(),
                                            },
                                            |ui| {
                                                let drag_handle = ui.label("::");
                                                drag_handle.widget_info(|| {
                                                    egui::WidgetInfo::labeled(
                                                        egui::WidgetType::Other,
                                                        true,
                                                        drag_label.clone(),
                                                    )
                                                });
                                            },
                                        );
                                        let selected =
                                            self.rule_editor_open && self.selected_rule == index;
                                        if ui
                                            .selectable_label(
                                                selected,
                                                format!(
                                                    "{} · {}",
                                                    index + 1,
                                                    rule_summary(rule, self.locale)
                                                ),
                                            )
                                            .clicked()
                                        {
                                            selected_rule = Some(index);
                                        }
                                        let move_up = ui
                                            .add_enabled(index > 0, egui::Button::new("<"))
                                            .on_hover_text(self.locale.text(
                                                "Move earlier (Alt+Left)",
                                                "앞으로 이동 (Alt+왼쪽)",
                                            ));
                                        move_up.widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::WidgetType::Button,
                                                index > 0,
                                                self.locale.text(
                                                    semantics::MOVE_RULE_UP,
                                                    "규칙 앞으로 이동",
                                                ),
                                            )
                                        });
                                        if move_up.clicked() {
                                            move_rule = Some((index, index - 1));
                                        }
                                        let move_down = ui
                                            .add_enabled(
                                                index + 1 < self.rules.len(),
                                                egui::Button::new(">"),
                                            )
                                            .on_hover_text(self.locale.text(
                                                "Move later (Alt+Right)",
                                                "뒤로 이동 (Alt+오른쪽)",
                                            ));
                                        move_down.widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::WidgetType::Button,
                                                index + 1 < self.rules.len(),
                                                self.locale.text(
                                                    semantics::MOVE_RULE_DOWN,
                                                    "규칙 뒤로 이동",
                                                ),
                                            )
                                        });
                                        if move_down.clicked() {
                                            move_rule = Some((index, index + 1));
                                        }
                                        let remove = ui.button("×");
                                        remove.widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::WidgetType::Button,
                                                true,
                                                self.locale
                                                    .text(semantics::REMOVE_RULE, "규칙 제거"),
                                            )
                                        });
                                        if remove.clicked() {
                                            remove_rule = Some(index);
                                        }
                                    });
                                })
                        });
                        let card_response = &rule_card.inner.response;
                        let insertion_index = ui.ctx().pointer_interact_pos().map(|pointer| {
                            if pointer.x < card_response.rect.center().x {
                                index
                            } else {
                                index + 1
                            }
                        });
                        if card_response
                            .dnd_hover_payload::<RuleDragPayload>()
                            .is_some()
                            && let Some(insertion_index) = insertion_index
                        {
                            let marker_x = if insertion_index == index {
                                card_response.rect.left()
                            } else {
                                card_response.rect.right()
                            };
                            ui.painter().vline(
                                marker_x,
                                card_response.rect.y_range(),
                                Stroke::new(3.0, self.palette.accent),
                            );
                        }
                        if let (Some(payload), Some(insertion_index)) = (
                            card_response.dnd_release_payload::<RuleDragPayload>(),
                            insertion_index,
                        ) {
                            dropped_rule = Some((payload.rule_id, insertion_index));
                        }
                    }
                });
            });

        if let Some(index) = selected_rule {
            self.selected_rule = index;
            self.rule_editor_open = true;
            self.focused_rule_id = None;
            self.draft_rule_id = None;
            self.draft_rule_changed = false;
            editor_opened_this_frame = true;
        }
        if let Some((from, to)) = move_rule {
            self.rules.swap(from, to);
            self.selected_rule = to;
            self.refresh_plan();
        }
        if let Some((rule_id, insertion_index)) = dropped_rule {
            self.move_rule_to_insertion(rule_id, insertion_index);
        }
        if let Some(index) = remove_rule {
            self.remove_rule(index);
        }

        if self.rule_editor_open {
            let mut changed = false;
            let mut done = false;
            let mut cancel = false;
            let discard_new_rule = self.draft_rule_id.is_some() && !self.draft_rule_changed;
            if let Some(rule) = self.rules.get_mut(self.selected_rule) {
                let rule_id = rule.rule_id();
                let focus_first = self.focused_rule_id == Some(rule_id);
                egui::Frame::new()
                    .fill(self.palette.paper_soft)
                    .stroke(Stroke::new(1.0, self.palette.rule))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(rule_kind(rule).label(self.locale))
                                    .strong()
                                    .color(self.palette.ink),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if discard_new_rule
                                    && ui
                                        .button(self.locale.text(
                                            semantics::CANCEL_EDITING,
                                            "새 규칙 버리기",
                                        ))
                                        .clicked()
                                {
                                    cancel = true;
                                }
                                if ui
                                    .button(
                                        self.locale
                                            .text(semantics::DONE_EDITING, "편집기 닫기"),
                                    )
                                    .clicked()
                                {
                                    done = true;
                                }
                            });
                        });
                        changed = ui
                            .push_id(rule_id, |ui| {
                                rule_editor(ui, rule, self.locale, focus_first)
                            })
                            .inner;
                        if let Some(error) = &self.rule_error {
                            ui.label(
                                RichText::new(match self.locale {
                                    Locale::English => format!("Check this rule: {error}"),
                                    Locale::Korean => format!("이 규칙을 확인하세요: {error}"),
                                })
                                .color(self.palette.blocked),
                            );
                        }
                        ui.label(
                            RichText::new(if discard_new_rule {
                                self.locale.text(
                                    "Enter: close editor · Escape: discard new rule · preview updates while typing",
                                    "Enter: 편집기 닫기 · Esc: 새 규칙 버리기 · 입력 중 미리보기 갱신",
                                )
                            } else {
                                self.locale.text(
                                    "Enter or Escape: close editor · preview updates while typing",
                                    "Enter 또는 Esc: 편집기 닫기 · 입력 중 미리보기 갱신",
                                )
                            })
                            .color(self.palette.ink_soft),
                        );
                    });
                if focus_first {
                    self.focused_rule_id = None;
                }
            }
            if changed {
                if self.draft_rule_id.is_some() {
                    self.draft_rule_changed = true;
                }
                self.schedule_plan_refresh(ui.ctx());
            }
            if !editor_opened_this_frame {
                done |= ui.ctx().input(|input| input.key_pressed(egui::Key::Enter));
                cancel |= ui.ctx().input(|input| input.key_pressed(egui::Key::Escape));
            }
            if cancel {
                self.cancel_rule_edit();
            } else if done {
                self.finish_rule_edit();
            }
        }

        if self.tools_open {
            self.show_rule_tools(ui);
        }
    }

    fn show_rule_tools(&mut self, ui: &mut egui::Ui) {
        let filesystem_authority = self.filesystem_authority_enabled();
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(self.locale.text(semantics::PRESETS, "저장한 프리셋"))
                    .strong()
                    .color(self.palette.ink),
            );
            let preset_label = ui.label(self.locale.text(semantics::PRESET_NAME, "프리셋 이름"));
            ui.add_enabled(
                filesystem_authority,
                egui::TextEdit::singleline(&mut self.preset_name)
                    .id_salt("preset-name")
                    .desired_width(160.0)
                    .hint_text(self.locale.text("Name", "이름")),
            )
            .labelled_by(preset_label.id);
            if ui
                .add_enabled(
                    filesystem_authority,
                    egui::Button::new(self.locale.text(semantics::SAVE_PRESET, "프리셋 저장")),
                )
                .clicked()
            {
                self.save_current_preset();
            }
            if ui
                .add_enabled(
                    self.plan.is_some() && self.document_task.is_none(),
                    egui::Button::new(self.locale.text(semantics::INSPECT_JSON, "계획 JSON 보기")),
                )
                .clicked()
            {
                self.inspect_plan(true);
            }
            if ui
                .add_enabled(
                    self.plan.is_some() && self.document_task.is_none(),
                    egui::Button::new(self.locale.text(semantics::INSPECT_CSV, "계획 CSV 보기")),
                )
                .clicked()
            {
                self.inspect_plan(false);
            }
            if ui
                .add_enabled(
                    filesystem_authority && self.plan.is_some() && self.document_task.is_none(),
                    egui::Button::new(self.locale.text(semantics::EXPORT_JSON, "JSON 내보내기")),
                )
                .clicked()
            {
                self.export_plan(true);
            }
            if ui
                .add_enabled(
                    filesystem_authority && self.plan.is_some() && self.document_task.is_none(),
                    egui::Button::new(self.locale.text(semantics::EXPORT_CSV, "CSV 내보내기")),
                )
                .clicked()
            {
                self.export_plan(false);
            }
        });

        let mut apply_preset = None;
        let mut delete_preset = None;
        ScrollArea::horizontal()
            .id_salt("preset-list")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for preset in self.presets.presets() {
                        ui.push_id(preset.preset_id(), |ui| {
                            ui.label(preset.name());
                            if ui
                                .add_enabled(
                                    filesystem_authority,
                                    egui::Button::new(
                                        self.locale.text(semantics::APPLY_PRESET, "프리셋 적용"),
                                    ),
                                )
                                .clicked()
                            {
                                apply_preset = Some(preset.preset_id());
                            }
                            if ui
                                .add_enabled(
                                    filesystem_authority,
                                    egui::Button::new(
                                        self.locale.text(semantics::DELETE_PRESET, "프리셋 삭제"),
                                    ),
                                )
                                .clicked()
                            {
                                delete_preset = Some(preset.preset_id());
                            }
                            ui.separator();
                        });
                    }
                });
            });
        if let Some(preset_id) = apply_preset {
            self.apply_preset(preset_id);
        }
        if let Some(preset_id) = delete_preset {
            self.delete_preset(preset_id);
        }
    }

    fn show_preview(&mut self, ui: &mut egui::Ui) {
        let visible = self.visible_indices();
        if self.preview_focus_requested {
            self.selected_preview_row = visible.first().copied();
            if self.selected_preview_row.is_none() {
                self.preview_focus_requested = false;
            }
        }
        let kind_width = if self.appearance.show_kind {
            PREVIEW_KIND_COLUMN_WIDTH
        } else {
            0.0
        };
        let diagnostic_width = if self.appearance.show_diagnostics {
            260.0
        } else {
            180.0
        };
        let available_for_names =
            (ui.available_width() - kind_width - PREVIEW_STATUS_COLUMN_WIDTH - diagnostic_width)
                .max(
                    PREVIEW_SOURCE_COLUMN_WIDTH
                        + PREVIEW_PROPOSED_COLUMN_WIDTH
                        + PREVIEW_COLUMN_RESIZE_HANDLE_WIDTH,
                )
                - PREVIEW_COLUMN_RESIZE_HANDLE_WIDTH;
        let maximum_source_width =
            (available_for_names - PREVIEW_PROPOSED_COLUMN_WIDTH).max(PREVIEW_SOURCE_COLUMN_WIDTH);
        let source_column_width = self
            .appearance
            .preview_source_column_width
            .map_or(available_for_names * 0.45, f32::from)
            .clamp(PREVIEW_SOURCE_COLUMN_WIDTH, maximum_source_width);
        let proposed_column_width = available_for_names - source_column_width;
        ui.horizontal(|ui| {
            ui.heading(
                RichText::new(self.locale.text(semantics::PREVIEW_HEADING, "미리보기"))
                    .color(self.palette.ink),
            );
            ui.label(
                RichText::new(match self.locale {
                    Locale::English => format!("{} shown", visible.len()),
                    Locale::Korean => format!("{}개 표시", visible.len()),
                })
                .color(self.palette.ink_soft),
            );
            if self.appearance.preview_source_column_width.is_some()
                && ui
                    .small_button(self.locale.text("Reset column widths", "열 너비 초기화"))
                    .clicked()
            {
                self.appearance.preview_source_column_width = None;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for candidate in PlanFilter::ALL {
                if ui
                    .selectable_label(self.filter == candidate, candidate.label(self.locale))
                    .clicked()
                {
                    self.select_preview_filter(candidate);
                }
            }
            ui.separator();
            egui::ComboBox::from_id_salt("diagnostic-filter")
                .selected_text(self.diagnostic_filter.label(self.locale))
                .show_ui(ui, |ui| {
                    for candidate in DiagnosticFilter::ALL {
                        ui.selectable_value(
                            &mut self.diagnostic_filter,
                            candidate,
                            candidate.label(self.locale),
                        );
                    }
                })
                .response
                .on_hover_text(self.locale.text(semantics::DIAGNOSTIC_FILTER, "문제 필터"));
            ui.separator();
            let source_query_label =
                ui.label(self.locale.text(semantics::SOURCE_QUERY_LABEL, "이름 필터"));
            let source_query = ui
                .add(
                    egui::TextEdit::singleline(&mut self.source_query)
                        .id_salt("preview.source-query")
                        .hint_text(self.locale.text("Name contains", "이름에 포함")),
                )
                .labelled_by(source_query_label.id);
            if self.source_query_focus_requested {
                source_query.request_focus();
                self.source_query_focus_requested = false;
            }
            source_query.on_hover_text(
                self.locale
                    .text("Press / to focus this filter", "/ 키로 이 필터에 포커스"),
            );
        });
        ui.add_space(8.0);

        egui::Frame::new()
            .fill(self.palette.paper_raised)
            .stroke(Stroke::new(1.0, self.palette.rule))
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.appearance.show_kind {
                        preview_column_label(
                            ui,
                            PREVIEW_KIND_COLUMN_WIDTH,
                            RichText::new(self.locale.text("Kind", "종류"))
                                .strong()
                                .color(self.palette.ink),
                        );
                    }
                    preview_column_label(
                        ui,
                        source_column_width,
                        RichText::new(self.locale.text("Current name", "현재 이름"))
                            .strong()
                            .color(self.palette.ink),
                    );
                    let resize_handle = ui
                        .allocate_response(
                            egui::vec2(PREVIEW_COLUMN_RESIZE_HANDLE_WIDTH, PREVIEW_CELL_HEIGHT),
                            egui::Sense::drag(),
                        )
                        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                    resize_handle.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Other,
                            true,
                            self.locale.text(
                                "Resize current-name and new-name columns",
                                "현재 이름과 바뀔 이름 열 너비 조절",
                            ),
                        )
                    });
                    ui.painter().vline(
                        resize_handle.rect.center().x,
                        resize_handle.rect.y_range(),
                        Stroke::new(1.0, self.palette.rule),
                    );
                    if resize_handle.dragged() {
                        let next_width = (source_column_width + resize_handle.drag_delta().x)
                            .clamp(PREVIEW_SOURCE_COLUMN_WIDTH, maximum_source_width);
                        self.appearance.preview_source_column_width =
                            Some(next_width.round() as u16);
                        ui.ctx().request_repaint();
                    } else if resize_handle.double_clicked() {
                        self.appearance.preview_source_column_width = None;
                    }
                    preview_column_label(
                        ui,
                        proposed_column_width,
                        RichText::new(self.locale.text("New name", "바뀔 이름"))
                            .strong()
                            .color(self.palette.ink),
                    );
                    preview_column_label(
                        ui,
                        PREVIEW_STATUS_COLUMN_WIDTH,
                        RichText::new(self.locale.text("Result", "결과"))
                            .strong()
                            .color(self.palette.ink),
                    );
                    ui.label(
                        RichText::new(if self.appearance.show_diagnostics {
                            self.locale.text("Problems and notes", "문제 및 안내")
                        } else {
                            self.locale.text("Why it cannot apply", "적용할 수 없는 이유")
                        })
                        .strong()
                        .color(self.palette.ink),
                    );
                });
                ui.separator();
                if visible.is_empty() {
                    let no_entries = self
                        .plan
                        .as_ref()
                        .is_none_or(|plan| plan.rows().is_empty());
                    ui.add_space((ui.available_height() * 0.2).clamp(8.0, 72.0));
                    ui.vertical_centered(|ui| {
                        if no_entries {
                            ui.heading(self.locale.text(
                                "Add files or a folder name to start",
                                "이름을 바꿀 파일이나 폴더를 추가하세요",
                            ));
                            ui.label(self.locale.text(
                                "Drag files here or use Add files above. Add folder name includes only the folder itself, not the items inside it.",
                                "파일을 여기에 놓거나 위의 파일 추가를 누르세요. 폴더 이름 추가는 폴더 자체만 포함하며 안의 항목은 추가하지 않습니다.",
                            ));
                        } else {
                            ui.heading(self.locale.text(
                                "No entries match these filters",
                                "조건에 맞는 항목이 없습니다",
                            ));
                            ui.label(self.locale.text(
                                "Change the result, problem, or name filters to show more entries.",
                                "결과, 문제 또는 이름 필터를 바꿔 다른 항목을 표시하세요.",
                            ));
                        }
                    });
                    return;
                }
                let mut requested_override_row = None;
                let mut requested_exclude_row = None;
                let mut requested_rule = None;
                let mut selected_preview_row = self.selected_preview_row;
                let request_preview_focus = self.preview_focus_requested;
                let mut preview_focus_fulfilled = false;
                ScrollArea::vertical()
                    .id_salt("preview.rows")
                    .auto_shrink([false, false])
                    .show_rows(
                        ui,
                        self.appearance.density.preview_row_height(),
                        visible.len(),
                        |ui, row_range| {
                            for visible_row in row_range {
                                let index = visible[visible_row];
                                let (
                                    source_id,
                                    entry_kind,
                                    source,
                                    proposed,
                                    status,
                                    diagnostics,
                                    blocked,
                                    last_changed_rule_id,
                                ) = self.plan.as_ref().map_or_else(
                                    || {
                                        let source = format!("IMG_{index:05}.jpg");
                                        let proposed =
                                            format!("{}{source}", self.synthetic_prefix());
                                        let blocked = Self::row_is_blocked(index);
                                        let status = if blocked {
                                            self.locale.text("Cannot apply", "적용 불가")
                                        } else {
                                            self.locale.text("Will change", "변경 예정")
                                        };
                                        (
                                            None,
                                            "file",
                                            Cow::Owned(source),
                                            Cow::Owned(proposed),
                                            status,
                                            if blocked {
                                                self.locale.text(
                                                    "Example name conflict",
                                                    "예시 이름 충돌",
                                                )
                                            } else {
                                                ""
                                            }
                                            .to_owned(),
                                            blocked,
                                            None,
                                        )
                                    },
                                    |plan| {
                                        let row = &plan.rows()[index];
                                        (
                                            Some(row.source_id()),
                                            row.entry_kind(),
                                            Cow::Borrowed(row.original_name()),
                                            Cow::Borrowed(row.proposed_name()),
                                            if row.status() == "blocked" {
                                                self.locale.text("Cannot apply", "적용 불가")
                                            } else if row.status() == "unchanged" {
                                                self.locale.text("Unchanged", "변경 없음")
                                            } else {
                                                self.locale.text("Will change", "변경 예정")
                                            },
                                            row.diagnostics()
                                                .iter()
                                                .map(|code| diagnostic_label(code, self.locale))
                                                .collect::<Vec<_>>()
                                                .join(", "),
                                            row.status() == "blocked",
                                            row.last_changed_rule_id(),
                                        )
                                    },
                                );
                                let related_rule_position =
                                    last_changed_rule_id.and_then(|rule_id| {
                                        self.rules
                                            .iter()
                                            .position(|rule| rule.rule_id() == rule_id)
                                            .map(|position| (rule_id, position + 1))
                                    });
                                ui.push_id(index, |ui| {
                                    let selected = selected_preview_row == Some(index);
                                    let row_rect = egui::Rect::from_min_size(
                                        ui.cursor().min,
                                        egui::vec2(
                                            ui.available_width(),
                                            self.appearance.density.preview_row_height(),
                                        ),
                                    );
                                    let row_response = ui.interact(
                                        row_rect,
                                        ui.id().with("preview-row"),
                                        egui::Sense::click(),
                                    );
                                    egui::Frame::new()
                                        .fill(if selected {
                                            self.palette.accent_soft
                                        } else {
                                            Color32::TRANSPARENT
                                        })
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                if self.appearance.show_kind {
                                                    preview_column_label(
                                                        ui,
                                                        PREVIEW_KIND_COLUMN_WIDTH,
                                                        match entry_kind {
                                                            "directory" => {
                                                                self.locale.text("Folder", "폴더")
                                                            }
                                                            "symlink" => {
                                                                self.locale.text("Link", "링크")
                                                            }
                                                            _ => self.locale.text("File", "파일"),
                                                        },
                                                    );
                                                }
                                                preview_column_label(
                                                    ui,
                                                    source_column_width,
                                                    source.as_ref(),
                                                );
                                                ui.allocate_space(egui::vec2(
                                                    PREVIEW_COLUMN_RESIZE_HANDLE_WIDTH,
                                                    PREVIEW_CELL_HEIGHT,
                                                ));
                                                preview_column_label(
                                                    ui,
                                                    proposed_column_width,
                                                    proposed.as_ref(),
                                                );
                                                let color = if blocked {
                                                    self.palette.blocked
                                                } else {
                                                    self.palette.accent
                                                };
                                                preview_column_label(
                                                    ui,
                                                    PREVIEW_STATUS_COLUMN_WIDTH,
                                                    RichText::new(status).color(color).strong(),
                                                );
                                                if self.appearance.show_diagnostics || blocked {
                                                    ui.label(&diagnostics);
                                                }
                                                if let Some((rule_id, position)) =
                                                    related_rule_position
                                                    && !diagnostics.is_empty()
                                                    && ui
                                                        .small_button(match self.locale {
                                                            Locale::English => {
                                                                format!("Open rule {position}")
                                                            }
                                                            Locale::Korean => {
                                                                format!("{position}번 규칙 열기")
                                                            }
                                                        })
                                                        .clicked()
                                                {
                                                    requested_rule = Some(rule_id);
                                                }
                                                if let Some(source_id) = source_id
                                                    && ui
                                                        .small_button(
                                                            if self
                                                                .overrides
                                                                .contains_key(&source_id)
                                                            {
                                                                self.locale.text(
                                                                    "Edit assigned name",
                                                                    "직접 지정한 이름 편집",
                                                                )
                                                            } else {
                                                                self.locale.text(
                                                                    "Assign name",
                                                                    "직접 이름 지정",
                                                                )
                                                            },
                                                        )
                                                        .clicked()
                                                {
                                                    requested_override_row = Some(index);
                                                }
                                                if source_id.is_some()
                                                    && ui
                                                        .small_button(self.locale.text(
                                                            semantics::EXCLUDE_SOURCE,
                                                            "계획에서 빼기",
                                                        ))
                                                        .on_hover_text(self.locale.text(
                                                            "Leave this file or folder unchanged",
                                                            "이 파일 또는 폴더의 이름은 바꾸지 않습니다",
                                                        ))
                                                        .clicked()
                                                {
                                                    requested_exclude_row = Some(index);
                                                }
                                            });
                                        });
                                    row_response.widget_info(|| {
                                        let label = match self.locale {
                                            Locale::English => {
                                                format!("{}, preview row", source.as_ref())
                                            }
                                            Locale::Korean => {
                                                format!("{}, 미리보기 행", source.as_ref())
                                            }
                                        };
                                        egui::WidgetInfo::selected(
                                            egui::WidgetType::SelectableLabel,
                                            true,
                                            selected,
                                            label,
                                        )
                                    });
                                    if row_response.clicked() {
                                        selected_preview_row = Some(index);
                                        row_response.request_focus();
                                    }
                                    if request_preview_focus && selected {
                                        row_response.scroll_to_me(Some(Align::Center));
                                        row_response.request_focus();
                                        preview_focus_fulfilled = true;
                                    }
                                    if source_id.is_some()
                                        && (row_response.double_clicked()
                                            || (row_response.clicked() && selected)
                                            || (selected
                                                && row_response.has_focus()
                                                && ui.ctx().input(|input| {
                                                    input.key_pressed(egui::Key::Enter)
                                                })))
                                    {
                                        requested_override_row = Some(index);
                                    }
                                });
                            }
                        },
                    );
                self.selected_preview_row = selected_preview_row;
                if preview_focus_fulfilled {
                    self.preview_focus_requested = false;
                }
                if let Some(row_index) = requested_override_row {
                    self.open_override_for_row(row_index);
                }
                if let Some(row_index) = requested_exclude_row {
                    self.exclude_source_for_row(row_index);
                }
                if let Some(rule_id) = requested_rule {
                    self.focus_rule(rule_id);
                }
            });
    }

    fn show_review_bar(&mut self, ui: &mut egui::Ui) {
        let (total_count, changed_count, blocked_count) = self.plan.as_ref().map_or_else(
            || {
                if self.synthetic_fixture {
                    (
                        SAMPLE_COUNT,
                        SAMPLE_COUNT - SAMPLE_BLOCKED_COUNT,
                        SAMPLE_BLOCKED_COUNT,
                    )
                } else {
                    (0, 0, 0)
                }
            },
            |plan| {
                (
                    plan.rows().len(),
                    plan.changed_count(),
                    plan.blocked_count(),
                )
            },
        );
        let unchanged_count = total_count.saturating_sub(changed_count + blocked_count);
        let can_apply = self.filesystem_authority_enabled()
            && self.plan.as_ref().is_some_and(PlanDto::can_apply)
            && self.journal_root.is_some()
            && self.mutation_task.is_none()
            && self.plan_is_current
            && self.planning_due.is_none()
            && self.planning_task.is_none()
            && self.admission_task.is_none()
            && self.pending_admission_paths.is_empty()
            && self.ledger_ready
            && self.ledger_task.is_none();
        let lock_reason: Cow<'static, str> = if self.synthetic_fixture && self.plan.is_none() {
            Cow::Borrowed(self.locale.text(
                "Sample preview cannot be applied",
                "샘플 미리보기는 적용할 수 없습니다",
            ))
        } else if self.plan.is_none() {
            Cow::Borrowed(self.locale.text(
                "Add files or a folder name to create a rename plan",
                "파일이나 폴더를 추가해 이름 변경 계획을 만드세요",
            ))
        } else if self.admission_task.is_some() || !self.pending_admission_paths.is_empty() {
            Cow::Borrowed(self.locale.text(
                "Wait for selected entries to finish loading",
                "선택한 항목을 모두 불러올 때까지 기다리세요",
            ))
        } else if !self.plan_is_current
            || self.planning_due.is_some()
            || self.planning_task.is_some()
        {
            Cow::Borrowed(self.locale.text(
                "Wait for the latest preview before renaming",
                "최신 미리보기가 준비될 때까지 기다리세요",
            ))
        } else if blocked_count > 0 {
            Cow::Owned(match self.locale {
                Locale::English => format!(
                    "Resolve {blocked_count} entries that cannot be applied before renaming"
                ),
                Locale::Korean => {
                    format!("이름을 바꾸기 전에 적용 불가 항목 {blocked_count}개를 해결하세요")
                }
            })
        } else if self.journal_root.is_none() {
            Cow::Borrowed(self.locale.text(
                "Rename history storage is unavailable, so names cannot be changed",
                "이름 변경 기록을 저장할 수 없어 이름을 바꿀 수 없습니다",
            ))
        } else if !self.ledger_ready || self.ledger_task.is_some() {
            Cow::Borrowed(self.locale.text(
                "Wait for rename history checks to finish",
                "이름 변경 기록 검사가 끝날 때까지 기다리세요",
            ))
        } else if self.mutation_task.is_some() {
            Cow::Borrowed(self.locale.text(
                "Another filesystem operation is running",
                "다른 파일 작업이 실행 중입니다",
            ))
        } else {
            Cow::Borrowed(self.locale.text(
                "The current plan has no changes",
                "현재 계획에 변경 사항이 없습니다",
            ))
        };

        ui.horizontal(|ui| {
            for (filter, count) in [
                (PlanFilter::All, total_count),
                (PlanFilter::Changed, changed_count),
                (PlanFilter::Unchanged, unchanged_count),
                (PlanFilter::Blocked, blocked_count),
            ] {
                let label = format!("{} {count}", filter.label(self.locale));
                if ui
                    .selectable_label(
                        self.filter == filter,
                        RichText::new(label).strong().color(self.palette.ink),
                    )
                    .clicked()
                {
                    self.select_preview_filter(filter);
                }
            }
        });
        ui.horizontal(|ui| {
            let status = if self.status == semantics::NO_SOURCES {
                self.locale
                    .text(semantics::NO_SOURCES, "아직 추가한 항목이 없습니다")
            } else {
                &self.status
            };
            ui.label(RichText::new(status).color(self.palette.ink_soft));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let apply_label = rename_entries_label(changed_count, self.locale);
                let accessible_apply_label = apply_label.clone();
                let apply_text = if self.palette.high_contrast && !can_apply {
                    RichText::new(apply_label).color(self.palette.disabled)
                } else {
                    RichText::new(apply_label)
                };
                let apply = ui.add_enabled(can_apply, egui::Button::new(apply_text));
                apply.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        can_apply,
                        accessible_apply_label.clone(),
                    )
                });
                if apply.clicked()
                    && let Some(plan) = &self.plan
                {
                    self.pending_confirmation = Some(PendingConfirmation::Apply {
                        plan_id: plan.plan_id(),
                        changed_count: plan.changed_count(),
                    });
                }
            });
        });
        if !can_apply {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(lock_reason).color(self.palette.blocked));
            });
        }
    }

    fn inspect_plan(&mut self, json: bool) {
        if self.document_task.is_some() {
            return;
        }
        let Some(plan_id) = self.plan.as_ref().map(PlanDto::plan_id) else {
            return;
        };
        let application = Arc::clone(&self.application);
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = if json {
                application.inspect_plan_json(plan_id)
            } else {
                application.inspect_plan_csv(plan_id)
            }
            .map_err(|error| error.kind());
            let _ = sender.send(DocumentMessage::Inspection { json, result });
        });
        self.document_task = Some(DocumentTask {
            receiver,
            handle: Some(handle),
        });
        self.status = self
            .locale
            .text(
                "Preparing plan inspection",
                "계획 검토 문서를 준비 중입니다",
            )
            .to_owned();
    }

    fn export_plan(&mut self, json: bool) {
        if self.document_task.is_some() || !self.filesystem_authority_enabled() {
            return;
        }
        let Some(plan_id) = self.plan.as_ref().map(PlanDto::plan_id) else {
            return;
        };
        let extension = if json { "json" } else { "csv" };
        let Some(path) = rfd::FileDialog::new()
            .set_title(
                self.locale
                    .text("Export the Renamewright plan", "이름 변경 계획 내보내기"),
            )
            .add_filter(extension.to_uppercase(), &[extension])
            .set_file_name(format!("renamewright-plan.{extension}"))
            .save_file()
        else {
            self.status = self
                .locale
                .text("Plan export cancelled", "계획 내보내기를 취소했습니다")
                .to_owned();
            return;
        };
        let application = Arc::clone(&self.application);
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = if json {
                application.export_plan_json(plan_id, &path)
            } else {
                application.export_plan_csv(plan_id, &path)
            }
            .map_err(|error| error.kind());
            let _ = sender.send(DocumentMessage::Export { extension, result });
        });
        self.document_task = Some(DocumentTask {
            receiver,
            handle: Some(handle),
        });
        self.status = self
            .locale
            .text(
                "Exporting the current plan",
                "현재 계획을 내보내는 중입니다",
            )
            .to_owned();
    }

    fn poll_document(&mut self, context: &egui::Context) {
        let message = self
            .document_task
            .as_ref()
            .map(|task| task.receiver.try_recv());
        match message {
            Some(Ok(message)) => {
                if let Some(task) = self.document_task.take() {
                    task.finish();
                }
                match message {
                    DocumentMessage::Inspection {
                        json,
                        result: Ok(content),
                    } => {
                        self.inspection = Some(InspectionDocument {
                            title: if json {
                                self.locale.text("Plan JSON", "계획 JSON")
                            } else {
                                self.locale.text("Plan CSV", "계획 CSV")
                            },
                            content,
                        });
                    }
                    DocumentMessage::Inspection {
                        result: Err(error), ..
                    }
                    | DocumentMessage::Export {
                        result: Err(error), ..
                    } => self.status = application_error_message(error, self.locale).to_owned(),
                    DocumentMessage::Export {
                        extension,
                        result: Ok(()),
                    } => {
                        self.status = match self.locale {
                            Locale::English => format!("Plan {extension} exported"),
                            Locale::Korean => format!("계획 {extension}을 내보냈습니다"),
                        };
                    }
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                if let Some(task) = self.document_task.take() {
                    task.finish();
                }
                self.status = self
                    .locale
                    .text(
                        "Plan document task ended unexpectedly",
                        "계획 문서 작업이 예기치 않게 종료되었습니다",
                    )
                    .to_owned();
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
        if self.document_task.is_some() {
            context.request_repaint_after(PLANNING_POLL_INTERVAL);
        }
    }

    fn show_transient_windows(&mut self, context: &egui::Context) {
        let mut save_override = false;
        let mut remove_override = false;
        let mut close_override = false;
        if let Some(editor) = self.override_editor.as_mut() {
            egui::Window::new(
                self.locale
                    .text("Assign a name to this entry", "이 항목의 이름 직접 지정"),
            )
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(match self.locale {
                    Locale::English => format!("Current name: {}", editor.original_name),
                    Locale::Korean => format!("현재 이름: {}", editor.original_name),
                });
                let name_label = ui.label(self.locale.text("New name", "직접 지정할 이름"));
                ui.add(egui::TextEdit::singleline(&mut editor.value))
                    .labelled_by(name_label.id);
                ui.horizontal(|ui| {
                    if ui
                        .button(
                            self.locale
                                .text(semantics::SAVE_OVERRIDE, "직접 지정한 이름 저장"),
                        )
                        .clicked()
                    {
                        save_override = true;
                    }
                    if ui
                        .button(
                            self.locale
                                .text("Use rule result", "규칙으로 만든 이름 사용"),
                        )
                        .clicked()
                    {
                        remove_override = true;
                    }
                    if ui
                        .button(self.locale.text(semantics::CANCEL_OVERRIDE, "취소"))
                        .clicked()
                    {
                        close_override = true;
                    }
                });
            });
        }
        if save_override {
            if let Some(editor) = self.override_editor.take() {
                self.overrides.insert(editor.source_id, editor.value);
                self.refresh_plan();
            }
        } else if remove_override {
            if let Some(editor) = self.override_editor.take() {
                self.overrides.remove(&editor.source_id);
                self.refresh_plan();
            }
        } else if close_override {
            self.override_editor = None;
        }

        let mut close_inspection = false;
        if let Some(document) = self.inspection.as_mut() {
            egui::Window::new(document.title)
                .default_width(720.0)
                .default_height(480.0)
                .show(context, |ui| {
                    ScrollArea::both().show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut document.content)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .interactive(false),
                        );
                    });
                    if ui
                        .button(self.locale.text(semantics::CLOSE_INSPECTOR, "검토 창 닫기"))
                        .clicked()
                    {
                        close_inspection = true;
                    }
                });
        }
        if close_inspection {
            self.inspection = None;
        }

        let mut confirm_action = false;
        let mut cancel_confirmation = false;
        if let Some(pending) = &self.pending_confirmation {
            let (title, detail, confirm_label) = match pending {
                PendingConfirmation::Apply {
                    plan_id: _,
                    changed_count,
                } => (
                    self.locale.text("Apply name changes", "이름 변경 적용"),
                    match self.locale {
                        Locale::English => format!(
                            "Use the names shown in the preview to rename {}.",
                            entry_count_label(*changed_count, self.locale)
                        ),
                        Locale::Korean => format!(
                            "미리보기에 표시된 이름으로 {changed_count}개 항목의 이름을 바꿉니다."
                        ),
                    },
                    rename_entries_label(*changed_count, self.locale),
                ),
                PendingConfirmation::Recovery { action, inspection } => {
                    let source_count = self
                        .ledger
                        .iter()
                        .find(|entry| entry.ledger_id() == inspection.ledger_id())
                        .map_or(0, LedgerEntryDto::source_count);
                    let detail = match (action, self.locale) {
                        (RecoveryCommandAction::Resume, Locale::English) => format!(
                            "Continue the interrupted rename for {}.",
                            entry_count_label(source_count, self.locale)
                        ),
                        (RecoveryCommandAction::Resume, Locale::Korean) => {
                            format!("중단된 {source_count}개 항목의 이름 변경을 이어서 진행합니다.")
                        }
                        (RecoveryCommandAction::Rollback, Locale::English) => format!(
                            "Restore the names that {} had before this operation.",
                            entry_count_label(source_count, self.locale)
                        ),
                        (RecoveryCommandAction::Rollback, Locale::Korean) => {
                            format!("{source_count}개 항목을 이 작업 전의 이름으로 되돌립니다.")
                        }
                        (RecoveryCommandAction::Reconcile, Locale::English) => format!(
                            "Check the current locations of {} and refresh the recovery options.",
                            entry_count_label(source_count, self.locale)
                        ),
                        (RecoveryCommandAction::Reconcile, Locale::Korean) => format!(
                            "{source_count}개 항목의 현재 위치를 확인하고 가능한 복구 방법을 갱신합니다."
                        ),
                    };
                    (
                        recovery_action_label(*action, self.locale),
                        detail,
                        recovery_action_label(*action, self.locale).to_owned(),
                    )
                }
                PendingConfirmation::Undo { inspection } => (
                    self.locale.text("Undo name change", "이름 변경 되돌리기"),
                    match self.locale {
                        Locale::English => format!(
                            "Restore the previous names of {}.",
                            entry_count_label(inspection.source_count(), self.locale)
                        ),
                        Locale::Korean => format!(
                            "{}개 항목을 이전 이름으로 되돌립니다.",
                            inspection.source_count()
                        ),
                    },
                    undo_entries_label(inspection.source_count(), self.locale),
                ),
                PendingConfirmation::Cancel => (
                    self.locale.text("Cancel operation", "작업 취소"),
                    self.locale
                        .text("Request cancellation?", "작업 취소를 요청할까요?")
                        .to_owned(),
                    self.locale
                        .text("Request cancellation", "작업 취소 요청")
                        .to_owned(),
                ),
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .show(context, |ui| {
                    ui.label(detail);
                    ui.label(self.locale.text(
                        "Files and folders will be checked again immediately before any names change.",
                        "이름을 바꾸기 직전에 파일과 폴더 상태를 다시 확인합니다.",
                    ));
                    ui.horizontal(|ui| {
                        if ui.button(self.locale.text("Cancel", "취소")).clicked() {
                            cancel_confirmation = true;
                        }
                        if ui.button(confirm_label).clicked() {
                            confirm_action = true;
                        }
                    });
                });
        }
        if confirm_action {
            if let Some(pending) = self.pending_confirmation.take() {
                self.start_confirmed_mutation(pending);
            }
        } else if cancel_confirmation {
            self.pending_confirmation = None;
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let entered_non_ascii_text = ui.ctx().input(|input| {
            input.events.iter().any(|event| match event {
                egui::Event::Text(text) | egui::Event::Paste(text) => !text.is_ascii(),
                egui::Event::Ime(_) => true,
                _ => false,
            })
        });
        if entered_non_ascii_text {
            self.ensure_korean_font(ui.ctx());
        }
        self.poll_korean_font(ui.ctx());
        self.apply_appearance(ui.ctx());
        self.poll_admission(ui.ctx());
        if self.korean_font_state == KoreanFontState::Idle
            && (self.rules.iter().any(rule_has_non_ascii_text)
                || self.overrides.values().any(|value| !value.is_ascii())
                || !self.source_query.is_ascii()
                || !self.preset_name.is_ascii()
                || self.presets.presets().iter().any(|preset| {
                    !preset.name().is_ascii() || preset.rules().iter().any(rule_has_non_ascii_text)
                })
                || self.plan.as_ref().is_some_and(|plan| {
                    plan.rows().iter().any(|row| {
                        !row.original_name().is_ascii() || !row.proposed_name().is_ascii()
                    })
                }))
        {
            self.ensure_korean_font(ui.ctx());
        }
        self.poll_planning(ui.ctx());
        self.poll_ledger(ui.ctx());
        self.poll_document(ui.ctx());
        self.poll_mutation(ui.ctx());
        let add_folder_shortcut = egui::KeyboardShortcut::new(
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            egui::Key::O,
        );
        let add_files_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::O);
        let move_rule_before =
            egui::KeyboardShortcut::new(egui::Modifiers::ALT, egui::Key::ArrowLeft);
        let move_rule_after =
            egui::KeyboardShortcut::new(egui::Modifiers::ALT, egui::Key::ArrowRight);
        let filesystem_authority = self.filesystem_authority_enabled();
        if filesystem_authority
            && ui
                .ctx()
                .input_mut(|input| input.consume_shortcut(&add_folder_shortcut))
        {
            self.choose_folder_entry(ui.ctx());
        } else if filesystem_authority
            && ui
                .ctx()
                .input_mut(|input| input.consume_shortcut(&add_files_shortcut))
        {
            self.choose_files(ui.ctx());
        }
        if ui
            .ctx()
            .input_mut(|input| input.consume_shortcut(&move_rule_before))
        {
            self.move_selected_rule(-1);
        } else if ui
            .ctx()
            .input_mut(|input| input.consume_shortcut(&move_rule_after))
        {
            self.move_selected_rule(1);
        }
        if !ui.ctx().egui_wants_keyboard_input()
            && ui
                .ctx()
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Slash))
        {
            self.source_query_focus_requested = true;
        }
        let hovered_source_count = if filesystem_authority {
            ui.ctx().input(|input| input.raw.hovered_files.len())
        } else {
            0
        };
        let dropped_paths = if filesystem_authority {
            ui.ctx().input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .collect::<Vec<_>>()
            })
        } else {
            Vec::new()
        };
        if !dropped_paths.is_empty() {
            self.start_source_admission(dropped_paths, ui.ctx());
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
                            RichText::new(self.locale.text(
                                semantics::HIGH_CONTRAST_ACTIVE,
                                "Windows 고대비 색상 사용 중",
                            ))
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

        let appearance_replaces_workbench =
            self.appearance_advanced_open && ui.available_width() < 1_040.0;

        if !appearance_replaces_workbench {
            egui::Panel::top("rule-command-bar")
                .resizable(true)
                .default_size(150.0)
                .min_size(104.0)
                .max_size(300.0)
                .frame(
                    egui::Frame::new()
                        .fill(self.palette.paper_soft)
                        .stroke(Stroke::new(1.0, self.palette.rule))
                        .inner_margin(10.0),
                )
                .show(ui, |ui| self.show_rule_command_bar(ui));
        }

        if self.ledger_open {
            egui::Panel::right("ledger")
                .resizable(true)
                .default_size(300.0)
                .min_size(240.0)
                .frame(
                    egui::Frame::new()
                        .fill(self.palette.paper_soft)
                        .inner_margin(12.0),
                )
                .show(ui, |ui| self.show_ledger(ui));
        }

        if self.appearance_advanced_open && !appearance_replaces_workbench {
            egui::Panel::right("advanced-appearance")
                .resizable(true)
                .default_size(320.0)
                .min_size(280.0)
                .max_size(420.0)
                .frame(
                    egui::Frame::new()
                        .fill(self.palette.paper_soft)
                        .stroke(Stroke::new(1.0, self.palette.rule))
                        .inner_margin(12.0),
                )
                .show(ui, |ui| {
                    self.show_advanced_appearance(ui, "advanced-appearance-wide-options");
                });
        }

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
            .show(ui, |ui| {
                if appearance_replaces_workbench {
                    self.show_advanced_appearance(ui, "advanced-appearance-compact-options");
                } else {
                    self.show_preview(ui);
                }
            });

        if hovered_source_count > 0 {
            self.show_source_drop_overlay(ui.ctx(), hovered_source_count);
        }

        self.show_transient_windows(ui.ctx());
    }
}

impl eframe::App for RenamewrightApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        #[cfg(feature = "automation")]
        if self.automation_mode {
            return;
        }
        eframe::set_value(storage, APPEARANCE_STORAGE_KEY, &self.appearance);
    }
}

impl Drop for RenamewrightApp {
    fn drop(&mut self) {
        if self.mutation_task.is_some() {
            let _ = self.application.request_confirmed_cancellation(|| true);
        }
        if let Some(task) = self.mutation_task.take() {
            task.finish();
        }
        if let Some(task) = self.planning_task.take() {
            task.finish();
        }
        let _ = self.admission_task.take();
        if let Some(task) = self.ledger_task.take() {
            task.finish();
        }
        if let Some(task) = self.document_task.take() {
            task.finish();
        }
        let _ = self.korean_font_task.take();
    }
}

pub fn install_theme(ctx: &egui::Context, palette: NativePalette) {
    install_theme_with_density(ctx, palette, InterfaceDensity::Standard);
}

fn install_theme_with_density(
    ctx: &egui::Context,
    palette: NativePalette,
    density: InterfaceDensity,
) {
    let theme = palette.theme();
    ctx.set_theme(theme);
    let mut style = theme.default_style();
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
    style.visuals.selection.stroke = Stroke::new(
        1.0,
        if palette.high_contrast {
            palette.accent_text
        } else {
            palette.accent
        },
    );
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
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(
        2.0,
        if palette.high_contrast {
            palette.accent_text
        } else {
            palette.accent
        },
    );
    style.visuals.widgets.hovered.fg_stroke.color = if palette.high_contrast {
        palette.accent_text
    } else {
        palette.ink
    };
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
    match density {
        InterfaceDensity::Standard => {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
            style.spacing.interact_size.y = 30.0;
            style.spacing.indent = 18.0;
        }
        InterfaceDensity::Compact => {
            style.spacing.item_spacing = egui::vec2(6.0, 4.0);
            style.spacing.button_padding = egui::vec2(6.0, 3.0);
            style.spacing.interact_size.y = 26.0;
            style.spacing.indent = 16.0;
        }
    }
    ctx.set_style_of(theme, style);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::fs;
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use eframe::egui;
    use egui_kittest::Harness;
    use kittest::{NodeT as _, Queryable as _};

    #[cfg(feature = "automation")]
    use super::automation::{AutomationProfile, AutomationRoot, AutomationRootErrorKind};
    use super::{
        AccentChoice, AppearanceTheme, InterfaceDensity, KoreanFontState, LedgerMessage,
        LedgerTask, Locale, MutationTask, NativePalette, PLANNING_DEBOUNCE,
        PREVIEW_PROPOSED_COLUMN_WIDTH, PREVIEW_SOURCE_COLUMN_WIDTH, PendingConfirmation, PlanDto,
        PlanFilter, RenamewrightApp, RuleKind, RuleRequestDto, adjacent_selection_index,
        apply_error_message, install_theme, install_theme_with_density, ledger_status_label,
        planning_error_message, preset_error_message, preview_column_label, recovery_error_message,
        semantics, undo_error_message,
    };

    #[derive(Default)]
    struct MemoryStorage {
        values: BTreeMap<String, String>,
    }

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.values.insert(key.to_owned(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.values.remove(key);
        }

        fn flush(&mut self) {}
    }

    #[test]
    fn embedded_base_font_covers_production_ui_glyphs() -> Result<(), Box<dyn Error>> {
        use skrifa::MetadataProvider as _;

        let definitions = super::base_font_definitions();
        assert!(definitions.font_data.contains_key(super::BASE_FONT_NAME));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let Some(family_fonts) = definitions.families.get(&family) else {
                return Err("the base font family was not installed".into());
            };
            assert_eq!(family_fonts, &[super::BASE_FONT_NAME]);
        }
        let font = skrifa::FontRef::new(epaint_default_fonts::UBUNTU_LIGHT)?;
        let character_map = font.charmap();
        let missing_glyphs: Vec<_> = "Renamewright 0123456789 · × “text” … <- -> \
            Korean (loading) Korean (font unavailable)"
            .chars()
            .filter(|character| character_map.map(*character).is_none())
            .collect();

        assert!(
            missing_glyphs.is_empty(),
            "the compact embedded font is missing production UI glyphs: {missing_glyphs:?}"
        );
        Ok(())
    }

    fn settle_ledger(app: &mut RenamewrightApp) {
        let context = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.ledger_task.is_some() {
            app.poll_ledger(&context);
            assert!(Instant::now() < deadline, "ledger task did not settle");
            thread::yield_now();
        }
    }

    fn settle_document(app: &mut RenamewrightApp) {
        let context = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.document_task.is_some() {
            app.poll_document(&context);
            assert!(Instant::now() < deadline, "document task did not settle");
            thread::yield_now();
        }
    }

    fn settle_admission(app: &mut RenamewrightApp) {
        let context = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.admission_task.is_some() {
            app.poll_admission(&context);
            assert!(Instant::now() < deadline, "admission task did not settle");
            thread::yield_now();
        }
    }

    fn ascii_synthetic_app() -> RenamewrightApp {
        let mut app = RenamewrightApp::new(false);
        app.set_prefix("sample_");
        app
    }

    #[test]
    fn source_admission_runs_off_frame_and_queues_a_second_selection() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        fs::write(&first, b"first")?;
        fs::write(&second, b"second")?;
        let context = egui::Context::default();
        let mut app = RenamewrightApp::new(false);

        app.start_source_admission(vec![first], &context);
        assert!(app.admission_task.is_some());
        assert!(!app.plan_is_current);
        app.start_source_admission(vec![second], &context);
        assert_eq!(app.pending_admission_paths.len(), 1);
        let mut output = context.run_ui(egui::RawInput::default(), |ui| app.show(ui));
        output.textures_delta.clear();
        settle_admission(&mut app);

        assert!(app.plan_is_current);
        assert_eq!(app.plan.as_ref().map(|plan| plan.rows().len()), Some(2));
        assert!(app.pending_admission_paths.is_empty());
        Ok(())
    }

    fn relative_luminance(color: egui::Color32) -> f32 {
        fn channel(value: u8) -> f32 {
            let value = f32::from(value) / 255.0;
            if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }

        0.212_6 * channel(color.r()) + 0.715_2 * channel(color.g()) + 0.072_2 * channel(color.b())
    }

    fn contrast_ratio(left: egui::Color32, right: egui::Color32) -> f32 {
        let left = relative_luminance(left);
        let right = relative_luminance(right);
        (left.max(right) + 0.05) / (left.min(right) + 0.05)
    }

    #[test]
    fn confirmed_mutation_cannot_replace_the_tracked_task() -> Result<(), Box<dyn Error>> {
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        let (message_sender, receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = release_receiver.recv();
            drop(message_sender);
        });
        let original_thread = handle.thread().id();
        app.mutation_task = Some(MutationTask {
            receiver,
            handle: Some(handle),
        });

        app.start_confirmed_mutation(PendingConfirmation::Apply {
            plan_id: 1,
            changed_count: 1,
        });

        let tracked_thread = app
            .mutation_task
            .as_ref()
            .and_then(|task| task.handle.as_ref())
            .map(|handle| handle.thread().id());
        assert_eq!(tracked_thread, Some(original_thread));
        assert_eq!(app.status, "Another operation is already running");

        release_sender.send(())?;
        if let Some(task) = app.mutation_task.take() {
            task.finish();
        }
        Ok(())
    }

    #[test]
    fn accesskit_exposes_primary_workbench_controls() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), ascii_synthetic_app());

        let add_files = harness.get_by_label(semantics::ADD_FILES);
        let add_folder = harness.get_by_label(semantics::ADD_FOLDER);
        let replace = harness.get_by_label(RuleKind::LiteralReplace.label(Locale::English));
        let prefix = harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, semantics::PREFIX_LABEL);
        let source_query = harness.get_by_role_and_label(
            egui::accesskit::Role::TextInput,
            semantics::SOURCE_QUERY_LABEL,
        );
        let apply = harness.get_by_label("Rename 9990 entries");
        let move_up = harness.get_by_label(semantics::MOVE_RULE_UP);
        let drag_rule = harness.get_by_label("Drag rule 1");
        let remove_rule = harness.get_by_label(semantics::REMOVE_RULE);
        assert_eq!(
            add_files.accesskit_node().role(),
            egui::accesskit::Role::Button
        );
        assert_eq!(
            add_folder.accesskit_node().role(),
            egui::accesskit::Role::Button
        );
        assert!(!add_folder.accesskit_node().is_disabled());
        assert_eq!(
            prefix.accesskit_node().role(),
            egui::accesskit::Role::TextInput
        );
        assert_eq!(
            replace.accesskit_node().role(),
            egui::accesskit::Role::Button
        );
        assert_eq!(
            source_query.accesskit_node().role(),
            egui::accesskit::Role::TextInput
        );
        assert_eq!(apply.accesskit_node().role(), egui::accesskit::Role::Button);
        assert!(apply.accesskit_node().is_disabled());
        assert_eq!(
            harness
                .get_by_label(semantics::HISTORY)
                .accesskit_node()
                .role(),
            egui::accesskit::Role::Button
        );
        assert_eq!(
            move_up.accesskit_node().role(),
            egui::accesskit::Role::Button
        );
        assert!(move_up.accesskit_node().is_disabled());
        assert_eq!(
            drag_rule.accesskit_node().role(),
            egui::accesskit::Role::Unknown
        );
        assert_eq!(
            remove_rule.accesskit_node().role(),
            egui::accesskit::Role::Button
        );
        assert!(harness.query_by_label(semantics::PRESET_NAME).is_none());
        assert!(harness.query_by_label(semantics::REFRESH_LEDGER).is_none());
        harness.get_by_label(semantics::TOOLS).click();
        harness.run_ok();
        harness.get_by_label(semantics::PRESET_NAME);
        harness.get_by_label(semantics::HISTORY).click();
        harness.run_ok();
        harness.get_by_label(semantics::REFRESH_LEDGER);
    }

    #[test]
    fn rule_reorder_uses_stable_ids_and_preserves_the_selected_rule() {
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.rules = vec![
            RuleKind::Prefix.create(11),
            RuleKind::Sequence.create(22),
            RuleKind::Case.create(33),
        ];
        app.selected_rule = 1;

        assert!(app.move_rule_to_insertion(11, 3));
        assert_eq!(
            app.rules
                .iter()
                .map(RuleRequestDto::rule_id)
                .collect::<Vec<_>>(),
            vec![22, 33, 11]
        );
        assert_eq!(app.selected_rule, 0);
        assert!(!app.move_rule_to_insertion(33, 1));
        assert!(!app.move_rule_to_insertion(404, 0));
    }

    #[test]
    fn rule_drag_handle_reorders_when_dropped_on_a_visible_rule_card() -> Result<(), Box<dyn Error>>
    {
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.rules = vec![
            RuleKind::Prefix.create(11),
            RuleKind::Sequence.create(22),
            RuleKind::Case.create(33),
        ];
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        let drag_position = harness.get_by_label("Drag rule 1").rect().center();
        harness.drag_at(drag_position);
        harness.run_ok();
        let target_remove = harness
            .get_all_by_label(semantics::REMOVE_RULE)
            .next_back()
            .ok_or("the final visible rule card lost its remove control")?;
        let drop_position = target_remove.rect().center();
        harness.hover_at(drop_position);
        harness.run_ok();
        harness.drop_at(drop_position);
        harness.run_ok();

        assert_eq!(
            harness
                .state()
                .rules
                .iter()
                .map(RuleRequestDto::rule_id)
                .collect::<Vec<_>>(),
            vec![22, 33, 11]
        );
        Ok(())
    }

    #[test]
    fn source_drag_hover_announces_the_drop_action_and_count() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                RenamewrightApp::new_product(NativePalette::default(), None),
            );
        harness
            .input_mut()
            .hovered_files
            .extend([egui::HoveredFile::default(), egui::HoveredFile::default()]);
        harness.run_steps(1);

        harness.get_by_label("Add files or folder names");
        harness.get_by_label("Release to add 2 entries");
    }

    #[test]
    fn selected_preview_row_opens_its_override_with_enter() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.set_prefix("final-");
        app.admit_sources_immediately(vec![source]);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness.get_by_label("report.txt, preview row").click();
        harness.run_ok();
        assert_eq!(harness.state().selected_preview_row, Some(0));
        harness.key_press(egui::Key::Enter);
        harness.run_ok();

        harness.get_by_label("Assign a name to this entry");
        assert_eq!(
            harness
                .state()
                .override_editor
                .as_ref()
                .map(|editor| editor.original_name.as_str()),
            Some("report.txt")
        );
        Ok(())
    }

    #[test]
    fn preview_row_double_click_opens_its_override() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.set_prefix("final-");
        app.admit_sources_immediately(vec![source]);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        let row = harness.get_by_label("report.txt, preview row");
        row.click();
        row.click();
        harness.run_ok();

        assert!(harness.state().override_editor.is_some());
        Ok(())
    }

    #[test]
    fn review_count_chip_filters_and_focuses_the_first_matching_row() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), ascii_synthetic_app());

        harness.get_by_label("Cannot apply 10").click();
        harness.run_ok();

        assert_eq!(harness.state().filter, PlanFilter::Blocked);
        assert_eq!(harness.state().selected_preview_row, Some(997));
        harness.get_by_label("10 shown");
        harness.get_by_label("IMG_00997.jpg, preview row");
    }

    #[test]
    fn blocked_diagnostic_navigates_to_the_last_changed_rule() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.rules = vec![RuleRequestDto::LiteralReplace {
            rule_id: 7,
            enabled: true,
            search: "report".to_owned(),
            replacement: "CON".to_owned(),
        }];
        app.next_rule_id = 8;
        app.admit_sources_immediately(vec![source]);
        assert_eq!(
            app.plan
                .as_ref()
                .and_then(|plan| plan.rows()[0].last_changed_rule_id()),
            Some(7)
        );
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness.get_by_label("Open rule 1").click();
        harness.run_ok();

        assert!(harness.state().rule_editor_open);
        assert_eq!(harness.state().selected_rule, 0);
        Ok(())
    }

    #[test]
    fn preview_column_divider_resizes_and_resets_the_view_only() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), ascii_synthetic_app());
        let divider = harness
            .get_by_label("Resize current-name and new-name columns")
            .rect()
            .center();

        harness.drag_at(divider);
        harness.run_ok();
        harness.hover_at(divider + egui::vec2(64.0, 0.0));
        harness.run_ok();
        harness.drop_at(divider + egui::vec2(64.0, 0.0));
        harness.run_ok();

        assert!(
            harness
                .state()
                .appearance
                .preview_source_column_width
                .is_some()
        );
        harness.get_by_label("Reset column widths").click();
        harness.run_ok();
        assert_eq!(harness.state().appearance.preview_source_column_width, None);
    }

    #[test]
    fn keyboard_shortcuts_reorder_rules_and_focus_preview_search() {
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.rules = vec![
            RuleKind::Prefix.create(11),
            RuleKind::Sequence.create(22),
            RuleKind::Case.create(33),
        ];
        app.selected_rule = 1;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
        harness.run_ok();
        assert_eq!(
            harness
                .state()
                .rules
                .iter()
                .map(RuleRequestDto::rule_id)
                .collect::<Vec<_>>(),
            vec![22, 11, 33]
        );
        harness.key_press(egui::Key::Slash);
        harness.run_ok();
        assert!(
            harness
                .get_by_role_and_label(
                    egui::accesskit::Role::TextInput,
                    semantics::SOURCE_QUERY_LABEL,
                )
                .accesskit_node()
                .is_focused()
        );
    }

    #[test]
    fn removing_a_source_from_the_plan_keeps_the_filesystem_entry() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.set_prefix("final-");
        app.admit_sources_immediately(vec![source.clone()]);
        app.pending_confirmation = Some(PendingConfirmation::Apply {
            plan_id: app.plan.as_ref().map_or(0, PlanDto::plan_id),
            changed_count: 1,
        });
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness.get_by_label(semantics::EXCLUDE_SOURCE).click();
        harness.run_ok();

        assert!(source.is_file());
        assert_eq!(
            harness.state().plan.as_ref().map(|plan| plan.rows().len()),
            Some(0)
        );
        assert!(harness.state().pending_confirmation.is_none());
        assert!(
            harness
                .state()
                .status
                .starts_with("Removed 1 entry from the plan")
        );
        Ok(())
    }

    #[test]
    fn ledger_keyboard_selection_is_bounded_and_starts_at_the_first_entry() {
        assert_eq!(adjacent_selection_index(3, None, 1), Some(0));
        assert_eq!(adjacent_selection_index(3, Some(1), -1), Some(0));
        assert_eq!(adjacent_selection_index(3, Some(1), 1), Some(2));
        assert_eq!(adjacent_selection_index(3, Some(2), 1), Some(2));
        assert_eq!(adjacent_selection_index(0, None, 1), None);
    }

    #[test]
    fn appearance_keeps_theme_choices_disclosed_until_requested() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                RenamewrightApp::new_product(NativePalette::default(), None),
            );

        assert!(harness.query_by_label(semantics::THEME_SYSTEM).is_none());
        harness.get_by_label(semantics::APPEARANCE).click();
        harness.run_ok();
        harness.get_by_label(semantics::THEME_SYSTEM);
        harness.get_by_label(semantics::THEME_LIGHT);
        harness.get_by_label(semantics::THEME_DARK).click();
        harness.run_ok();

        assert_eq!(harness.state().appearance.theme, AppearanceTheme::Dark);
        assert_eq!(harness.state().palette.theme(), egui::Theme::Dark);
        assert!(harness.state().plan.is_none());
    }

    #[test]
    fn appearance_theme_persists_without_storing_a_plan() {
        let mut storage = MemoryStorage::default();
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.appearance.theme = AppearanceTheme::Dark;
        app.appearance.accent = AccentChoice::Teal;
        app.appearance.density = InterfaceDensity::Compact;
        app.appearance.show_kind = false;
        app.appearance.show_diagnostics = false;
        app.appearance.preview_source_column_width = Some(240);
        eframe::App::save(&mut app, &mut storage);

        let restored = RenamewrightApp::new_product_with_persistence(
            NativePalette::default(),
            None,
            None,
            Some(&storage),
        );
        assert_eq!(restored.appearance.theme, AppearanceTheme::Dark);
        assert_eq!(restored.appearance.accent, AccentChoice::Teal);
        assert_eq!(restored.appearance.density, InterfaceDensity::Compact);
        assert!(!restored.appearance.show_kind);
        assert!(!restored.appearance.show_diagnostics);
        assert_eq!(restored.appearance.preview_source_column_width, Some(240));
        assert!(restored.plan.is_none());
        assert!(restored.pending_confirmation.is_none());
    }

    #[test]
    fn advanced_appearance_stays_hidden_behind_the_simple_theme_menu() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                RenamewrightApp::new_product(NativePalette::default(), None),
            );

        assert!(
            harness
                .query_by_label(semantics::ADVANCED_APPEARANCE)
                .is_none()
        );
        harness.get_by_label(semantics::APPEARANCE).click();
        harness.run_ok();
        harness.get_by_label(semantics::ADVANCED_APPEARANCE).click();
        harness.run_ok();

        harness.get_by_label(semantics::CLOSE_APPEARANCE);
        harness.get_by_label(semantics::ACCENT_COLOR);
        harness.get_by_label(semantics::DENSITY);
        harness.get_by_label(semantics::SHOW_KIND);
        harness.get_by_label(semantics::SHOW_DIAGNOSTICS);
        assert!(harness.state().appearance_advanced_open);
    }

    #[test]
    fn advanced_appearance_changes_view_without_rebuilding_the_plan() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.set_prefix("final-");
        app.admit_sources_immediately(vec![source]);
        let plan_id = app.plan.as_ref().map(PlanDto::plan_id);
        let changed_count = app.plan.as_ref().map(PlanDto::changed_count);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness.get_by_label(semantics::APPEARANCE).click();
        harness.run_ok();
        harness.get_by_label(semantics::ADVANCED_APPEARANCE).click();
        harness.run_ok();
        harness.get_by_label("Teal").click();
        harness.run_ok();
        harness.get_by_label(semantics::DENSITY_COMPACT).click();
        harness.run_ok();
        harness.get_by_label(semantics::SHOW_KIND).click();
        harness.run_ok();
        harness.get_by_label(semantics::SHOW_DIAGNOSTICS).click();
        harness.run_ok();

        assert_eq!(harness.state().appearance.accent, AccentChoice::Teal);
        assert_eq!(
            harness.state().appearance.density,
            InterfaceDensity::Compact
        );
        assert!(!harness.state().appearance.show_kind);
        assert!(!harness.state().appearance.show_diagnostics);
        assert_eq!(harness.state().plan.as_ref().map(PlanDto::plan_id), plan_id);
        assert_eq!(
            harness.state().plan.as_ref().map(PlanDto::changed_count),
            changed_count
        );
        assert!(harness.query_by_label("Kind").is_none());
        Ok(())
    }

    #[test]
    fn hidden_diagnostic_details_keep_blocker_reasons_visible() {
        let mut app = ascii_synthetic_app();
        app.appearance.show_diagnostics = false;
        app.filter = PlanFilter::Blocked;
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness.get_by_label("Why it cannot apply");
        assert!(harness.query_all_by_label("Example name conflict").count() > 0);
        harness.get_by_label("Sample preview cannot be applied");
    }

    #[test]
    fn compact_window_replaces_preview_with_a_closeable_appearance_panel() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(820.0, 560.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                RenamewrightApp::new_product(NativePalette::default(), None),
            );

        harness.get_by_label(semantics::APPEARANCE).click();
        harness.run_ok();
        harness.get_by_label(semantics::ADVANCED_APPEARANCE).click();
        harness.run_ok();
        assert!(harness.query_by_label(semantics::RULES_HEADING).is_none());
        harness.get_by_label(semantics::CLOSE_APPEARANCE).click();
        harness.run_ok();

        harness.get_by_label(semantics::RULES_HEADING);
        harness.get_by_label(semantics::PREVIEW_HEADING);
        harness.get_by_label("Add files or a folder name to create a rename plan");
        assert!(!harness.state().appearance_advanced_open);

        harness.get_by_label(semantics::APPEARANCE).click();
        harness.run_ok();
        harness.get_by_label(semantics::ADVANCED_APPEARANCE).click();
        harness.run_ok();
        harness
            .get_by_label(semantics::RESET_APPEARANCE)
            .scroll_to_me();
        harness.run_ok();
        harness.get_by_label(semantics::RESET_APPEARANCE);
        harness.get_by_label("Add files or a folder name to create a rename plan");
        assert!(harness.state().appearance_advanced_open);
    }

    #[test]
    fn compact_source_bar_prioritizes_actions_over_supporting_copy() {
        let harness = Harness::builder()
            .with_size(egui::vec2(820.0, 560.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                RenamewrightApp::new_product(NativePalette::default(), None),
            );

        harness.get_by_label(semantics::PRODUCT_NAME);
        harness.get_by_label(semantics::ADD_FILES);
        harness.get_by_label(semantics::ADD_FOLDER);
        harness.get_by_label(semantics::APPEARANCE);
        assert!(harness.query_by_label(semantics::TAGLINE).is_none());
        assert!(harness.query_by_label("0 entries").is_none());
    }

    #[test]
    fn high_contrast_disables_theme_choices_and_explains_the_override() {
        let palette = NativePalette::high_contrast(
            [0, 0, 0],
            [255, 255, 255],
            [255, 255, 0],
            [0, 0, 0],
            [0, 255, 0],
        );
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                RenamewrightApp::new_product(palette, None),
            );

        harness.get_by_label(semantics::APPEARANCE).click();
        harness.run_ok();
        assert!(
            harness
                .get_by_label(semantics::THEME_LIGHT)
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label(semantics::HIGH_CONTRAST_OVERRIDES_APPEARANCE);
        assert_eq!(harness.state().palette, palette);
    }

    #[test]
    fn blocked_filter_updates_the_accessible_result_count() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), ascii_synthetic_app());

        assert_eq!(
            harness.state().korean_font_state,
            KoreanFontState::Idle,
            "this filter test must not race an operating-system font layout change"
        );

        harness.get_by_label(semantics::FILTER_BLOCKED).click();
        harness.run_ok();

        assert_eq!(harness.state().filter, PlanFilter::Blocked);
        harness.get_by_label("10 shown");
    }

    #[test]
    fn unchanged_preview_filters_reuse_the_cached_index_projection() {
        let mut app = RenamewrightApp::new(false);

        let first = app.visible_indices();
        let second = app.visible_indices();
        assert!(Arc::ptr_eq(&first, &second));

        app.source_query = "9999".to_owned();
        let filtered = app.visible_indices();
        assert!(!Arc::ptr_eq(&second, &filtered));
        assert_eq!(&*filtered, &[9_999]);
    }

    #[test]
    fn pending_preview_refresh_keeps_apply_visibly_locked() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product_with_data(
            NativePalette::default(),
            Some(directory.path().join("presets.json")),
            Some(directory.path().join("journals")),
        );
        settle_ledger(&mut app);
        app.set_prefix("final-");
        app.admit_sources_immediately(vec![source]);
        assert!(app.plan.as_ref().is_some_and(PlanDto::can_apply));

        app.set_prefix("reviewed-");
        app.schedule_plan_refresh(&egui::Context::default());
        app.planning_due = Some(Instant::now() + Duration::from_secs(5));
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        assert!(
            harness
                .get_by_label("Rename 1 entry")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("Wait for the latest preview before renaming");
        Ok(())
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
                RenamewrightApp::new_with_palette(false, palette),
            );

        assert!(palette.is_high_contrast());
        harness.get_by_label(semantics::HIGH_CONTRAST_ACTIVE);
        let apply = harness.get_by_label("Rename 9990 entries");
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

    #[test]
    fn every_advanced_accent_keeps_text_and_fill_contrast() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            for accent in AccentChoice::ALL {
                let palette = NativePalette::for_theme(theme, accent);
                assert!(
                    contrast_ratio(palette.accent, palette.paper) >= 4.5,
                    "{theme:?} {accent:?} foreground contrast was too low"
                );
                assert!(
                    contrast_ratio(palette.accent_text, palette.accent_fill) >= 4.5,
                    "{theme:?} {accent:?} fill contrast was too low"
                );
            }
        }
    }

    #[test]
    fn density_presets_apply_distinct_bounded_native_spacing() {
        let context = egui::Context::default();
        let palette = NativePalette::for_theme(egui::Theme::Light, AccentChoice::Cobalt);
        install_theme_with_density(&context, palette, InterfaceDensity::Standard);
        let standard = context.style_of(egui::Theme::Light);
        assert_eq!(standard.spacing.interact_size.y, 30.0);
        assert_eq!(standard.spacing.item_spacing, egui::vec2(8.0, 6.0));

        install_theme_with_density(&context, palette, InterfaceDensity::Compact);
        let compact = context.style_of(egui::Theme::Light);
        assert_eq!(compact.spacing.interact_size.y, 26.0);
        assert_eq!(compact.spacing.item_spacing, egui::vec2(6.0, 4.0));
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_build_has_a_visible_mode_banner() {
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), RenamewrightApp::new(true));

        harness.get_by_label(semantics::AUTOMATION_BANNER);
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_workbench_with_empty_profile_matches_the_product_empty_state()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = AutomationRoot::open(directory.path())?;
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                RenamewrightApp::new_automated(
                    NativePalette::default(),
                    root,
                    AutomationProfile::Empty,
                ),
            );

        harness.get_by_label(semantics::AUTOMATION_BANNER);
        harness.get_by_label("0 shown");
        harness.get_by_label("Add files or a folder name to create a rename plan");
        harness.get_by_label("Add files or a folder name to start");
        assert!(harness.query_by_label(semantics::PREFIX_LABEL).is_none());
        assert!(harness.query_by_label("IMG_00000.jpg").is_none());
        Ok(())
    }

    #[test]
    fn ten_thousand_entry_preview_keeps_the_accessibility_tree_bounded() {
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), ascii_synthetic_app());

        assert!(harness.query_by_label("IMG_00000.jpg").is_some());
        assert!(harness.query_by_label("IMG_09999.jpg").is_none());
        assert!(harness.query_all_by(|_| true).count() < 500);
    }

    #[test]
    fn production_workbench_starts_empty_without_synthetic_sources() {
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                RenamewrightApp::new_product(NativePalette::default(), None),
            );

        harness.get_by_label(semantics::NO_SOURCES);
        harness.get_by_label("0 shown");
        harness.get_by_label("Add files or a folder name to create a rename plan");
        harness.get_by_label("Add files or a folder name to start");
        harness.get_by_role_and_label(
            egui::accesskit::Role::Button,
            RuleKind::LiteralReplace.label(Locale::English),
        );
        assert!(harness.query_by_label(semantics::PREFIX_LABEL).is_none());
        assert!(harness.query_by_label("IMG_00000.jpg").is_none());
    }

    #[test]
    fn minimum_window_keeps_rules_preview_and_apply_state_reachable() {
        let harness = Harness::builder()
            .with_size(egui::vec2(820.0, 560.0))
            .build_ui_state(|ui, app| app.show(ui), ascii_synthetic_app());

        harness.get_by_label(semantics::RULES_HEADING);
        harness.get_by_role_and_label(
            egui::accesskit::Role::Button,
            RuleKind::LiteralReplace.label(Locale::English),
        );
        harness.get_by_label(semantics::PREVIEW_HEADING);
        harness.get_by_label("Sample preview cannot be applied");
        harness.get_by_label("Rename 9990 entries");
    }

    #[test]
    fn product_workbench_initializes_its_journal_root() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let journal_root = directory.path().join("journals");
        let mut app = RenamewrightApp::new_product_with_data(
            NativePalette::default(),
            Some(directory.path().join("presets.json")),
            Some(journal_root.clone()),
        );
        assert!(app.ledger_task.is_some());
        settle_ledger(&mut app);

        assert!(journal_root.is_dir());
        assert_eq!(app.journal_root.as_deref(), Some(journal_root.as_path()));
        assert!(app.ledger.is_empty());
        Ok(())
    }

    #[test]
    fn startup_history_scan_keeps_apply_locked_until_it_is_authoritative()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product_with_data(
            NativePalette::default(),
            Some(directory.path().join("presets.json")),
            Some(directory.path().join("journals")),
        );
        app.set_prefix("final-");
        app.admit_sources_immediately(vec![source]);
        assert!(app.ledger_task.is_some());
        assert!(!app.ledger_ready);

        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 120.0))
            .build_ui_state(|ui, app| app.show_review_bar(ui), app);

        assert!(
            harness
                .get_by_label("Rename 1 entry")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("Wait for rename history checks to finish");
        Ok(())
    }

    #[test]
    fn startup_history_completion_rebuilds_a_pre_initialization_plan() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        let journal_root = directory.path().join("journals");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new(false);
        app.set_prefix("final-");
        app.admit_sources_immediately(vec![source]);
        let stale_plan_id = app
            .plan
            .as_ref()
            .ok_or("the plan was not retained")?
            .plan_id();
        assert_eq!(stale_plan_id, 1);
        app.application.initialize(&journal_root)?;
        let ledger = app.application.ledger_snapshot()?;
        let (sender, receiver) = mpsc::channel();
        sender.send(LedgerMessage::Snapshot {
            result: Ok(ledger),
            announce: false,
        })?;
        app.ledger_task = Some(LedgerTask {
            receiver,
            handle: None,
        });

        app.poll_ledger(&egui::Context::default());

        let authoritative = app.plan.as_ref().ok_or("the plan was not rebuilt")?;
        assert_eq!(authoritative.plan_id(), 2);
        assert_eq!(authoritative.rows()[0].proposed_name(), "final-report.txt");
        assert!(app.plan_is_current);
        assert!(app.ledger_ready);
        Ok(())
    }

    #[test]
    fn applicable_plan_requires_explicit_native_confirmation() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product_with_data(
            NativePalette::default(),
            Some(directory.path().join("presets.json")),
            Some(directory.path().join("journals")),
        );
        settle_ledger(&mut app);
        app.set_prefix("final-");
        app.admit_sources_immediately(vec![source]);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_200.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        let apply = harness.get_by_label("Rename 1 entry");
        assert!(!apply.accesskit_node().is_disabled());
        apply.click();
        harness.run_ok();
        harness.get_by_label("Apply name changes");
        assert_eq!(harness.query_all_by_label("Rename 1 entry").count(), 2);
        assert!(harness.state().pending_confirmation.is_some());
        Ok(())
    }

    #[test]
    fn user_messages_hide_internal_status_and_error_codes() {
        for code in [
            "emptyLiteralSearch",
            "invalidRegex",
            "sourceAdmissionFailed",
            "stalePlanningSnapshot",
            "unknownOverrideSource",
        ] {
            assert_ne!(planning_error_message(code, Locale::English), code);
            assert!(!planning_error_message(code, Locale::Korean).is_ascii());
        }
        for code in ["invalidPresetName", "duplicatePresetName"] {
            assert_ne!(preset_error_message(code, Locale::English), code);
            assert!(!preset_error_message(code, Locale::Korean).is_ascii());
        }
        for code in ["planChanged", "executionFailed"] {
            assert_ne!(apply_error_message(code, Locale::English), code);
            assert!(!apply_error_message(code, Locale::Korean).is_ascii());
        }
        for code in ["inspectionChanged", "actionUnavailable"] {
            assert_ne!(recovery_error_message(code, Locale::English), code);
            assert_ne!(undo_error_message(code, Locale::English), code);
        }
        for status in [
            "completed",
            "rolledBack",
            "reconciliationRequired",
            "recoveryRequired",
            "damaged",
        ] {
            assert_ne!(ledger_status_label(status, Locale::English), status);
            assert!(!ledger_status_label(status, Locale::Korean).is_ascii());
        }
    }

    #[test]
    fn admitted_sources_render_the_application_service_plan() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new(false);
        app.set_prefix("final-");
        app.admit_sources_immediately(vec![source]);

        let Some(plan) = &app.plan else {
            return Err("the native workbench did not retain the service plan".into());
        };
        assert_eq!(plan.rows()[0].proposed_name(), "final-report.txt");
        app.set_prefix("reviewed-");
        app.refresh_plan();
        let Some(plan) = &app.plan else {
            return Err("the refreshed service plan was not retained".into());
        };
        assert_eq!(plan.rows()[0].proposed_name(), "reviewed-report.txt");

        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), app);
        harness.get_by_label("report.txt");
        harness.get_by_label("reviewed-report.txt");
        harness.get_by_label("1 shown");
        Ok(())
    }

    #[test]
    fn preview_column_labels_left_align_and_truncate_long_names() -> Result<(), Box<dyn Error>> {
        let short_name = "a.txt";
        let original_name =
            "quarterly-report-with-a-deliberately-long-name-for-column-alignment.txt";
        let short_proposed_name = format!("reviewed-{short_name}");
        let proposed_name = format!("reviewed-{original_name}");

        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 160.0))
            .build_ui(|ui| {
                ui.horizontal(|ui| {
                    preview_column_label(ui, PREVIEW_SOURCE_COLUMN_WIDTH, "Source");
                    preview_column_label(ui, PREVIEW_PROPOSED_COLUMN_WIDTH, "Proposed");
                });
                ui.horizontal(|ui| {
                    preview_column_label(ui, PREVIEW_SOURCE_COLUMN_WIDTH, short_name);
                    preview_column_label(ui, PREVIEW_PROPOSED_COLUMN_WIDTH, &short_proposed_name);
                });
                ui.horizontal(|ui| {
                    preview_column_label(ui, PREVIEW_SOURCE_COLUMN_WIDTH, original_name);
                    preview_column_label(ui, PREVIEW_PROPOSED_COLUMN_WIDTH, &proposed_name);
                });
            });
        harness.run_ok();
        let source_header = harness.get_by_label("Source").rect();
        let proposed_header = harness.get_by_label("Proposed").rect();
        let short_original = harness.get_by_label(short_name).rect();
        let long_original = harness.get_by_label(original_name).rect();
        let short_proposed = harness.get_by_label(&short_proposed_name).rect();
        let long_proposed = harness.get_by_label(&proposed_name).rect();
        assert!((source_header.left() - short_original.left()).abs() < 0.5);
        assert!((source_header.left() - long_original.left()).abs() < 0.5);
        assert!((proposed_header.left() - short_proposed.left()).abs() < 0.5);
        assert!((proposed_header.left() - long_proposed.left()).abs() < 0.5);
        assert!(long_original.width() <= PREVIEW_SOURCE_COLUMN_WIDTH + 0.5);
        assert!(long_proposed.width() <= PREVIEW_PROPOSED_COLUMN_WIDTH + 0.5);
        Ok(())
    }

    #[test]
    fn explicit_directory_entry_is_projected_without_enumerating_children()
    -> Result<(), Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        let selected = root.path().join("selected-folder");
        fs::create_dir(&selected)?;
        fs::write(selected.join("child.txt"), b"child")?;
        let mut app = RenamewrightApp::new(false);
        app.set_prefix("final-");
        app.admit_sources_immediately(vec![selected]);

        let plan = app.plan.as_ref().ok_or("the directory plan was missing")?;
        assert_eq!(plan.rows().len(), 1);
        assert_eq!(plan.rows()[0].entry_kind(), "directory");
        assert_eq!(plan.rows()[0].proposed_name(), "final-selected-folder");
        Ok(())
    }

    #[test]
    fn every_native_rule_family_builds_a_valid_ordered_service_request()
    -> Result<(), Box<dyn Error>> {
        let mut app = RenamewrightApp::new(false);
        app.rules.clear();
        for (index, kind) in RuleKind::ALL.into_iter().enumerate() {
            app.rules.push(kind.create(index as u64 + 1));
        }

        let request = app.rule_request();
        app.application.validate_rule_request(&request)?;
        assert_eq!(request.rules().len(), RuleKind::ALL.len());
        assert!(
            request
                .rules()
                .iter()
                .enumerate()
                .all(|(index, rule)| rule.rule_id() == index as u64 + 1)
        );
        Ok(())
    }

    #[test]
    fn native_override_renders_service_diagnostics_and_path_free_inspection()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new(false);
        app.set_prefix("");
        app.admit_sources_immediately(vec![source]);
        let source_id = app
            .plan
            .as_ref()
            .and_then(|plan| plan.rows().first())
            .map(|row| row.source_id())
            .ok_or("the admitted row was missing")?;
        app.overrides.insert(source_id, "CON".to_owned());
        app.refresh_plan();
        let plan = app.plan.as_ref().ok_or("the override plan was missing")?;
        assert_eq!(plan.blocked_count(), 1);
        assert!(plan.rows()[0].diagnostics().contains(&"reservedName"));
        app.inspect_plan(true);
        assert!(app.document_task.is_some());
        settle_document(&mut app);
        let inspection = app
            .inspection
            .as_ref()
            .ok_or("the plan inspector did not open")?;
        assert!(inspection.content.contains("\"proposedDisplay\": \"CON\""));
        assert!(
            !inspection
                .content
                .contains(&directory.path().to_string_lossy().to_string())
        );

        let harness = Harness::builder()
            .with_size(egui::vec2(1_180.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), app);
        harness.get_by_label("Reserved Windows name");
        harness.get_by_label("Plan JSON");
        Ok(())
    }

    #[test]
    fn korean_catalog_updates_native_workbench_labels() {
        let mut app = RenamewrightApp::new(false);
        app.locale = Locale::Korean;
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness.get_by_label("파일 추가");
        harness.get_by_label("폴더 이름 추가");
        harness.get_by_label("이름 바꾸기 규칙");
        harness.get_by_label("규칙 순서");
        harness.get_by_role_and_label(egui::accesskit::Role::Button, "앞에 붙이기");
        harness.get_by_label("접두사 텍스트");
        harness.get_by_label("편집기 닫기");
        harness.get_by_label("미리보기");
        harness.get_by_label("전체");
        harness.get_by_label("현재 이름");
        harness.get_by_label("바뀔 이름");
        harness.get_by_label("결과");
        harness.get_by_label("문제 및 안내");
        harness.get_by_label("이름 변경 기록");
        harness.get_by_label("화면 설정");
        harness.get_by_label("고급 도구");
        harness.get_by_label("규칙 앞으로 이동");
        harness.get_by_label("규칙 1 드래그");
        harness.get_by_label("규칙 제거");
        harness.get_by_label("IMG_00000.jpg, 미리보기 행");
        harness.get_by_label("9990개 이름 바꾸기");
        harness.get_by_label("샘플 미리보기는 적용할 수 없습니다");
    }

    #[test]
    fn korean_apply_confirmation_uses_task_language_and_exact_count() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product_with_data(
            NativePalette::default(),
            Some(directory.path().join("presets.json")),
            Some(directory.path().join("journals")),
        );
        settle_ledger(&mut app);
        app.locale = Locale::Korean;
        app.set_prefix("완료-");
        app.admit_sources_immediately(vec![source]);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_200.0, 760.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness.get_by_label("1개 이름 바꾸기").click();
        harness.run_ok();

        harness.get_by_label("이름 변경 적용");
        assert_eq!(harness.query_all_by_label("1개 이름 바꾸기").count(), 2);
        harness.get_by_label("취소");
        Ok(())
    }

    #[test]
    fn korean_rule_editors_use_explanatory_field_labels() {
        let cases: &[(RuleKind, &[&str])] = &[
            (RuleKind::Prefix, &["접두사 텍스트"]),
            (RuleKind::Suffix, &["접미사 텍스트"]),
            (RuleKind::LiteralReplace, &["찾기", "바꿀 내용"]),
            (RuleKind::RegexReplace, &["패턴", "바꿀 내용"]),
            (
                RuleKind::Sequence,
                &[
                    "번호 매기기 단위",
                    "번호 순서 기준",
                    "시작 번호",
                    "증가값",
                    "자릿수",
                    "번호와 이름 사이",
                    "번호 위치",
                ],
            ),
            (RuleKind::Extension, &["처리 방법", "새 확장자"]),
            (RuleKind::Case, &["바꿀 부분", "바꿀 형태"]),
            (
                RuleKind::WhitespaceCleanup,
                &["바꿀 부분", "공백을 바꿀 문자"],
            ),
            (
                RuleKind::UnicodeNormalization,
                &["바꿀 부분", "유니코드 형식"],
            ),
            (
                RuleKind::Range,
                &["바꿀 부분", "처리 방법", "위치 기준", "시작 위치", "끝까지"],
            ),
            (
                RuleKind::CharacterClass,
                &["바꿀 부분", "처리 방법", "문자 종류"],
            ),
        ];

        for (kind, labels) in cases {
            let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
            app.locale = Locale::Korean;
            app.rules = vec![kind.create(1)];
            app.rule_editor_open = true;
            let harness = Harness::builder()
                .with_size(egui::vec2(1_180.0, 760.0))
                .build_ui_state(|ui, app| app.show(ui), app);
            for label in *labels {
                harness.get_by_label(label);
            }
            assert!(harness.query_by_label("한글 IME 입력 확인").is_none());
        }
    }

    #[test]
    fn opening_initial_language_menu_uses_a_safe_label_and_starts_korean_font_loading() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                RenamewrightApp::new_product(NativePalette::default(), None),
            );
        assert_eq!(harness.state().locale, Locale::English);
        assert_eq!(harness.state().korean_font_state, KoreanFontState::Idle);

        harness.get_by_value("English").click();
        harness.run_ok();

        assert_eq!(harness.state().locale, Locale::English);
        let korean_font_state = harness.state().korean_font_state;
        assert_ne!(korean_font_state, KoreanFontState::Idle);
        harness.get_by_label(Locale::Korean.picker_label(korean_font_state));
        if korean_font_state != KoreanFontState::Ready {
            assert!(Locale::Korean.picker_label(korean_font_state).is_ascii());
            assert!(!Locale::Korean.picker_enabled(korean_font_state));
        }
        assert_eq!(
            Locale::Korean.picker_label(KoreanFontState::Ready),
            "한국어"
        );
        assert!(Locale::Korean.picker_enabled(KoreanFontState::Ready));
        assert_eq!(
            Locale::Korean.picker_label(KoreanFontState::Unavailable),
            "Korean (font unavailable)"
        );
        assert!(!Locale::Korean.picker_enabled(KoreanFontState::Unavailable));
    }

    #[test]
    fn korean_font_loading_is_deferred_until_korean_text_is_needed() {
        let english_app = RenamewrightApp::new_product(NativePalette::default(), None);
        let english_harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), english_app);
        assert_eq!(
            english_harness.state().korean_font_state,
            KoreanFontState::Idle
        );
        assert!(english_harness.state().korean_font_task.is_none());

        let mut korean_app = RenamewrightApp::new_product(NativePalette::default(), None);
        korean_app.locale = Locale::Korean;
        let korean_harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), korean_app);
        assert_ne!(
            korean_harness.state().korean_font_state,
            KoreanFontState::Idle
        );
    }

    #[test]
    fn korean_filename_starts_font_loading_in_the_english_ui() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("보고서.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.admit_sources_immediately(vec![source]);

        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        assert_eq!(harness.state().locale, Locale::English);
        assert_ne!(harness.state().korean_font_state, KoreanFontState::Idle);
        Ok(())
    }

    #[test]
    fn korean_rule_text_starts_font_loading_without_sources() {
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.rules = vec![RuleRequestDto::Prefix {
            rule_id: 1,
            enabled: true,
            value: "정리_".to_owned(),
        }];

        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        assert_eq!(harness.state().locale, Locale::English);
        assert_ne!(harness.state().korean_font_state, KoreanFontState::Idle);
    }

    #[test]
    fn direct_rule_button_focuses_input_and_enter_commits_the_edit() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = RenamewrightApp::new_product(NativePalette::default(), None);
        app.admit_sources_immediately(vec![source]);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness
            .get_by_role_and_label(
                egui::accesskit::Role::Button,
                RuleKind::Prefix.label(Locale::English),
            )
            .click();
        harness.run_ok();
        let prefix_input = harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, semantics::PREFIX_LABEL);
        assert!(harness.get_by_label(semantics::PREFIX_LABEL).is_focused());
        assert_eq!(
            prefix_input.accesskit_node().role(),
            egui::accesskit::Role::TextInput
        );
        harness.event(egui::Event::Text("한글_".to_owned()));
        harness.run_ok();
        thread::sleep(PLANNING_DEBOUNCE);
        for _ in 0..100 {
            harness.run_ok();
            if harness.query_by_label("한글_report.txt").is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        harness.get_by_label("한글_report.txt");
        harness.key_press(egui::Key::Enter);
        harness.run_ok();

        let Some(RuleRequestDto::Prefix { value, .. }) = harness.state().rules.first() else {
            return Err("the direct prefix rule was missing".into());
        };
        assert_eq!(value, "한글_");
        assert!(!harness.state().rule_editor_open);
        Ok(())
    }

    #[test]
    fn escape_removes_an_unedited_direct_rule_draft() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                RenamewrightApp::new_product(NativePalette::default(), None),
            );

        harness
            .get_by_role_and_label(
                egui::accesskit::Role::Button,
                RuleKind::Prefix.label(Locale::English),
            )
            .click();
        harness.run_ok();
        assert_eq!(harness.state().rules.len(), 1);
        harness.key_press(egui::Key::Escape);
        harness.run_ok();
        assert!(harness.state().rules.is_empty());
        assert!(!harness.state().rule_editor_open);
    }

    #[test]
    fn native_presets_persist_and_restore_ordered_rules() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let preset_path = directory.path().join("presets.json");
        let mut app = RenamewrightApp::new_with_storage(
            false,
            NativePalette::default(),
            Some(preset_path.clone()),
        );
        app.rules = vec![
            RuleKind::Sequence.create(4),
            RuleRequestDto::Prefix {
                rule_id: 8,
                enabled: true,
                value: "archive-".to_owned(),
            },
        ];
        app.preset_name = "Archive order".to_owned();
        app.save_current_preset();
        assert!(preset_path.is_file());

        let mut restored =
            RenamewrightApp::new_with_storage(false, NativePalette::default(), Some(preset_path));
        assert_eq!(restored.presets.presets().len(), 1);
        restored.rules = vec![RuleKind::Case.create(1)];
        restored.apply_preset(1);
        assert_eq!(restored.rules.len(), 2);
        assert_eq!(restored.rules[0].rule_id(), 4);
        assert_eq!(restored.rules[1].rule_id(), 8);
        assert_eq!(restored.next_rule_id, 9);
        Ok(())
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_mode_does_not_persist_appearance_in_product_storage() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let root = AutomationRoot::open(directory.path())?;
        let mut app = RenamewrightApp::new_automated(
            NativePalette::default(),
            root,
            AutomationProfile::Empty,
        );
        app.appearance.theme = AppearanceTheme::Dark;
        let mut storage = MemoryStorage::default();

        eframe::App::save(&mut app, &mut storage);

        assert!(storage.values.is_empty());
        Ok(())
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_root_validation_is_read_only() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = AutomationRoot::open(directory.path())?;
        let second = AutomationRoot::open(directory.path())?;

        assert_eq!(root.root(), fs::canonicalize(directory.path())?);
        assert_eq!(second.root(), root.root());
        assert_eq!(fs::read_dir(directory.path())?.count(), 0);
        Ok(())
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_profiles_are_bounded_and_path_free() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            AutomationProfile::parse("empty".as_ref())?,
            AutomationProfile::Empty
        );
        assert_eq!(
            AutomationProfile::parse("performance".as_ref())?,
            AutomationProfile::Performance
        );
        for rejected in ["performance.json", "../performance", "C:\\fixture.json"] {
            let error = AutomationProfile::parse(rejected.as_ref())
                .err()
                .ok_or("a path-bearing automation profile was accepted")?;
            assert_eq!(error.kind(), AutomationRootErrorKind::InvalidProfile);
        }
        Ok(())
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_performance_profile_is_synthetic_and_read_only() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = AutomationRoot::open(directory.path())?;
        let mut app = RenamewrightApp::new_automated(
            NativePalette::default(),
            root,
            AutomationProfile::Performance,
        );
        app.tools_open = true;
        assert!(!app.filesystem_authority_enabled());
        assert!(app.preset_path.is_none());
        assert!(app.journal_root.is_none());
        assert!(app.ledger_task.is_none());
        app.preset_name = "must-not-persist".to_owned();
        app.save_current_preset();
        assert!(app.presets.presets().is_empty());
        app.start_confirmed_mutation(PendingConfirmation::Apply {
            plan_id: 1,
            changed_count: 1,
        });
        assert!(app.mutation_task.is_none());
        assert_eq!(app.status, "Automation mode is read-only");
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness.get_by_label("Automation mode is read-only");
        harness.get_by_label("10000 shown");
        assert!(
            harness
                .get_by_label(semantics::ADD_FILES)
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            harness
                .get_by_label(semantics::ADD_FOLDER)
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            harness
                .get_by_label("Rename 9990 entries")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            harness
                .get_by_label(semantics::SAVE_PRESET)
                .accesskit_node()
                .is_disabled()
        );
        assert_eq!(fs::read_dir(directory.path())?.count(), 0);
        Ok(())
    }
}
