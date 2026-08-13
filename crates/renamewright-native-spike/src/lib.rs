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
            AutomationFixture::parse(&self.read_fixture(relative)?)
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
        Self {
            prefix: "정리_".to_owned(),
            source_query: String::new(),
            filter: PlanFilter::All,
            selected_rule: 0,
            status: format!("{SAMPLE_COUNT} sample entries ready"),
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
        let mut app = Self::new_with_palette(true, palette);
        if let Some(fixture) = fixture {
            if let Some(prefix) = fixture.prefix() {
                app.prefix = prefix.to_owned();
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
            app.status = "Automation fixture loaded".to_owned();
        }
        app._automation_root = Some(automation_root);
        app
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
    #[cfg(feature = "automation")]
    use std::error::Error;
    #[cfg(feature = "automation")]
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

    #[cfg(all(feature = "automation", unix))]
    #[test]
    fn automation_fixture_rejects_symlink_escape() -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        fs::write(outside.path().join("secret.json"), b"secret")?;
        fs::create_dir(directory.path().join("fixtures"))?;
        symlink(outside.path(), directory.path().join("fixtures/escape"))?;
        let root = AutomationRoot::open(directory.path())?;

        let Err(error) = root.read_fixture(Path::new("escape/secret.json")) else {
            return Err("a symlink escaped the automation root".into());
        };
        assert_eq!(error.kind(), AutomationRootErrorKind::ReparsePointRejected);
        Ok(())
    }
}
