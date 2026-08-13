#![forbid(unsafe_code)]

// Hallmark · pre-emit critique: P5 H5 E4 S5 R5 V4
// Hallmark · macrostructure: workbench · theme: Cobalt · slop: pass (native-app scope)

use std::collections::BTreeMap;
use std::path::PathBuf;

use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Layout, RichText, ScrollArea, Stroke,
};
use renamewright_application::{
    ApplicationService, CaseModeDto, CharacterClassDto, CharacterClassOperationDto,
    ExtensionOperationDto, FilenamePartDto, PlanDto, PresetDocumentDto, RangeOperationDto,
    RangeOriginDto, RulePipelineRequestDto, RuleRequestDto, SequenceOrderDto, SequencePlacementDto,
    SequenceScopeDto, SourceOverrideDto, UnicodeNormalizationFormDto,
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
    pub const ADD_RULE: &str = "Add rule";
    pub const MOVE_RULE_UP: &str = "Move rule up";
    pub const MOVE_RULE_DOWN: &str = "Move rule down";
    pub const REMOVE_RULE: &str = "Remove rule";
    pub const ENABLE_RULE: &str = "Enable rule";
    pub const LANGUAGE: &str = "Language";
    pub const DIAGNOSTIC_FILTER: &str = "Diagnostic filter";
    pub const INSPECT_JSON: &str = "Inspect JSON";
    pub const INSPECT_CSV: &str = "Inspect CSV";
    pub const EXPORT_JSON: &str = "Export JSON";
    pub const EXPORT_CSV: &str = "Export CSV";
    pub const CLOSE_INSPECTOR: &str = "Close inspector";
    pub const SAVE_OVERRIDE: &str = "Save override";
    pub const CANCEL_OVERRIDE: &str = "Cancel override";
    pub const PRESETS: &str = "Local presets";
    pub const PRESET_NAME: &str = "Preset name";
    pub const SAVE_PRESET: &str = "Save preset";
    pub const APPLY_PRESET: &str = "Apply preset";
    pub const DELETE_PRESET: &str = "Delete preset";
    pub const AUTOMATION_BANNER: &str = "AUTOMATION TEST MODE";
    pub const HIGH_CONTRAST_ACTIVE: &str = "Windows high contrast palette active";
}

#[cfg(feature = "automation")]
pub mod automation {
    use std::fmt::{Display, Formatter};
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read as _, Write as _};
    use std::net::{TcpListener, TcpStream, ToSocketAddrs as _};
    use std::path::{Component, Path, PathBuf};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use eframe::egui;
    use egui_inspection::{InspectionPlugin, Request, Response};
    use serde::Deserialize;

    pub const AUTOMATION_BIND_ADDRESS: &str = "127.0.0.1:45719";
    pub const MAX_AUTOMATION_FIXTURE_BYTES: u64 = 256 * 1024;
    pub const MAX_AUTOMATION_MESSAGE_BYTES: usize = 1024 * 1024;
    pub const MAX_AUTOMATION_TEXT_BYTES: usize = 4 * 1024;
    pub const MAX_AUTOMATION_EVENTS: usize = 256;
    pub const MAX_AUTOMATION_REQUESTS_PER_CONNECTION: usize = 128;
    pub const MAX_AUTOMATION_SOURCES: usize = 10_000;
    const MAX_AUTOMATION_RELATIVE_PATH_BYTES: usize = 4 * 1024;
    const MAX_AUTOMATION_CONNECTION_DURATION: Duration = Duration::from_secs(120);
    const AUTOMATION_IO_TIMEOUT: Duration = Duration::from_secs(5);
    const AUTOMATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
    const MAX_AUTOMATION_VIEWPORT_WIDTH: u32 = 3_840;
    const MAX_AUTOMATION_VIEWPORT_HEIGHT: u32 = 2_160;
    const LOCK_FILE_NAME: &str = ".renamewright-automation.lock";
    const FIXTURE_DIRECTORY_NAME: &str = "fixtures";
    const STATE_DIRECTORY_NAME: &str = "state";
    const JOURNAL_DIRECTORY_NAME: &str = "journals";

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum AutomationRootErrorKind {
        RootMustBeAbsolute,
        RootUnavailable,
        RootNotDirectory,
        ReparsePointRejected,
        ConcurrentSession,
        InvalidRelativePath,
        RelativePathTooLong,
        FixtureUnavailable,
        FixtureTooLarge,
        InvalidFixture,
        FixtureEscapedRoot,
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

    #[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    pub enum AutomationFilter {
        All,
        Changed,
        Blocked,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct AutomationFixture {
        schema_version: u16,
        #[serde(default)]
        prefix: Option<String>,
        #[serde(default)]
        source_query: Option<String>,
        #[serde(default)]
        filter: Option<AutomationFilter>,
        #[serde(default)]
        sources: Vec<String>,
        #[serde(skip)]
        resolved_sources: Vec<PathBuf>,
    }

    impl AutomationFixture {
        pub fn parse(bytes: &[u8]) -> Result<Self, AutomationRootError> {
            let fixture: Self = serde_json::from_slice(bytes)
                .map_err(|_| AutomationRootError::new(AutomationRootErrorKind::InvalidFixture))?;
            if fixture.schema_version != 1
                || fixture
                    .prefix
                    .as_ref()
                    .is_some_and(|value| value.len() > MAX_AUTOMATION_TEXT_BYTES)
                || fixture
                    .source_query
                    .as_ref()
                    .is_some_and(|value| value.len() > MAX_AUTOMATION_TEXT_BYTES)
                || fixture.sources.len() > MAX_AUTOMATION_SOURCES
                || fixture.sources.iter().any(|source| {
                    source.is_empty() || source.len() > MAX_AUTOMATION_RELATIVE_PATH_BYTES
                })
            {
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::InvalidFixture,
                ));
            }
            Ok(fixture)
        }

        #[must_use]
        pub fn prefix(&self) -> Option<&str> {
            self.prefix.as_deref()
        }

        #[must_use]
        pub fn source_query(&self) -> Option<&str> {
            self.source_query.as_deref()
        }

        #[must_use]
        pub const fn filter(&self) -> Option<AutomationFilter> {
            self.filter
        }

        #[must_use]
        pub fn sources(&self) -> &[PathBuf] {
            &self.resolved_sources
        }
    }

    #[derive(Debug)]
    pub struct AutomationRoot {
        canonical_root: PathBuf,
        fixture_root: PathBuf,
        state_root: PathBuf,
        journal_root: PathBuf,
        lock_path: PathBuf,
        _lock: File,
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
            let lock_path = canonical_root.join(LOCK_FILE_NAME);
            let mut lock = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
                .map_err(|_| {
                    AutomationRootError::new(AutomationRootErrorKind::ConcurrentSession)
                })?;
            if writeln!(lock, "{}", std::process::id()).is_err() || lock.sync_all().is_err() {
                let _ = fs::remove_file(&lock_path);
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::RootUnavailable,
                ));
            }

            let result = Self::prepare(canonical_root, lock_path, lock);
            if result.is_err() {
                let _ = fs::remove_file(root.join(LOCK_FILE_NAME));
            }
            result
        }

        fn prepare(
            canonical_root: PathBuf,
            lock_path: PathBuf,
            lock: File,
        ) -> Result<Self, AutomationRootError> {
            let fixture_root = prepare_child_directory(&canonical_root, FIXTURE_DIRECTORY_NAME)?;
            let state_root = prepare_child_directory(&canonical_root, STATE_DIRECTORY_NAME)?;
            let journal_root = prepare_child_directory(&canonical_root, JOURNAL_DIRECTORY_NAME)?;
            Ok(Self {
                canonical_root,
                fixture_root,
                state_root,
                journal_root,
                lock_path,
                _lock: lock,
            })
        }

        #[must_use]
        pub fn root(&self) -> &Path {
            &self.canonical_root
        }

        #[must_use]
        pub fn state_root(&self) -> &Path {
            &self.state_root
        }

        #[must_use]
        pub fn journal_root(&self) -> &Path {
            &self.journal_root
        }

        pub fn read_fixture(&self, relative: &Path) -> Result<Vec<u8>, AutomationRootError> {
            validate_relative_path(relative)?;
            let candidate = self.fixture_root.join(relative);
            verify_existing_path(&self.fixture_root, &candidate)?;
            let metadata = fs::metadata(&candidate).map_err(|_| {
                AutomationRootError::new(AutomationRootErrorKind::FixtureUnavailable)
            })?;
            if !metadata.is_file() {
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::FixtureUnavailable,
                ));
            }
            if metadata.len() > MAX_AUTOMATION_FIXTURE_BYTES {
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::FixtureTooLarge,
                ));
            }
            let file = File::open(&candidate).map_err(|_| {
                AutomationRootError::new(AutomationRootErrorKind::FixtureUnavailable)
            })?;
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            file.take(MAX_AUTOMATION_FIXTURE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| {
                    AutomationRootError::new(AutomationRootErrorKind::FixtureUnavailable)
                })?;
            if bytes.len() as u64 > MAX_AUTOMATION_FIXTURE_BYTES {
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::FixtureTooLarge,
                ));
            }
            Ok(bytes)
        }

        pub fn load_fixture(
            &self,
            relative: &Path,
        ) -> Result<AutomationFixture, AutomationRootError> {
            let mut fixture = AutomationFixture::parse(&self.read_fixture(relative)?)?;
            let mut resolved_sources = Vec::with_capacity(fixture.sources.len());
            for relative_source in &fixture.sources {
                let relative_source = Path::new(relative_source);
                validate_relative_path(relative_source)?;
                let candidate = self.fixture_root.join(relative_source);
                verify_existing_path(&self.fixture_root, &candidate)?;
                let metadata = fs::metadata(&candidate).map_err(|_| {
                    AutomationRootError::new(AutomationRootErrorKind::FixtureUnavailable)
                })?;
                if !metadata.is_file() {
                    return Err(AutomationRootError::new(
                        AutomationRootErrorKind::FixtureUnavailable,
                    ));
                }
                resolved_sources.push(candidate);
            }
            fixture.resolved_sources = resolved_sources;
            Ok(fixture)
        }
    }

    impl Drop for AutomationRoot {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.lock_path);
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

    fn serve_connection(stream: TcpStream, ctx: &egui::Context) -> std::io::Result<()> {
        stream.set_read_timeout(Some(AUTOMATION_IO_TIMEOUT))?;
        stream.set_write_timeout(Some(AUTOMATION_IO_TIMEOUT))?;
        let mut reader = std::io::BufReader::new(stream.try_clone()?);
        let mut writer = std::io::BufWriter::new(stream);
        egui_inspection::protocol::write_handshake(&mut writer)?;
        let started = Instant::now();

        for _ in 0..MAX_AUTOMATION_REQUESTS_PER_CONNECTION {
            if started.elapsed() > MAX_AUTOMATION_CONNECTION_DURATION {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the automation connection exceeded its runtime bound",
                ));
            }
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
            let response = receiver
                .recv_timeout(AUTOMATION_REQUEST_TIMEOUT)
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
        }
    }

    fn validate_relative_path(path: &Path) -> Result<(), AutomationRootError> {
        if path.as_os_str().as_encoded_bytes().len() > MAX_AUTOMATION_RELATIVE_PATH_BYTES {
            return Err(AutomationRootError::new(
                AutomationRootErrorKind::RelativePathTooLong,
            ));
        }
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(AutomationRootError::new(
                AutomationRootErrorKind::InvalidRelativePath,
            ));
        }
        Ok(())
    }

    fn prepare_child_directory(root: &Path, name: &str) -> Result<PathBuf, AutomationRootError> {
        let path = root.join(name);
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::RootUnavailable,
                ));
            }
        }
        let metadata = fs::symlink_metadata(&path)
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
        let canonical = fs::canonicalize(&path)
            .map_err(|_| AutomationRootError::new(AutomationRootErrorKind::RootUnavailable))?;
        if !canonical.starts_with(root) {
            return Err(AutomationRootError::new(
                AutomationRootErrorKind::FixtureEscapedRoot,
            ));
        }
        Ok(canonical)
    }

    fn verify_existing_path(root: &Path, candidate: &Path) -> Result<(), AutomationRootError> {
        let relative = candidate
            .strip_prefix(root)
            .map_err(|_| AutomationRootError::new(AutomationRootErrorKind::FixtureEscapedRoot))?;
        let mut current = root.to_path_buf();
        for component in relative.components() {
            let Component::Normal(component) = component else {
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::InvalidRelativePath,
                ));
            };
            current.push(component);
            let metadata = fs::symlink_metadata(&current).map_err(|_| {
                AutomationRootError::new(AutomationRootErrorKind::FixtureUnavailable)
            })?;
            if metadata_is_reparse_point(&metadata) {
                return Err(AutomationRootError::new(
                    AutomationRootErrorKind::ReparsePointRejected,
                ));
            }
        }
        let canonical = fs::canonicalize(candidate)
            .map_err(|_| AutomationRootError::new(AutomationRootErrorKind::FixtureUnavailable))?;
        if !canonical.starts_with(root) {
            return Err(AutomationRootError::new(
                AutomationRootErrorKind::FixtureEscapedRoot,
            ));
        }
        Ok(())
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
        use std::io::Cursor;

        use eframe::egui;
        use egui_inspection::Request;

        use super::{
            MAX_AUTOMATION_EVENTS, MAX_AUTOMATION_MESSAGE_BYTES, read_bounded_message,
            request_is_bounded, serve_bounded,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Locale {
    #[default]
    English,
    Korean,
}

impl Locale {
    const ALL: [Self; 2] = [Self::English, Self::Korean];

    const fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Korean => "한국어",
        }
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
            Self::Prefix => locale.text("Add prefix", "접두사 추가"),
            Self::Suffix => locale.text("Add suffix", "접미사 추가"),
            Self::LiteralReplace => locale.text("Replace text", "텍스트 바꾸기"),
            Self::RegexReplace => locale.text("Replace by pattern", "패턴으로 바꾸기"),
            Self::Sequence => locale.text("Add sequence", "일련번호 추가"),
            Self::Extension => locale.text("Change extension", "확장자 변경"),
            Self::Case => locale.text("Change case", "대소문자 변경"),
            Self::WhitespaceCleanup => locale.text("Clean whitespace", "공백 정리"),
            Self::UnicodeNormalization => locale.text("Normalize Unicode", "유니코드 정규화"),
            Self::Range => locale.text("Select character range", "문자 범위 선택"),
            Self::CharacterClass => locale.text("Filter character class", "문자 종류 필터"),
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
    SequenceOverflow,
}

impl DiagnosticFilter {
    const ALL: [Self; 13] = [
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
        Self::SequenceOverflow,
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
            Self::SequenceOverflow => Some("sequenceOverflow"),
        }
    }

    const fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::All => locale.text("All diagnostics", "모든 진단"),
            Self::Unchanged => locale.text("No change", "변경 없음"),
            Self::EmptyName => locale.text("Empty name", "빈 이름"),
            Self::IllegalCharacter => locale.text("Illegal Windows character", "Windows 금지 문자"),
            Self::TrailingDotOrSpace => locale.text("Trailing dot or space", "끝의 점 또는 공백"),
            Self::ReservedName => locale.text("Reserved Windows name", "Windows 예약 이름"),
            Self::NameTooLong => locale.text("Name too long", "이름이 너무 김"),
            Self::DuplicateDestination => locale.text("Duplicate target", "대상 이름 중복"),
            Self::UnsupportedEncoding => {
                locale.text("Unsupported encoding", "지원하지 않는 인코딩")
            }
            Self::OccupiedDestination => locale.text("Target exists", "대상이 이미 존재함"),
            Self::StaleSource => locale.text("Source changed", "원본이 변경됨"),
            Self::ParentUnavailable => locale.text("Parent unavailable", "상위 폴더 확인 불가"),
            Self::SequenceOverflow => locale.text("Sequence overflow", "일련번호 범위 초과"),
        }
    }
}

fn diagnostic_label(code: &str, locale: Locale) -> &str {
    DiagnosticFilter::ALL
        .into_iter()
        .find(|candidate| candidate.code() == Some(code))
        .map_or(code, |candidate| candidate.label(locale))
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

fn filename_part_control(ui: &mut egui::Ui, target: &mut FilenamePartDto, locale: Locale) -> bool {
    let before = *target;
    egui::ComboBox::from_id_salt("filename-part")
        .selected_text(filename_part_label(*target, locale))
        .show_ui(ui, |ui| {
            for candidate in [
                FilenamePartDto::WholeName,
                FilenamePartDto::Stem,
                FilenamePartDto::Extension,
            ] {
                ui.selectable_value(target, candidate, filename_part_label(candidate, locale));
            }
        });
    before != *target
}

fn rule_editor(ui: &mut egui::Ui, rule: &mut RuleRequestDto, locale: Locale) -> bool {
    let mut changed = ui
        .checkbox(
            rule_enabled_mut(rule),
            locale.text(semantics::ENABLE_RULE, "규칙 사용"),
        )
        .changed();
    ui.add_space(4.0);
    match rule {
        RuleRequestDto::Prefix { value, .. } => {
            let label = ui.label(locale.text(semantics::PREFIX_LABEL, "접두사 텍스트"));
            changed |= ui
                .add(
                    egui::TextEdit::singleline(value)
                        .id_salt("rule.prefix.value")
                        .hint_text(locale.text("Text", "텍스트")),
                )
                .labelled_by(label.id)
                .changed();
        }
        RuleRequestDto::Suffix { value, .. } => {
            let label = ui.label(locale.text("Suffix text", "접미사 텍스트"));
            changed |= ui
                .add(
                    egui::TextEdit::singleline(value)
                        .id_salt("rule.suffix.value")
                        .hint_text(locale.text("Text", "텍스트")),
                )
                .labelled_by(label.id)
                .changed();
        }
        RuleRequestDto::LiteralReplace {
            search,
            replacement,
            ..
        } => {
            ui.label(locale.text("Find", "찾기"));
            changed |= ui.text_edit_singleline(search).changed();
            ui.label(locale.text("Replace with", "바꿀 내용"));
            changed |= ui.text_edit_singleline(replacement).changed();
        }
        RuleRequestDto::RegexReplace {
            pattern,
            replacement,
            ..
        } => {
            ui.label(locale.text("Pattern", "패턴"));
            changed |= ui.text_edit_singleline(pattern).changed();
            ui.label(locale.text("Replacement", "바꿀 내용"));
            changed |= ui.text_edit_singleline(replacement).changed();
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
            ui.label(locale.text("Scope", "범위"));
            let before = *scope;
            egui::ComboBox::from_id_salt("sequence-scope")
                .selected_text(match scope {
                    SequenceScopeDto::AllSources => locale.text("All sources", "모든 원본"),
                    SequenceScopeDto::PerParent => locale.text("Per folder", "폴더별"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        scope,
                        SequenceScopeDto::AllSources,
                        locale.text("All sources", "모든 원본"),
                    );
                    ui.selectable_value(
                        scope,
                        SequenceScopeDto::PerParent,
                        locale.text("Per folder", "폴더별"),
                    );
                });
            changed |= before != *scope;
            ui.label(locale.text("Order", "정렬"));
            let before = *order;
            egui::ComboBox::from_id_salt("sequence-order")
                .selected_text(match order {
                    SequenceOrderDto::SourceOrder => locale.text("Source order", "원본 순서"),
                    SequenceOrderDto::NameAscending => {
                        locale.text("Name ascending", "이름 오름차순")
                    }
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        order,
                        SequenceOrderDto::SourceOrder,
                        locale.text("Source order", "원본 순서"),
                    );
                    ui.selectable_value(
                        order,
                        SequenceOrderDto::NameAscending,
                        locale.text("Name ascending", "이름 오름차순"),
                    );
                });
            changed |= before != *order;
            ui.horizontal(|ui| {
                ui.label(locale.text("Start", "시작"));
                changed |= ui.add(egui::DragValue::new(start)).changed();
                ui.label(locale.text("Step", "증가"));
                changed |= ui
                    .add(egui::DragValue::new(step).range(1..=u64::MAX))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label(locale.text("Digits", "자리수"));
                changed |= ui
                    .add(egui::DragValue::new(padding).range(1..=32))
                    .changed();
                ui.label(locale.text("Separator", "구분자"));
                changed |= ui.text_edit_singleline(separator).changed();
            });
            let before = *placement;
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
                changed |= ui
                    .add(egui::TextEdit::singleline(value).hint_text("txt"))
                    .changed();
            }
        }
        RuleRequestDto::Case { target, mode, .. } => {
            changed |= filename_part_control(ui, target, locale);
            let before = *mode;
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
            changed |= filename_part_control(ui, target, locale);
            ui.label(locale.text("Replacement", "바꿀 내용"));
            changed |= ui.text_edit_singleline(replacement).changed();
        }
        RuleRequestDto::UnicodeNormalization { target, form, .. } => {
            changed |= filename_part_control(ui, target, locale);
            let before = *form;
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
        }
        RuleRequestDto::Range {
            target,
            operation,
            origin,
            offset,
            length,
            ..
        } => {
            changed |= filename_part_control(ui, target, locale);
            let before = *operation;
            egui::ComboBox::from_id_salt("range-operation")
                .selected_text(match operation {
                    RangeOperationDto::Keep => locale.text("Keep", "유지"),
                    RangeOperationDto::Remove => locale.text("Remove", "제거"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        operation,
                        RangeOperationDto::Keep,
                        locale.text("Keep", "유지"),
                    );
                    ui.selectable_value(
                        operation,
                        RangeOperationDto::Remove,
                        locale.text("Remove", "제거"),
                    );
                });
            changed |= before != *operation;
            let before = *origin;
            egui::ComboBox::from_id_salt("range-origin")
                .selected_text(match origin {
                    RangeOriginDto::Start => locale.text("From start", "앞에서"),
                    RangeOriginDto::End => locale.text("From end", "뒤에서"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        origin,
                        RangeOriginDto::Start,
                        locale.text("From start", "앞에서"),
                    );
                    ui.selectable_value(
                        origin,
                        RangeOriginDto::End,
                        locale.text("From end", "뒤에서"),
                    );
                });
            changed |= before != *origin;
            ui.horizontal(|ui| {
                ui.label(locale.text("Offset", "오프셋"));
                changed |= ui.add(egui::DragValue::new(offset)).changed();
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
                ui.label(locale.text("Length", "길이"));
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
            changed |= filename_part_control(ui, target, locale);
            let before = *operation;
            egui::ComboBox::from_id_salt("class-operation")
                .selected_text(match operation {
                    CharacterClassOperationDto::Keep => locale.text("Keep", "유지"),
                    CharacterClassOperationDto::Remove => locale.text("Remove", "제거"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        operation,
                        CharacterClassOperationDto::Keep,
                        locale.text("Keep", "유지"),
                    );
                    ui.selectable_value(
                        operation,
                        CharacterClassOperationDto::Remove,
                        locale.text("Remove", "제거"),
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
    changed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanFilter {
    All,
    Changed,
    Blocked,
}

impl PlanFilter {
    const ALL: [Self; 3] = [Self::All, Self::Changed, Self::Blocked];

    const fn label(self, locale: Locale) -> &'static str {
        match self {
            Self::All => locale.text(semantics::FILTER_ALL, "전체"),
            Self::Changed => locale.text(semantics::FILTER_CHANGED, "변경됨"),
            Self::Blocked => locale.text(semantics::FILTER_BLOCKED, "차단됨"),
        }
    }
}

#[derive(Debug)]
pub struct NativeSpikeApp {
    application: ApplicationService,
    plan: Option<PlanDto>,
    rules: Vec<RuleRequestDto>,
    overrides: BTreeMap<u64, String>,
    new_rule_kind: RuleKind,
    next_rule_id: u64,
    source_query: String,
    filter: PlanFilter,
    diagnostic_filter: DiagnosticFilter,
    selected_rule: usize,
    locale: Locale,
    override_editor: Option<OverrideEditor>,
    inspection: Option<InspectionDocument>,
    preset_path: Option<PathBuf>,
    presets: PresetDocumentDto,
    preset_name: String,
    status: String,
    palette: NativePalette,
    #[cfg(feature = "automation")]
    automation_mode: bool,
    #[cfg(feature = "automation")]
    _automation_root: Option<automation::AutomationRoot>,
}

impl NativeSpikeApp {
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
        let (presets, preset_status) = preset_path.as_ref().map_or_else(
            || (PresetDocumentDto::default(), None),
            |path| match PresetDocumentDto::load(path) {
                Ok(document) => (document, None),
                Err(error) => (
                    PresetDocumentDto::default(),
                    Some(format!("Presets unavailable ({})", error.code())),
                ),
            },
        );
        Self {
            application: ApplicationService::default(),
            plan: None,
            rules: vec![RuleRequestDto::Prefix {
                rule_id: 1,
                enabled: true,
                value: "정리_".to_owned(),
            }],
            overrides: BTreeMap::new(),
            new_rule_kind: RuleKind::Suffix,
            next_rule_id: 2,
            source_query: String::new(),
            filter: PlanFilter::All,
            diagnostic_filter: DiagnosticFilter::All,
            selected_rule: 0,
            locale: Locale::English,
            override_editor: None,
            inspection: None,
            preset_path,
            presets,
            preset_name: String::new(),
            status: preset_status.unwrap_or_else(|| format!("{SAMPLE_COUNT} sample entries ready")),
            palette,
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
        fixture: Option<&automation::AutomationFixture>,
    ) -> Self {
        let preset_path = automation_root.state_root().join("presets.json");
        let mut app = Self::new_with_storage(true, palette, Some(preset_path));
        if let Some(fixture) = fixture {
            if let Some(prefix) = fixture.prefix() {
                app.set_prefix(prefix);
            }
            if let Some(source_query) = fixture.source_query() {
                app.source_query = source_query.to_owned();
            }
            if let Some(filter) = fixture.filter() {
                app.filter = match filter {
                    automation::AutomationFilter::All => PlanFilter::All,
                    automation::AutomationFilter::Changed => PlanFilter::Changed,
                    automation::AutomationFilter::Blocked => PlanFilter::Blocked,
                };
            }
            if fixture.sources().is_empty() {
                app.status = "Automation fixture loaded".to_owned();
            } else {
                app.admit_sources(fixture.sources().to_vec());
                app.status = format!(
                    "Automation fixture loaded · {} sources",
                    fixture.sources().len()
                );
            }
        }
        app._automation_root = Some(automation_root);
        app
    }

    fn row_is_blocked(index: usize) -> bool {
        index > 0 && index.is_multiple_of(997)
    }

    fn row_is_visible(&self, index: usize) -> bool {
        if let Some(plan) = &self.plan {
            let Some(row) = plan.rows().get(index) else {
                return false;
            };
            let matches_filter = match self.filter {
                PlanFilter::All => true,
                PlanFilter::Changed => row.status() == "changed",
                PlanFilter::Blocked => row.status() == "blocked",
            };
            let query = self.source_query.trim().to_lowercase();
            let matches_query = query.is_empty()
                || row.original_name().to_lowercase().contains(&query)
                || row.proposed_name().to_lowercase().contains(&query);
            let matches_diagnostic = self
                .diagnostic_filter
                .code()
                .is_none_or(|code| row.diagnostics().contains(&code));
            return matches_filter && matches_query && matches_diagnostic;
        }

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
        let row_count = self
            .plan
            .as_ref()
            .map_or(SAMPLE_COUNT, |plan| plan.rows().len());
        (0..row_count)
            .filter(|index| self.row_is_visible(*index))
            .collect()
    }

    fn admit_sources(&mut self, paths: Vec<PathBuf>) {
        let request = self.rule_request();
        match self.application.admit_sources_with_rules(paths, request) {
            Ok(plan) => {
                self.status = self.plan_status(&plan);
                self.plan = Some(plan);
            }
            Err(error) => {
                self.status = format!(
                    "{} ({})",
                    self.locale
                        .text("Sources were not admitted", "원본을 추가하지 못했습니다"),
                    error.code()
                );
            }
        }
    }

    fn refresh_plan(&mut self) {
        if self.plan.is_none() {
            return;
        }
        match self.application.preview_rules(self.rule_request()) {
            Ok(plan) => {
                self.status = self.plan_status(&plan);
                self.plan = Some(plan);
            }
            Err(code) => {
                self.status = format!(
                    "{} ({code})",
                    self.locale
                        .text("Preview unavailable", "미리보기를 만들 수 없습니다")
                );
            }
        }
    }

    fn plan_status(&self, plan: &PlanDto) -> String {
        match self.locale {
            Locale::English => format!(
                "{} sources · {} changed · {} blocked",
                plan.rows().len(),
                plan.changed_count(),
                plan.blocked_count()
            ),
            Locale::Korean => format!(
                "원본 {}개 · 변경 {}개 · 차단 {}개",
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
    fn set_prefix(&mut self, prefix: &str) {
        if let Some(RuleRequestDto::Prefix { value, .. }) = self.rules.first_mut() {
            prefix.clone_into(value);
        }
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
        if let Some(path) = &self.preset_path
            && let Err(error) = next.save(path)
        {
            self.status = format!("Presets unavailable ({})", error.code());
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
            Err(error) => self.status = format!("Preset not saved ({})", error.code()),
        }
    }

    fn apply_preset(&mut self, preset_id: u64) {
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
        self.refresh_plan();
        self.status = format!(
            "{}: {name}",
            self.locale.text("Preset applied", "프리셋 적용")
        );
    }

    fn delete_preset(&mut self, preset_id: u64) {
        let mut next = self.presets.clone();
        next.remove(preset_id);
        self.save_presets(
            next,
            self.locale
                .text("Preset deleted", "프리셋을 삭제했습니다")
                .to_owned(),
        );
    }

    fn show_source_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new(semantics::PRODUCT_NAME).color(self.palette.ink));
            ui.label(
                RichText::new(
                    self.locale
                        .text(semantics::TAGLINE, "모든 이름 변경을 계획하세요."),
                )
                .color(self.palette.ink_soft),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                egui::ComboBox::from_id_salt("locale")
                    .selected_text(self.locale.label())
                    .show_ui(ui, |ui| {
                        for locale in Locale::ALL {
                            ui.selectable_value(&mut self.locale, locale, locale.label());
                        }
                    })
                    .response
                    .on_hover_text(semantics::LANGUAGE);
                ui.add_enabled(
                    false,
                    egui::Button::new(self.locale.text(semantics::ADD_FOLDER, "폴더 추가")),
                )
                .on_disabled_hover_text(self.locale.text(
                    "Directory admission is planned for Stage 6G",
                    "폴더 추가는 Stage 6G에서 지원할 예정입니다",
                ));
                if ui
                    .button(self.locale.text(semantics::ADD_FILES, "파일 추가"))
                    .clicked()
                {
                    match rfd::FileDialog::new()
                        .set_title("Add files to Renamewright")
                        .pick_files()
                    {
                        Some(paths) => self.admit_sources(paths),
                        None => {
                            self.status = self
                                .locale
                                .text("File selection cancelled", "파일 선택을 취소했습니다")
                                .to_owned();
                        }
                    }
                }
            });
        });
    }

    fn show_rule_rail(&mut self, ui: &mut egui::Ui) {
        ui.heading(
            RichText::new(self.locale.text(semantics::RULES_HEADING, "규칙"))
                .color(self.palette.ink),
        );
        ui.label(
            RichText::new(
                self.locale
                    .text(semantics::RULES_ORDER_HELP, "위에서 아래 순서로 적용"),
            )
            .color(self.palette.ink_soft),
        );
        ui.add_space(8.0);

        let mut move_rule = None;
        let mut remove_rule = None;
        ScrollArea::vertical()
            .id_salt("rule-list")
            .max_height(220.0)
            .show(ui, |ui| {
                for (index, rule) in self.rules.iter().enumerate() {
                    ui.push_id(rule.rule_id(), |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(
                                    self.selected_rule == index,
                                    format!(
                                        "{:02} {}",
                                        index + 1,
                                        rule_kind(rule).label(self.locale)
                                    ),
                                )
                                .clicked()
                            {
                                self.selected_rule = index;
                            }
                            if ui
                                .add_enabled(index > 0, egui::Button::new("↑"))
                                .on_hover_text(semantics::MOVE_RULE_UP)
                                .clicked()
                            {
                                move_rule = Some((index, index - 1));
                            }
                            if ui
                                .add_enabled(index + 1 < self.rules.len(), egui::Button::new("↓"))
                                .on_hover_text(semantics::MOVE_RULE_DOWN)
                                .clicked()
                            {
                                move_rule = Some((index, index + 1));
                            }
                            if ui
                                .button("×")
                                .on_hover_text(semantics::REMOVE_RULE)
                                .clicked()
                            {
                                remove_rule = Some(index);
                            }
                        });
                    });
                }
            });
        if let Some((from, to)) = move_rule {
            self.rules.swap(from, to);
            self.selected_rule = to;
            self.refresh_plan();
        }
        if let Some(index) = remove_rule {
            self.rules.remove(index);
            self.selected_rule = self.selected_rule.min(self.rules.len().saturating_sub(1));
            self.refresh_plan();
        }

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("new-rule-kind")
                .selected_text(self.new_rule_kind.label(self.locale))
                .show_ui(ui, |ui| {
                    for kind in RuleKind::ALL {
                        ui.selectable_value(&mut self.new_rule_kind, kind, kind.label(self.locale));
                    }
                });
            if ui
                .button(self.locale.text(semantics::ADD_RULE, "규칙 추가"))
                .clicked()
            {
                self.rules
                    .push(self.new_rule_kind.create(self.next_rule_id));
                self.next_rule_id = self.next_rule_id.saturating_add(1);
                self.selected_rule = self.rules.len().saturating_sub(1);
                self.refresh_plan();
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
        if let Some(rule) = self.rules.get_mut(self.selected_rule) {
            ui.label(
                RichText::new(rule_kind(rule).label(self.locale))
                    .strong()
                    .color(self.palette.ink),
            );
            let changed = ui
                .push_id(rule.rule_id(), |ui| rule_editor(ui, rule, self.locale))
                .inner;
            if changed {
                self.refresh_plan();
            }
        } else {
            ui.label(self.locale.text("No rules", "규칙 없음"));
        }
        ui.label(RichText::new(semantics::HANGUL_IME_HELP).color(self.palette.ink_soft));
        ui.add_space(8.0);
        ui.separator();
        ui.label(
            RichText::new(self.locale.text(semantics::PRESETS, "로컬 프리셋"))
                .strong()
                .color(self.palette.ink),
        );
        let preset_label = ui.label(self.locale.text(semantics::PRESET_NAME, "프리셋 이름"));
        ui.add(
            egui::TextEdit::singleline(&mut self.preset_name)
                .id_salt("preset-name")
                .hint_text(self.locale.text("Name", "이름")),
        )
        .labelled_by(preset_label.id);
        if ui
            .button(self.locale.text(semantics::SAVE_PRESET, "프리셋 저장"))
            .clicked()
        {
            self.save_current_preset();
        }
        let mut apply_preset = None;
        let mut delete_preset = None;
        ScrollArea::vertical()
            .id_salt("preset-list")
            .max_height(120.0)
            .show(ui, |ui| {
                for preset in self.presets.presets() {
                    ui.push_id(preset.preset_id(), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(preset.name());
                            if ui
                                .small_button(
                                    self.locale.text(semantics::APPLY_PRESET, "프리셋 적용"),
                                )
                                .clicked()
                            {
                                apply_preset = Some(preset.preset_id());
                            }
                            if ui
                                .small_button(
                                    self.locale.text(semantics::DELETE_PRESET, "프리셋 삭제"),
                                )
                                .clicked()
                            {
                                delete_preset = Some(preset.preset_id());
                            }
                        });
                    });
                }
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
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for candidate in PlanFilter::ALL {
                if ui
                    .selectable_label(self.filter == candidate, candidate.label(self.locale))
                    .clicked()
                {
                    self.filter = candidate;
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
                .on_hover_text(semantics::DIAGNOSTIC_FILTER);
            ui.separator();
            let source_query_label =
                ui.label(self.locale.text(semantics::SOURCE_QUERY_LABEL, "이름 필터"));
            ui.add(
                egui::TextEdit::singleline(&mut self.source_query)
                    .id_salt("preview.source-query")
                    .hint_text(self.locale.text("Name contains", "이름에 포함")),
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
                        [150.0, 20.0],
                        egui::Label::new(
                            RichText::new(self.locale.text("Source", "원본"))
                                .strong()
                                .color(self.palette.ink),
                        ),
                    );
                    ui.add_sized(
                        [180.0, 20.0],
                        egui::Label::new(
                            RichText::new(self.locale.text("Proposed", "변경안"))
                                .strong()
                                .color(self.palette.ink),
                        ),
                    );
                    ui.add_sized(
                        [80.0, 20.0],
                        egui::Label::new(
                            RichText::new(self.locale.text("Status", "상태"))
                                .strong()
                                .color(self.palette.ink),
                        ),
                    );
                    ui.label(
                        RichText::new(self.locale.text("Diagnostics", "진단"))
                            .strong()
                            .color(self.palette.ink),
                    );
                });
                ui.separator();
                let mut requested_override = None;
                ScrollArea::vertical()
                    .id_salt("preview.rows")
                    .auto_shrink([false, false])
                    .show_rows(ui, PREVIEW_ROW_HEIGHT, visible.len(), |ui, row_range| {
                        for visible_row in row_range {
                            let index = visible[visible_row];
                            let (source_id, source, proposed, status, diagnostics, blocked) =
                                self.plan.as_ref().map_or_else(
                                    || {
                                        let source = format!("IMG_{index:05}.jpg");
                                        let proposed =
                                            format!("{}{source}", self.synthetic_prefix());
                                        let blocked = Self::row_is_blocked(index);
                                        let status = if blocked {
                                            self.locale.text("Blocked", "차단됨")
                                        } else {
                                            self.locale.text("Changed", "변경됨")
                                        };
                                        (
                                            None,
                                            source,
                                            proposed,
                                            status,
                                            if blocked {
                                                self.locale.text("Sample conflict", "샘플 충돌")
                                            } else {
                                                ""
                                            }
                                            .to_owned(),
                                            blocked,
                                        )
                                    },
                                    |plan| {
                                        let row = &plan.rows()[index];
                                        (
                                            Some(row.source_id()),
                                            row.original_name().to_owned(),
                                            row.proposed_name().to_owned(),
                                            if row.status() == "blocked" {
                                                self.locale.text("Blocked", "차단됨")
                                            } else if row.status() == "unchanged" {
                                                self.locale.text("Unchanged", "변경 없음")
                                            } else {
                                                self.locale.text("Changed", "변경됨")
                                            },
                                            row.diagnostics()
                                                .iter()
                                                .map(|code| diagnostic_label(code, self.locale))
                                                .collect::<Vec<_>>()
                                                .join(", "),
                                            row.status() == "blocked",
                                        )
                                    },
                                );
                            ui.push_id(index, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_sized([150.0, 20.0], egui::Label::new(&source));
                                    ui.add_sized([180.0, 20.0], egui::Label::new(proposed));
                                    let color = if blocked {
                                        self.palette.blocked
                                    } else {
                                        self.palette.accent
                                    };
                                    ui.add_sized(
                                        [80.0, 20.0],
                                        egui::Label::new(
                                            RichText::new(status).color(color).strong(),
                                        ),
                                    );
                                    ui.label(diagnostics);
                                    if let Some(source_id) = source_id
                                        && ui
                                            .small_button(
                                                if self.overrides.contains_key(&source_id) {
                                                    self.locale.text("Edit override", "재정의 편집")
                                                } else {
                                                    self.locale.text("Override", "재정의")
                                                },
                                            )
                                            .clicked()
                                    {
                                        requested_override = Some((source_id, source));
                                    }
                                });
                            });
                        }
                    });
                if let Some((source_id, original_name)) = requested_override {
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
            });
    }

    fn show_review_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.status).color(self.palette.ink_soft));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let apply_text = if self.palette.high_contrast {
                    RichText::new(self.locale.text(semantics::APPLY, "적용"))
                        .color(self.palette.disabled)
                } else {
                    RichText::new(self.locale.text(semantics::APPLY, "적용"))
                };
                ui.add_enabled(false, egui::Button::new(apply_text))
                    .on_disabled_hover_text(self.locale.text(
                        "The native read-only workbench never mutates the filesystem",
                        "네이티브 읽기 전용 작업대는 파일 시스템을 변경하지 않습니다",
                    ));
                ui.label(
                    RichText::new(self.locale.text(semantics::APPLY_LOCKED, "적용 잠김"))
                        .color(self.palette.blocked)
                        .strong(),
                );
                if ui
                    .add_enabled(
                        self.plan.is_some(),
                        egui::Button::new(self.locale.text(semantics::EXPORT_CSV, "CSV 내보내기")),
                    )
                    .clicked()
                {
                    self.export_plan(false);
                }
                if ui
                    .add_enabled(
                        self.plan.is_some(),
                        egui::Button::new(
                            self.locale.text(semantics::EXPORT_JSON, "JSON 내보내기"),
                        ),
                    )
                    .clicked()
                {
                    self.export_plan(true);
                }
                if ui
                    .add_enabled(
                        self.plan.is_some(),
                        egui::Button::new(self.locale.text(semantics::INSPECT_CSV, "CSV 검토")),
                    )
                    .clicked()
                {
                    self.inspect_plan(false);
                }
                if ui
                    .add_enabled(
                        self.plan.is_some(),
                        egui::Button::new(self.locale.text(semantics::INSPECT_JSON, "JSON 검토")),
                    )
                    .clicked()
                {
                    self.inspect_plan(true);
                }
            });
        });
    }

    fn inspect_plan(&mut self, json: bool) {
        let Some(plan_id) = self.plan.as_ref().map(PlanDto::plan_id) else {
            return;
        };
        let document = if json {
            self.application.inspect_plan_json(plan_id)
        } else {
            self.application.inspect_plan_csv(plan_id)
        };
        match document {
            Ok(content) => {
                self.inspection = Some(InspectionDocument {
                    title: if json {
                        self.locale.text("Plan JSON", "계획 JSON")
                    } else {
                        self.locale.text("Plan CSV", "계획 CSV")
                    },
                    content,
                });
            }
            Err(error) => {
                self.status = format!(
                    "{} ({error})",
                    self.locale
                        .text("Inspection unavailable", "계획을 검토할 수 없습니다")
                );
            }
        }
    }

    fn export_plan(&mut self, json: bool) {
        let Some(plan_id) = self.plan.as_ref().map(PlanDto::plan_id) else {
            return;
        };
        let document = if json {
            self.application.inspect_plan_json(plan_id)
        } else {
            self.application.inspect_plan_csv(plan_id)
        };
        let Ok(document) = document else {
            self.status = self
                .locale
                .text(
                    "The current plan could not be inspected",
                    "현재 계획을 검토할 수 없습니다",
                )
                .to_owned();
            return;
        };
        let extension = if json { "json" } else { "csv" };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export the path-free Renamewright plan")
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
        match ApplicationService::export_document(&path, &document) {
            Ok(()) => {
                self.status = match self.locale {
                    Locale::English => format!("Plan {extension} exported"),
                    Locale::Korean => format!("계획 {extension}을 내보냈습니다"),
                };
            }
            Err(error) => self.status = error,
        }
    }

    fn show_transient_windows(&mut self, context: &egui::Context) {
        let mut save_override = false;
        let mut remove_override = false;
        let mut close_override = false;
        if let Some(editor) = self.override_editor.as_mut() {
            egui::Window::new(self.locale.text("Filename override", "파일 이름 재정의"))
                .collapsible(false)
                .resizable(false)
                .show(context, |ui| {
                    ui.label(&editor.original_name);
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.value)
                            .hint_text(self.locale.text("Proposed filename", "변경할 파일 이름")),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .button(self.locale.text(semantics::SAVE_OVERRIDE, "재정의 저장"))
                            .clicked()
                        {
                            save_override = true;
                        }
                        if ui
                            .button(self.locale.text("Reset override", "재정의 초기화"))
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
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let dropped_paths = ui.ctx().input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if !dropped_paths.is_empty() {
            self.admit_sources(dropped_paths);
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

        self.show_transient_windows(ui.ctx());
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
    use std::error::Error;
    use std::fs;
    #[cfg(feature = "automation")]
    use std::path::Path;

    use eframe::egui;
    use egui_kittest::Harness;
    use kittest::{NodeT as _, Queryable as _};

    #[cfg(feature = "automation")]
    use super::automation::{
        AutomationRoot, AutomationRootErrorKind, MAX_AUTOMATION_FIXTURE_BYTES,
    };
    use super::{
        Locale, NativePalette, NativeSpikeApp, RuleKind, RuleRequestDto, install_theme, semantics,
    };

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
        assert!(add_folder.accesskit_node().is_disabled());
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

    #[test]
    fn admitted_sources_render_the_application_service_plan() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"report")?;
        let mut app = NativeSpikeApp::new(false);
        app.set_prefix("final-");
        app.admit_sources(vec![source]);

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
    fn every_native_rule_family_builds_a_valid_ordered_service_request()
    -> Result<(), Box<dyn Error>> {
        let mut app = NativeSpikeApp::new(false);
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
        let mut app = NativeSpikeApp::new(false);
        app.set_prefix("");
        app.admit_sources(vec![source]);
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
        let mut app = NativeSpikeApp::new(false);
        app.locale = Locale::Korean;
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), app);

        harness.get_by_label("파일 추가");
        harness.get_by_label("폴더 추가");
        harness.get_by_label("규칙");
        harness.get_by_label("접두사 추가");
        harness.get_by_label("접두사 텍스트");
        harness.get_by_label("미리보기");
        harness.get_by_label("전체");
        harness.get_by_label("적용 잠김");
    }

    #[test]
    fn keyboard_input_edits_and_traverses_the_selected_rule() -> Result<(), Box<dyn Error>> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(|ui, app| app.show(ui), NativeSpikeApp::new(false));

        let prefix = harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, semantics::PREFIX_LABEL);
        prefix.click();
        harness.run_ok();
        assert!(harness.get_by_label(semantics::PREFIX_LABEL).is_focused());
        harness.event(egui::Event::Text("한글".to_owned()));
        harness.run_ok();
        harness.key_press(egui::Key::Tab);
        harness.run_ok();

        let Some(RuleRequestDto::Prefix { value, .. }) = harness.state().rules.first() else {
            return Err("the selected prefix rule was missing".into());
        };
        assert!(value.contains("한글"));
        assert!(harness.get_by_label(semantics::PRESET_NAME).is_focused());
        Ok(())
    }

    #[test]
    fn native_presets_persist_and_restore_ordered_rules() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let preset_path = directory.path().join("presets.json");
        let mut app = NativeSpikeApp::new_with_storage(
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
            NativeSpikeApp::new_with_storage(false, NativePalette::default(), Some(preset_path));
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
    fn automation_root_is_exclusive_and_prepares_isolated_state() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = AutomationRoot::open(directory.path())?;

        assert_eq!(root.root(), fs::canonicalize(directory.path())?);
        assert!(root.state_root().is_dir());
        assert!(root.journal_root().is_dir());
        let Err(error) = AutomationRoot::open(directory.path()) else {
            return Err("a concurrent automation session acquired the same root".into());
        };
        assert_eq!(error.kind(), AutomationRootErrorKind::ConcurrentSession);
        Ok(())
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_fixture_reads_are_relative_and_bounded() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let fixture_directory = directory.path().join("fixtures").join("nested");
        fs::create_dir_all(&fixture_directory)?;
        let fixture_json = br#"{"schemaVersion":1,"prefix":"fixture_","filter":"blocked"}"#;
        fs::write(fixture_directory.join("fixture.json"), fixture_json)?;
        let oversized = fixture_directory.join("oversized.json");
        fs::File::create(&oversized)?.set_len(MAX_AUTOMATION_FIXTURE_BYTES + 1)?;
        let root = AutomationRoot::open(directory.path())?;

        assert_eq!(
            root.read_fixture(Path::new("nested/fixture.json"))?,
            fixture_json
        );
        let fixture = root.load_fixture(Path::new("nested/fixture.json"))?;
        assert_eq!(fixture.prefix(), Some("fixture_"));
        assert_eq!(
            fixture.filter(),
            Some(super::automation::AutomationFilter::Blocked)
        );
        for rejected in [Path::new("../fixture.json"), directory.path()] {
            let Err(error) = root.read_fixture(rejected) else {
                return Err("an invalid automation fixture path was accepted".into());
            };
            assert_eq!(error.kind(), AutomationRootErrorKind::InvalidRelativePath);
        }
        let Err(error) = root.read_fixture(Path::new("nested/oversized.json")) else {
            return Err("an oversized automation fixture was accepted".into());
        };
        assert_eq!(error.kind(), AutomationRootErrorKind::FixtureTooLarge);
        Ok(())
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_fixture_initializes_a_deterministic_ui_state() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::create_dir(directory.path().join("fixtures"))?;
        fs::write(
            directory.path().join("fixtures/session.json"),
            br#"{"schemaVersion":1,"prefix":"fixture_","filter":"blocked"}"#,
        )?;
        let root = AutomationRoot::open(directory.path())?;
        let fixture = root.load_fixture(Path::new("session.json"))?;
        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                NativeSpikeApp::new_automated(NativePalette::default(), root, Some(&fixture)),
            );

        harness.get_by_label("Automation fixture loaded");
        harness.get_by_label("10 shown");
        harness.get_by_label("fixture_IMG_00997.jpg");
        Ok(())
    }

    #[cfg(feature = "automation")]
    #[test]
    fn automation_fixture_admits_only_confined_real_sources() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let fixtures = directory.path().join("fixtures");
        fs::create_dir(&fixtures)?;
        fs::write(fixtures.join("report.txt"), b"report")?;
        fs::write(
            fixtures.join("session.json"),
            br#"{"schemaVersion":1,"prefix":"final-","sources":["report.txt"]}"#,
        )?;
        let root = AutomationRoot::open(directory.path())?;
        let fixture = root.load_fixture(Path::new("session.json"))?;
        assert_eq!(fixture.sources().len(), 1);

        let harness = Harness::builder()
            .with_size(egui::vec2(1_100.0, 720.0))
            .build_ui_state(
                |ui, app| app.show(ui),
                NativeSpikeApp::new_automated(NativePalette::default(), root, Some(&fixture)),
            );
        harness.get_by_label("report.txt");
        harness.get_by_label("final-report.txt");
        harness.get_by_label("Automation fixture loaded · 1 sources");
        Ok(())
    }

    #[cfg(all(feature = "automation", unix))]
    #[test]
    fn automation_fixture_rejects_symlink_escape() -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        fs::write(outside.path().join("secret.json"), b"secret")?;
        fs::create_dir(directory.path().join("fixtures"))?;
        symlink(outside.path(), directory.path().join("fixtures/escape"))?;
        fs::write(
            directory.path().join("fixtures/session.json"),
            br#"{"schemaVersion":1,"sources":["escape/secret.json"]}"#,
        )?;
        let root = AutomationRoot::open(directory.path())?;

        let Err(error) = root.read_fixture(Path::new("escape/secret.json")) else {
            return Err("a symlink escaped the automation root".into());
        };
        assert_eq!(error.kind(), AutomationRootErrorKind::ReparsePointRejected);
        let Err(error) = root.load_fixture(Path::new("session.json")) else {
            return Err("a symlinked source escaped the automation root".into());
        };
        assert_eq!(error.kind(), AutomationRootErrorKind::ReparsePointRejected);
        Ok(())
    }
}
