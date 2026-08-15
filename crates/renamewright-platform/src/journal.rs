use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Display, Formatter};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use renamewright_core::{
    EntryIdentitySignal, EntryKind, ExecutionIdentity, JournalEntry, JournalNameGraph,
    JournalRecord, JournalReplayErrorKind, JournalStatus, ParentId, PlanId, RollbackCause,
    SourceFingerprint, SourceId, replay_journal,
};

pub const JOURNAL_SCHEMA_VERSION: u16 = 4;
pub const MIN_SUPPORTED_JOURNAL_SCHEMA_VERSION: u16 = 1;
const MIN_RESUMABLE_JOURNAL_SCHEMA_VERSION: u16 = 2;
pub const MAX_JOURNAL_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_JOURNAL_FILE_BYTES: u64 = 64 * 1024 * 1024;

const MAGIC: [u8; 4] = *b"RWJR";
const FRAME_HEADER_BYTES: usize = 24;
const FLAG_NONE: u8 = 0;
#[cfg(unix)]
const NATIVE_ENCODING_UNIX_BYTES: u8 = 1;
#[cfg(windows)]
const NATIVE_ENCODING_WINDOWS_WIDE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalCodecErrorKind {
    TruncatedHeader,
    TruncatedPayload,
    InvalidMagic,
    UnsupportedVersion { version: u16 },
    MixedVersion { expected: u16, actual: u16 },
    InvalidFlags { flags: u8 },
    FrameTooLarge { payload_length: u32 },
    ChecksumMismatch,
    UnknownRecordKind { kind: u8 },
    InvalidPayload,
    InvalidNativeNameEncoding,
    IntegerOutOfRange,
    SequenceMismatch { expected: u64, actual: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalCodecError {
    frame_index: usize,
    kind: JournalCodecErrorKind,
}

impl JournalCodecError {
    const fn new(frame_index: usize, kind: JournalCodecErrorKind) -> Self {
        Self { frame_index, kind }
    }

    #[must_use]
    pub const fn frame_index(self) -> usize {
        self.frame_index
    }

    #[must_use]
    pub const fn kind(self) -> JournalCodecErrorKind {
        self.kind
    }
}

impl Display for JournalCodecError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "journal frame {} is not valid ({:?})",
            self.frame_index, self.kind
        )
    }
}

impl Error for JournalCodecError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalFrame {
    schema_version: u16,
    sequence: u64,
    record: JournalRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalInspection {
    frames: Vec<JournalFrame>,
    issue: Option<JournalCodecError>,
}

impl JournalInspection {
    #[must_use]
    pub fn frames(&self) -> &[JournalFrame] {
        &self.frames
    }

    #[must_use]
    pub const fn issue(&self) -> Option<JournalCodecError> {
        self.issue
    }

    #[must_use]
    pub fn is_torn_tail(&self) -> bool {
        matches!(
            self.issue.map(JournalCodecError::kind),
            Some(JournalCodecErrorKind::TruncatedHeader | JournalCodecErrorKind::TruncatedPayload)
        )
    }

    #[must_use]
    pub fn into_frames(self) -> Vec<JournalFrame> {
        self.frames
    }
}

impl JournalFrame {
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn record(&self) -> &JournalRecord {
        &self.record
    }

    #[must_use]
    pub fn into_record(self) -> JournalRecord {
        self.record
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalStorageErrorKind {
    InvalidInitialRecord,
    AlreadyExists,
    OpenFailed { io_kind: io::ErrorKind },
    LockFailed { io_kind: io::ErrorKind },
    ResumeReadFailed { io_kind: io::ErrorKind },
    ResumeTooLarge,
    ResumeCodec { kind: JournalCodecErrorKind },
    ResumeProtocol { kind: JournalReplayErrorKind },
    ResumeVersion { version: u16 },
    ResumeTerminal,
    HeaderAfterStart,
    RecordAfterTerminal,
    Codec { kind: JournalCodecErrorKind },
    WriteFailed { io_kind: io::ErrorKind },
    SyncFailed { io_kind: io::ErrorKind },
    WriterPoisoned,
    SequenceExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalStorageError {
    sequence: u64,
    kind: JournalStorageErrorKind,
}

impl JournalStorageError {
    const fn new(sequence: u64, kind: JournalStorageErrorKind) -> Self {
        Self { sequence, kind }
    }

    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn kind(self) -> JournalStorageErrorKind {
        self.kind
    }
}

impl Display for JournalStorageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "journal storage operation at sequence {} failed ({:?})",
            self.sequence, self.kind
        )
    }
}

impl Error for JournalStorageError {}

#[derive(Debug)]
pub struct JournalWriter {
    appender: DurableAppender<File>,
}

impl JournalWriter {
    pub fn create_new(
        path: &Path,
        initial_record: &JournalRecord,
    ) -> Result<Self, JournalStorageError> {
        if !matches!(initial_record, JournalRecord::TransactionStarted { .. }) {
            return Err(JournalStorageError::new(
                0,
                JournalStorageErrorKind::InvalidInitialRecord,
            ));
        }

        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create_new(true)
            .open(path)
            .map_err(|error| {
                let kind = if error.kind() == io::ErrorKind::AlreadyExists {
                    JournalStorageErrorKind::AlreadyExists
                } else {
                    JournalStorageErrorKind::OpenFailed {
                        io_kind: error.kind(),
                    }
                };
                JournalStorageError::new(0, kind)
            })?;
        file.try_lock().map_err(|error| {
            JournalStorageError::new(
                0,
                JournalStorageErrorKind::LockFailed {
                    io_kind: io::Error::from(error).kind(),
                },
            )
        })?;
        let mut appender = DurableAppender::new(file);
        appender.append_initial(initial_record)?;
        Ok(Self { appender })
    }

    pub fn resume(path: &Path) -> Result<(Self, Vec<JournalRecord>), JournalStorageError> {
        let mut file = open_existing_journal_no_follow(path).map_err(|error| {
            JournalStorageError::new(
                0,
                JournalStorageErrorKind::OpenFailed {
                    io_kind: error.kind(),
                },
            )
        })?;
        file.try_lock().map_err(|error| {
            JournalStorageError::new(
                0,
                JournalStorageErrorKind::LockFailed {
                    io_kind: io::Error::from(error).kind(),
                },
            )
        })?;
        let metadata = file.metadata().map_err(|error| {
            JournalStorageError::new(
                0,
                JournalStorageErrorKind::ResumeReadFailed {
                    io_kind: error.kind(),
                },
            )
        })?;
        if !metadata.is_file() || metadata.len() > MAX_JOURNAL_FILE_BYTES {
            return Err(JournalStorageError::new(
                0,
                JournalStorageErrorKind::ResumeTooLarge,
            ));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_JOURNAL_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| {
                JournalStorageError::new(
                    0,
                    JournalStorageErrorKind::ResumeReadFailed {
                        io_kind: error.kind(),
                    },
                )
            })?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_JOURNAL_FILE_BYTES {
            return Err(JournalStorageError::new(
                0,
                JournalStorageErrorKind::ResumeTooLarge,
            ));
        }
        let frames = decode_journal(&bytes).map_err(|error| {
            JournalStorageError::new(
                u64::try_from(error.frame_index()).unwrap_or(u64::MAX),
                JournalStorageErrorKind::ResumeCodec { kind: error.kind() },
            )
        })?;
        let Some(first) = frames.first() else {
            return Err(JournalStorageError::new(
                0,
                JournalStorageErrorKind::ResumeProtocol {
                    kind: JournalReplayErrorKind::EmptyJournal,
                },
            ));
        };
        if !(MIN_RESUMABLE_JOURNAL_SCHEMA_VERSION..=JOURNAL_SCHEMA_VERSION)
            .contains(&first.schema_version())
        {
            return Err(JournalStorageError::new(
                0,
                JournalStorageErrorKind::ResumeVersion {
                    version: first.schema_version(),
                },
            ));
        }
        let schema_version = first.schema_version();
        let records = frames
            .into_iter()
            .map(JournalFrame::into_record)
            .collect::<Vec<_>>();
        let status = replay_journal(&records).map_err(|error| {
            JournalStorageError::new(
                u64::try_from(error.record_index()).unwrap_or(u64::MAX),
                JournalStorageErrorKind::ResumeProtocol { kind: error.kind() },
            )
        })?;
        if matches!(
            status,
            JournalStatus::Completed | JournalStatus::RolledBack { .. }
        ) {
            return Err(JournalStorageError::new(
                u64::try_from(records.len()).unwrap_or(u64::MAX),
                JournalStorageErrorKind::ResumeTerminal,
            ));
        }
        let next_sequence = u64::try_from(records.len()).map_err(|_| {
            JournalStorageError::new(u64::MAX, JournalStorageErrorKind::SequenceExhausted)
        })?;
        Ok((
            Self {
                appender: DurableAppender::resume(file, next_sequence, schema_version),
            },
            records,
        ))
    }

    pub fn append(&mut self, record: &JournalRecord) -> Result<(), JournalStorageError> {
        self.appender.append(record)
    }

    pub(crate) fn append_buffered_completion(
        &mut self,
        record: &JournalRecord,
    ) -> Result<(), JournalStorageError> {
        self.appender.append_buffered_completion(record)
    }

    #[must_use]
    pub const fn next_sequence(&self) -> u64 {
        self.appender.next_sequence
    }

    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.appender.terminal
    }
}

trait JournalSink {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()>;
    fn sync_all(&self) -> io::Result<()>;
}

impl JournalSink for File {
    fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.write_all(frame)
    }

    fn sync_all(&self) -> io::Result<()> {
        File::sync_all(self)
    }
}

#[derive(Debug)]
struct DurableAppender<S> {
    sink: S,
    next_sequence: u64,
    terminal: bool,
    poisoned: bool,
    schema_version: u16,
}

impl<S: JournalSink> DurableAppender<S> {
    const fn new(sink: S) -> Self {
        Self {
            sink,
            next_sequence: 0,
            terminal: false,
            poisoned: false,
            schema_version: JOURNAL_SCHEMA_VERSION,
        }
    }

    const fn resume(sink: S, next_sequence: u64, schema_version: u16) -> Self {
        Self {
            sink,
            next_sequence,
            terminal: false,
            poisoned: false,
            schema_version,
        }
    }

    fn append_initial(&mut self, record: &JournalRecord) -> Result<(), JournalStorageError> {
        if !matches!(record, JournalRecord::TransactionStarted { .. }) {
            return Err(JournalStorageError::new(
                self.next_sequence,
                JournalStorageErrorKind::InvalidInitialRecord,
            ));
        }
        self.append_durable(record)
    }

    fn append(&mut self, record: &JournalRecord) -> Result<(), JournalStorageError> {
        self.validate_append(record)?;
        self.append_record(record, true)
    }

    fn append_buffered_completion(
        &mut self,
        record: &JournalRecord,
    ) -> Result<(), JournalStorageError> {
        if !matches!(
            record,
            JournalRecord::ForwardStepCompleted { .. }
                | JournalRecord::RollbackStepCompleted { .. }
        ) {
            return self.append(record);
        }
        self.validate_append(record)?;
        self.append_record(record, false)
    }

    fn validate_append(&self, record: &JournalRecord) -> Result<(), JournalStorageError> {
        if self.poisoned {
            return Err(JournalStorageError::new(
                self.next_sequence,
                JournalStorageErrorKind::WriterPoisoned,
            ));
        }
        if matches!(record, JournalRecord::TransactionStarted { .. }) {
            return Err(JournalStorageError::new(
                self.next_sequence,
                JournalStorageErrorKind::HeaderAfterStart,
            ));
        }
        if self.terminal {
            return Err(JournalStorageError::new(
                self.next_sequence,
                JournalStorageErrorKind::RecordAfterTerminal,
            ));
        }
        Ok(())
    }

    fn append_durable(&mut self, record: &JournalRecord) -> Result<(), JournalStorageError> {
        self.append_record(record, true)
    }

    fn append_record(
        &mut self,
        record: &JournalRecord,
        synchronize: bool,
    ) -> Result<(), JournalStorageError> {
        let following_sequence = self.next_sequence.checked_add(1).ok_or_else(|| {
            JournalStorageError::new(
                self.next_sequence,
                JournalStorageErrorKind::SequenceExhausted,
            )
        })?;
        let frame_index = usize::try_from(self.next_sequence).map_err(|_| {
            JournalStorageError::new(
                self.next_sequence,
                JournalStorageErrorKind::SequenceExhausted,
            )
        })?;
        let frame =
            encode_frame_for_version(self.next_sequence, record, frame_index, self.schema_version)
                .map_err(|error| {
                    JournalStorageError::new(
                        self.next_sequence,
                        JournalStorageErrorKind::Codec { kind: error.kind() },
                    )
                })?;

        if let Err(error) = self.sink.write_frame(&frame) {
            self.poisoned = true;
            return Err(JournalStorageError::new(
                self.next_sequence,
                JournalStorageErrorKind::WriteFailed {
                    io_kind: error.kind(),
                },
            ));
        }
        if synchronize && let Err(error) = self.sink.sync_all() {
            self.poisoned = true;
            return Err(JournalStorageError::new(
                self.next_sequence,
                JournalStorageErrorKind::SyncFailed {
                    io_kind: error.kind(),
                },
            ));
        }

        self.next_sequence = following_sequence;
        self.terminal = matches!(
            record,
            JournalRecord::TransactionCompleted | JournalRecord::TransactionRolledBack
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn open_existing_journal_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let flags = i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    OpenOptions::new()
        .read(true)
        .append(true)
        .custom_flags(flags)
        .open(path)
}

#[cfg(windows)]
fn open_existing_journal_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .append(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn open_existing_journal_no_follow(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).append(true).open(path)
}

pub fn encode_journal(records: &[JournalRecord]) -> Result<Vec<u8>, JournalCodecError> {
    let mut encoded = Vec::new();
    for (frame_index, record) in records.iter().enumerate() {
        let sequence = u64::try_from(frame_index).map_err(|_| {
            JournalCodecError::new(frame_index, JournalCodecErrorKind::IntegerOutOfRange)
        })?;
        encoded.extend_from_slice(&encode_frame(sequence, record, frame_index)?);
    }
    Ok(encoded)
}

pub fn decode_journal(bytes: &[u8]) -> Result<Vec<JournalFrame>, JournalCodecError> {
    let mut frames: Vec<JournalFrame> = Vec::new();
    let mut offset = 0usize;

    while offset < bytes.len() {
        let frame_index = frames.len();
        let remaining = bytes.len().saturating_sub(offset);
        if remaining < FRAME_HEADER_BYTES {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::TruncatedHeader,
            ));
        }
        let header = &bytes[offset..offset + FRAME_HEADER_BYTES];
        if header[..4] != MAGIC {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::InvalidMagic,
            ));
        }
        let version = u16::from_le_bytes([header[4], header[5]]);
        if !(MIN_SUPPORTED_JOURNAL_SCHEMA_VERSION..=JOURNAL_SCHEMA_VERSION).contains(&version) {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::UnsupportedVersion { version },
            ));
        }
        if let Some(first) = frames.first()
            && first.schema_version != version
        {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::MixedVersion {
                    expected: first.schema_version,
                    actual: version,
                },
            ));
        }
        let record_kind = header[6];
        let flags = header[7];
        if flags != FLAG_NONE {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::InvalidFlags { flags },
            ));
        }
        let sequence = u64::from_le_bytes(header[8..16].try_into().map_err(|_| {
            JournalCodecError::new(frame_index, JournalCodecErrorKind::TruncatedHeader)
        })?);
        let expected = u64::try_from(frame_index).map_err(|_| {
            JournalCodecError::new(frame_index, JournalCodecErrorKind::IntegerOutOfRange)
        })?;
        if sequence != expected {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::SequenceMismatch {
                    expected,
                    actual: sequence,
                },
            ));
        }
        let payload_length = u32::from_le_bytes(header[16..20].try_into().map_err(|_| {
            JournalCodecError::new(frame_index, JournalCodecErrorKind::TruncatedHeader)
        })?);
        if payload_length as usize > MAX_JOURNAL_PAYLOAD_BYTES {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::FrameTooLarge { payload_length },
            ));
        }
        let frame_length = FRAME_HEADER_BYTES
            .checked_add(payload_length as usize)
            .ok_or_else(|| {
                JournalCodecError::new(frame_index, JournalCodecErrorKind::IntegerOutOfRange)
            })?;
        if remaining < frame_length {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::TruncatedPayload,
            ));
        }
        let payload_start = offset + FRAME_HEADER_BYTES;
        let payload_end = offset + frame_length;
        let payload = &bytes[payload_start..payload_end];
        let expected_checksum = u32::from_le_bytes(header[20..24].try_into().map_err(|_| {
            JournalCodecError::new(frame_index, JournalCodecErrorKind::TruncatedHeader)
        })?);
        let actual_checksum = crc32_parts(&[&header[4..20], payload]);
        if actual_checksum != expected_checksum {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::ChecksumMismatch,
            ));
        }
        let record = decode_record(record_kind, payload, frame_index, version)?;
        frames.push(JournalFrame {
            schema_version: version,
            sequence,
            record,
        });
        offset = payload_end;
    }

    Ok(frames)
}

/// Reads the longest valid journal prefix and retains a pathless description of
/// the first invalid or torn frame. This function never treats a damaged prefix
/// as permission to resume execution.
#[must_use]
pub fn inspect_journal(bytes: &[u8]) -> JournalInspection {
    match decode_journal(bytes) {
        Ok(frames) => JournalInspection {
            frames,
            issue: None,
        },
        Err(issue) => {
            let prefix_length = complete_prefix_length(bytes, issue.frame_index());
            let frames = prefix_length
                .and_then(|length| decode_journal(&bytes[..length]).ok())
                .unwrap_or_default();
            JournalInspection {
                frames,
                issue: Some(issue),
            }
        }
    }
}

fn complete_prefix_length(bytes: &[u8], frame_count: usize) -> Option<usize> {
    let mut offset = 0usize;
    for _ in 0..frame_count {
        let header_end = offset.checked_add(FRAME_HEADER_BYTES)?;
        let header = bytes.get(offset..header_end)?;
        let payload_length = u32::from_le_bytes(header.get(16..20)?.try_into().ok()?) as usize;
        if payload_length > MAX_JOURNAL_PAYLOAD_BYTES {
            return None;
        }
        offset = header_end.checked_add(payload_length)?;
        if offset > bytes.len() {
            return None;
        }
    }
    Some(offset)
}

pub(crate) fn encode_frame(
    sequence: u64,
    record: &JournalRecord,
    frame_index: usize,
) -> Result<Vec<u8>, JournalCodecError> {
    encode_frame_for_version(sequence, record, frame_index, JOURNAL_SCHEMA_VERSION)
}

fn encode_frame_for_version(
    sequence: u64,
    record: &JournalRecord,
    frame_index: usize,
    schema_version: u16,
) -> Result<Vec<u8>, JournalCodecError> {
    let mut payload = Vec::new();
    let record_kind = encode_record(record, &mut payload, frame_index, schema_version)?;
    let payload_length = u32::try_from(payload.len()).map_err(|_| {
        JournalCodecError::new(
            frame_index,
            JournalCodecErrorKind::FrameTooLarge {
                payload_length: u32::MAX,
            },
        )
    })?;
    if payload.len() > MAX_JOURNAL_PAYLOAD_BYTES {
        return Err(JournalCodecError::new(
            frame_index,
            JournalCodecErrorKind::FrameTooLarge { payload_length },
        ));
    }

    let capacity = FRAME_HEADER_BYTES
        .checked_add(payload.len())
        .ok_or_else(|| {
            JournalCodecError::new(frame_index, JournalCodecErrorKind::IntegerOutOfRange)
        })?;
    let mut frame = Vec::with_capacity(capacity);
    frame.extend_from_slice(&MAGIC);
    frame.extend_from_slice(&schema_version.to_le_bytes());
    frame.push(record_kind);
    frame.push(FLAG_NONE);
    frame.extend_from_slice(&sequence.to_le_bytes());
    frame.extend_from_slice(&payload_length.to_le_bytes());
    frame.extend_from_slice(&0_u32.to_le_bytes());
    frame.extend_from_slice(&payload);
    let checksum = crc32_parts(&[&frame[4..20], &frame[FRAME_HEADER_BYTES..]]);
    frame[20..24].copy_from_slice(&checksum.to_le_bytes());
    Ok(frame)
}

fn encode_record(
    record: &JournalRecord,
    payload: &mut Vec<u8>,
    frame_index: usize,
    schema_version: u16,
) -> Result<u8, JournalCodecError> {
    match record {
        JournalRecord::TransactionStarted {
            plan_id,
            source_generation,
            step_count,
            entries,
        } => {
            put_u64(payload, plan_id.value());
            put_u64(payload, *source_generation);
            put_usize(payload, *step_count, frame_index)?;
            put_u32_len(payload, entries.len(), frame_index)?;
            for entry in entries {
                encode_entry(payload, entry, frame_index, schema_version)?;
            }
            Ok(1)
        }
        JournalRecord::ForwardStepPrepared { step_index } => {
            put_usize(payload, *step_index, frame_index)?;
            Ok(2)
        }
        JournalRecord::ForwardStepCompleted {
            step_index,
            observed_identity,
        } => {
            put_usize(payload, *step_index, frame_index)?;
            encode_execution_identity(payload, *observed_identity);
            Ok(3)
        }
        JournalRecord::ForwardStepNotApplied { step_index } => {
            put_usize(payload, *step_index, frame_index)?;
            Ok(10)
        }
        JournalRecord::RollbackStarted { cause } => {
            encode_rollback_cause(payload, *cause, frame_index)?;
            Ok(4)
        }
        JournalRecord::RollbackStepPrepared { step_index } => {
            put_usize(payload, *step_index, frame_index)?;
            Ok(5)
        }
        JournalRecord::RollbackStepCompleted {
            step_index,
            observed_identity,
        } => {
            put_usize(payload, *step_index, frame_index)?;
            encode_execution_identity(payload, *observed_identity);
            Ok(6)
        }
        JournalRecord::RollbackStepNotApplied { step_index } => {
            put_usize(payload, *step_index, frame_index)?;
            Ok(11)
        }
        JournalRecord::RollbackStepFailed { step_index } => {
            put_usize(payload, *step_index, frame_index)?;
            Ok(7)
        }
        JournalRecord::RollbackRecoveryStarted { step_index } => {
            put_usize(payload, *step_index, frame_index)?;
            Ok(12)
        }
        JournalRecord::TransactionCompleted => Ok(8),
        JournalRecord::TransactionRolledBack => Ok(9),
    }
}

fn decode_record(
    kind: u8,
    payload: &[u8],
    frame_index: usize,
    schema_version: u16,
) -> Result<JournalRecord, JournalCodecError> {
    let mut cursor = PayloadCursor::new(payload, frame_index);
    let record = match kind {
        1 => {
            let plan_id = PlanId::new(cursor.read_u64()?);
            let source_generation = cursor.read_u64()?;
            let step_count = cursor.read_usize()?;
            let entry_count = cursor.read_u32()? as usize;
            let mut entries = Vec::with_capacity(entry_count.min(1024));
            for _ in 0..entry_count {
                entries.push(decode_entry(&mut cursor, schema_version)?);
            }
            JournalRecord::TransactionStarted {
                plan_id,
                source_generation,
                step_count,
                entries,
            }
        }
        2 => JournalRecord::ForwardStepPrepared {
            step_index: cursor.read_usize()?,
        },
        3 => JournalRecord::ForwardStepCompleted {
            step_index: cursor.read_usize()?,
            observed_identity: decode_execution_identity(&mut cursor)?,
        },
        10 => JournalRecord::ForwardStepNotApplied {
            step_index: cursor.read_usize()?,
        },
        4 => JournalRecord::RollbackStarted {
            cause: decode_rollback_cause(&mut cursor)?,
        },
        5 => JournalRecord::RollbackStepPrepared {
            step_index: cursor.read_usize()?,
        },
        6 => JournalRecord::RollbackStepCompleted {
            step_index: cursor.read_usize()?,
            observed_identity: decode_execution_identity(&mut cursor)?,
        },
        11 => JournalRecord::RollbackStepNotApplied {
            step_index: cursor.read_usize()?,
        },
        7 => JournalRecord::RollbackStepFailed {
            step_index: cursor.read_usize()?,
        },
        12 => JournalRecord::RollbackRecoveryStarted {
            step_index: cursor.read_usize()?,
        },
        8 => JournalRecord::TransactionCompleted,
        9 => JournalRecord::TransactionRolledBack,
        _ => {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::UnknownRecordKind { kind },
            ));
        }
    };
    cursor.finish()?;
    Ok(record)
}

fn encode_entry(
    payload: &mut Vec<u8>,
    entry: &JournalEntry,
    frame_index: usize,
    schema_version: u16,
) -> Result<(), JournalCodecError> {
    put_u64(payload, entry.source_id().value());
    put_u64(payload, entry.parent_id().value());
    encode_native_name(payload, entry.names().original_name(), frame_index)?;
    encode_native_name(payload, entry.names().temporary_name(), frame_index)?;
    encode_native_name(payload, entry.names().final_name(), frame_index)?;
    encode_fingerprint(payload, entry.admission_fingerprint());
    encode_execution_identity(payload, entry.execution_identity());
    if schema_version >= 2 {
        match entry.native_parent() {
            Some(parent) => {
                payload.push(1);
                encode_native_name(payload, parent.as_os_str(), frame_index)?;
            }
            None => payload.push(0),
        }
    }
    if schema_version >= 3 {
        match entry.undo_of_plan_id() {
            Some(plan_id) => {
                payload.push(1);
                put_u64(payload, plan_id.value());
            }
            None => payload.push(0),
        }
    } else if entry.undo_of_plan_id().is_some() {
        return Err(JournalCodecError::new(
            frame_index,
            JournalCodecErrorKind::InvalidPayload,
        ));
    }
    Ok(())
}

fn decode_entry(
    cursor: &mut PayloadCursor<'_>,
    schema_version: u16,
) -> Result<JournalEntry, JournalCodecError> {
    let source_id = SourceId::new(cursor.read_u64()?);
    let parent_id = ParentId::new(cursor.read_u64()?);
    let names = JournalNameGraph::new(
        decode_native_name(cursor)?,
        decode_native_name(cursor)?,
        decode_native_name(cursor)?,
    );
    let fingerprint = decode_fingerprint(cursor, schema_version)?;
    let execution_identity = decode_execution_identity(cursor)?;
    let native_parent = if schema_version >= 2 {
        match cursor.read_u8()? {
            0 => None,
            1 => Some(std::path::PathBuf::from(decode_native_name(cursor)?)),
            _ => return Err(cursor.error(JournalCodecErrorKind::InvalidPayload)),
        }
    } else {
        None
    };
    let undo_of_plan_id = if schema_version >= 3 {
        match cursor.read_u8()? {
            0 => None,
            1 => Some(PlanId::new(cursor.read_u64()?)),
            _ => return Err(cursor.error(JournalCodecErrorKind::InvalidPayload)),
        }
    } else {
        None
    };
    let entry = match native_parent {
        Some(parent) => JournalEntry::with_native_parent(
            source_id,
            parent_id,
            names,
            fingerprint,
            execution_identity,
            parent,
        ),
        None => JournalEntry::new(source_id, parent_id, names, fingerprint, execution_identity),
    };
    Ok(match undo_of_plan_id {
        Some(plan_id) => entry.into_undo_of(plan_id),
        None => entry,
    })
}

fn encode_fingerprint(payload: &mut Vec<u8>, fingerprint: &SourceFingerprint) {
    payload.push(match fingerprint.entry_kind() {
        EntryKind::File => 1,
        EntryKind::Symlink => 2,
        EntryKind::Directory => 3,
    });
    match fingerprint.entry_identity_signal() {
        Some(signal) => {
            payload.push(1);
            put_u64(payload, signal.primary());
            put_u64(payload, signal.secondary());
        }
        None => payload.push(0),
    }
    put_u64(payload, fingerprint.byte_len());
    match fingerprint.modified_nanos() {
        Some(modified_nanos) => {
            payload.push(1);
            payload.extend_from_slice(&modified_nanos.to_le_bytes());
        }
        None => payload.push(0),
    }
}

fn decode_fingerprint(
    cursor: &mut PayloadCursor<'_>,
    schema_version: u16,
) -> Result<SourceFingerprint, JournalCodecError> {
    let entry_kind = match cursor.read_u8()? {
        1 => EntryKind::File,
        2 => EntryKind::Symlink,
        3 if schema_version >= 4 => EntryKind::Directory,
        _ => return Err(cursor.error(JournalCodecErrorKind::InvalidPayload)),
    };
    let identity_signal = match cursor.read_u8()? {
        0 => None,
        1 => Some(EntryIdentitySignal::new(
            cursor.read_u64()?,
            cursor.read_u64()?,
        )),
        _ => return Err(cursor.error(JournalCodecErrorKind::InvalidPayload)),
    };
    let byte_len = cursor.read_u64()?;
    let modified_nanos = match cursor.read_u8()? {
        0 => None,
        1 => Some(cursor.read_u128()?),
        _ => return Err(cursor.error(JournalCodecErrorKind::InvalidPayload)),
    };
    Ok(SourceFingerprint::new(
        entry_kind,
        identity_signal,
        byte_len,
        modified_nanos,
    ))
}

fn encode_execution_identity(payload: &mut Vec<u8>, identity: ExecutionIdentity) {
    put_u64(payload, identity.volume_serial_number());
    payload.extend_from_slice(&identity.file_id());
}

fn decode_execution_identity(
    cursor: &mut PayloadCursor<'_>,
) -> Result<ExecutionIdentity, JournalCodecError> {
    let volume_serial_number = cursor.read_u64()?;
    let file_id = cursor.read_array::<16>()?;
    Ok(ExecutionIdentity::new(volume_serial_number, file_id))
}

fn encode_rollback_cause(
    payload: &mut Vec<u8>,
    cause: RollbackCause,
    frame_index: usize,
) -> Result<(), JournalCodecError> {
    match cause {
        RollbackCause::Cancelled => payload.push(1),
        RollbackCause::ForwardStepFailed { step_index } => {
            payload.push(2);
            put_usize(payload, step_index, frame_index)?;
        }
        RollbackCause::RecoveryRequested => payload.push(3),
    }
    Ok(())
}

fn decode_rollback_cause(
    cursor: &mut PayloadCursor<'_>,
) -> Result<RollbackCause, JournalCodecError> {
    match cursor.read_u8()? {
        1 => Ok(RollbackCause::Cancelled),
        2 => Ok(RollbackCause::ForwardStepFailed {
            step_index: cursor.read_usize()?,
        }),
        3 => Ok(RollbackCause::RecoveryRequested),
        _ => Err(cursor.error(JournalCodecErrorKind::InvalidPayload)),
    }
}

#[cfg(unix)]
fn encode_native_name(
    payload: &mut Vec<u8>,
    name: &OsStr,
    frame_index: usize,
) -> Result<(), JournalCodecError> {
    use std::os::unix::ffi::OsStrExt;

    payload.push(NATIVE_ENCODING_UNIX_BYTES);
    let bytes = name.as_bytes();
    put_u32_len(payload, bytes.len(), frame_index)?;
    payload.extend_from_slice(bytes);
    Ok(())
}

#[cfg(windows)]
fn encode_native_name(
    payload: &mut Vec<u8>,
    name: &OsStr,
    frame_index: usize,
) -> Result<(), JournalCodecError> {
    use std::os::windows::ffi::OsStrExt;

    let wide = name.encode_wide().collect::<Vec<_>>();
    let byte_length = wide.len().checked_mul(2).ok_or_else(|| {
        JournalCodecError::new(frame_index, JournalCodecErrorKind::IntegerOutOfRange)
    })?;
    payload.push(NATIVE_ENCODING_WINDOWS_WIDE);
    put_u32_len(payload, byte_length, frame_index)?;
    for unit in wide {
        payload.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn encode_native_name(
    _payload: &mut Vec<u8>,
    _name: &OsStr,
    frame_index: usize,
) -> Result<(), JournalCodecError> {
    Err(JournalCodecError::new(
        frame_index,
        JournalCodecErrorKind::InvalidNativeNameEncoding,
    ))
}

#[cfg(unix)]
fn decode_native_name(cursor: &mut PayloadCursor<'_>) -> Result<OsString, JournalCodecError> {
    use std::os::unix::ffi::OsStringExt;

    if cursor.read_u8()? != NATIVE_ENCODING_UNIX_BYTES {
        return Err(cursor.error(JournalCodecErrorKind::InvalidNativeNameEncoding));
    }
    let length = cursor.read_u32()? as usize;
    Ok(OsString::from_vec(cursor.take(length)?.to_vec()))
}

#[cfg(windows)]
fn decode_native_name(cursor: &mut PayloadCursor<'_>) -> Result<OsString, JournalCodecError> {
    use std::os::windows::ffi::OsStringExt;

    if cursor.read_u8()? != NATIVE_ENCODING_WINDOWS_WIDE {
        return Err(cursor.error(JournalCodecErrorKind::InvalidNativeNameEncoding));
    }
    let byte_length = cursor.read_u32()? as usize;
    if !byte_length.is_multiple_of(2) {
        return Err(cursor.error(JournalCodecErrorKind::InvalidNativeNameEncoding));
    }
    let bytes = cursor.take(byte_length)?;
    let wide = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    Ok(OsString::from_wide(&wide))
}

#[cfg(not(any(unix, windows)))]
fn decode_native_name(cursor: &mut PayloadCursor<'_>) -> Result<OsString, JournalCodecError> {
    Err(cursor.error(JournalCodecErrorKind::InvalidNativeNameEncoding))
}

fn put_u64(payload: &mut Vec<u8>, value: u64) {
    payload.extend_from_slice(&value.to_le_bytes());
}

fn put_usize(
    payload: &mut Vec<u8>,
    value: usize,
    frame_index: usize,
) -> Result<(), JournalCodecError> {
    let value = u64::try_from(value).map_err(|_| {
        JournalCodecError::new(frame_index, JournalCodecErrorKind::IntegerOutOfRange)
    })?;
    put_u64(payload, value);
    Ok(())
}

fn put_u32_len(
    payload: &mut Vec<u8>,
    value: usize,
    frame_index: usize,
) -> Result<(), JournalCodecError> {
    let value = u32::try_from(value).map_err(|_| {
        JournalCodecError::new(frame_index, JournalCodecErrorKind::IntegerOutOfRange)
    })?;
    payload.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn crc32_parts(parts: &[&[u8]]) -> u32 {
    let mut crc = u32::MAX;
    for part in parts {
        for byte in *part {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = 0_u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
    }
    !crc
}

struct PayloadCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
    frame_index: usize,
}

impl<'a> PayloadCursor<'a> {
    const fn new(bytes: &'a [u8], frame_index: usize) -> Self {
        Self {
            bytes,
            offset: 0,
            frame_index,
        }
    }

    const fn error(&self, kind: JournalCodecErrorKind) -> JournalCodecError {
        JournalCodecError::new(self.frame_index, kind)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], JournalCodecError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| self.error(JournalCodecErrorKind::IntegerOutOfRange))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| self.error(JournalCodecErrorKind::InvalidPayload))?;
        self.offset = end;
        Ok(bytes)
    }

    fn read_u8(&mut self) -> Result<u8, JournalCodecError> {
        Ok(self.take(1)?[0])
    }

    fn read_u32(&mut self) -> Result<u32, JournalCodecError> {
        Ok(u32::from_le_bytes(self.read_array()?))
    }

    fn read_u64(&mut self) -> Result<u64, JournalCodecError> {
        Ok(u64::from_le_bytes(self.read_array()?))
    }

    fn read_u128(&mut self) -> Result<u128, JournalCodecError> {
        Ok(u128::from_le_bytes(self.read_array()?))
    }

    fn read_usize(&mut self) -> Result<usize, JournalCodecError> {
        usize::try_from(self.read_u64()?)
            .map_err(|_| self.error(JournalCodecErrorKind::IntegerOutOfRange))
    }

    fn read_array<const LENGTH: usize>(&mut self) -> Result<[u8; LENGTH], JournalCodecError> {
        self.take(LENGTH)?
            .try_into()
            .map_err(|_| self.error(JournalCodecErrorKind::InvalidPayload))
    }

    fn finish(self) -> Result<(), JournalCodecError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(self.error(JournalCodecErrorKind::InvalidPayload))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::ffi::OsString;
    use std::fs;
    use std::io;

    use renamewright_core::{
        EntryKind, ExecutionIdentity, JournalEntry, JournalNameGraph, JournalRecord, ParentId,
        PlanId, SourceFingerprint, SourceId,
    };

    use super::{
        DurableAppender, JournalCodecErrorKind, JournalSink, JournalStorageErrorKind,
        JournalWriter, crc32_parts, decode_journal, encode_frame_for_version,
    };

    #[derive(Debug, Default)]
    struct TestSink {
        frames: Vec<Vec<u8>>,
        sync_attempts: Cell<usize>,
        frames_seen_at_sync: Cell<usize>,
        fail_write: bool,
        fail_sync: bool,
    }

    impl JournalSink for TestSink {
        fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
            if self.fail_write {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "injected write failure",
                ));
            }
            self.frames.push(frame.to_vec());
            Ok(())
        }

        fn sync_all(&self) -> io::Result<()> {
            self.sync_attempts
                .set(self.sync_attempts.get().saturating_add(1));
            self.frames_seen_at_sync.set(self.frames.len());
            if self.fail_sync {
                Err(io::Error::other("injected sync failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn crc32_matches_the_ieee_check_value() {
        assert_eq!(crc32_parts(&[b"123456789"]), 0xcbf4_3926);
    }

    #[test]
    fn schema_one_header_remains_readable_without_a_native_parent()
    -> Result<(), Box<dyn std::error::Error>> {
        let record = JournalRecord::TransactionStarted {
            plan_id: PlanId::new(1),
            source_generation: 2,
            step_count: 2,
            entries: vec![JournalEntry::new(
                SourceId::new(3),
                ParentId::new(4),
                JournalNameGraph::new(
                    OsString::from("original.txt"),
                    OsString::from("temporary.tmp"),
                    OsString::from("final.txt"),
                ),
                SourceFingerprint::new(EntryKind::File, None, 5, None),
                ExecutionIdentity::new(6, [7; 16]),
            )],
        };
        let bytes = encode_frame_for_version(0, &record, 0, 1)?;

        let frames = decode_journal(&bytes)?;
        let JournalRecord::TransactionStarted { entries, .. } = frames[0].record() else {
            return Err("schema-one header changed record kind".into());
        };

        assert_eq!(frames[0].schema_version(), 1);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].native_parent(), None);
        Ok(())
    }

    #[test]
    fn schema_two_header_remains_readable_without_undo_lineage()
    -> Result<(), Box<dyn std::error::Error>> {
        let record = transaction_started_with_native_parent(None);
        let bytes = encode_frame_for_version(0, &record, 0, 2)?;
        let frames = decode_journal(&bytes)?;
        let JournalRecord::TransactionStarted { entries, .. } = frames[0].record() else {
            return Err("schema-two header changed record kind".into());
        };

        assert_eq!(frames[0].schema_version(), 2);
        assert_eq!(
            entries[0].native_parent(),
            Some(std::path::Path::new("native-parent"))
        );
        assert_eq!(entries[0].undo_of_plan_id(), None);
        Ok(())
    }

    #[test]
    fn schema_two_encoding_rejects_undo_lineage() {
        let record = transaction_started_with_native_parent(Some(PlanId::new(9)));
        let error = encode_frame_for_version(0, &record, 0, 2).err();

        assert_eq!(
            error.map(|value| value.kind()),
            Some(JournalCodecErrorKind::InvalidPayload)
        );
    }

    #[test]
    fn resuming_schema_two_keeps_appended_frames_on_schema_two()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("schema-two.rwj");
        fs::write(
            &path,
            encode_frame_for_version(0, &transaction_started_with_native_parent(None), 0, 2)?,
        )?;

        let (mut writer, _) = JournalWriter::resume(&path)?;
        writer.append(&JournalRecord::ForwardStepPrepared { step_index: 0 })?;
        drop(writer);

        let frames = decode_journal(&fs::read(path)?)?;
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|frame| frame.schema_version() == 2));
        Ok(())
    }

    fn transaction_started_with_native_parent(undo_of: Option<PlanId>) -> JournalRecord {
        let entry = JournalEntry::with_native_parent(
            SourceId::new(3),
            ParentId::new(4),
            JournalNameGraph::new(
                OsString::from("original.txt"),
                OsString::from("temporary.tmp"),
                OsString::from("final.txt"),
            ),
            SourceFingerprint::new(EntryKind::File, None, 5, None),
            ExecutionIdentity::new(6, [7; 16]),
            std::path::PathBuf::from("native-parent"),
        );
        let entry = undo_of.map_or(entry.clone(), |plan_id| entry.into_undo_of(plan_id));
        JournalRecord::TransactionStarted {
            plan_id: PlanId::new(1),
            source_generation: 2,
            step_count: 2,
            entries: vec![entry],
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_windows_native_name_encoding_on_unix() {
        let mut cursor = super::PayloadCursor::new(&[2, 0, 0, 0, 0], 0);
        let error = super::decode_native_name(&mut cursor).err();

        assert!(matches!(
            error.map(|value| value.kind()),
            Some(super::JournalCodecErrorKind::InvalidNativeNameEncoding)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_unix_native_name_encoding_on_windows() {
        let mut cursor = super::PayloadCursor::new(&[1, 0, 0, 0, 0], 0);
        let error = super::decode_native_name(&mut cursor).err();

        assert!(matches!(
            error.map(|value| value.kind()),
            Some(super::JournalCodecErrorKind::InvalidNativeNameEncoding)
        ));
    }

    #[test]
    fn durable_append_writes_before_syncing() -> Result<(), Box<dyn std::error::Error>> {
        let mut appender = DurableAppender::new(TestSink::default());

        appender.append_initial(&JournalRecord::TransactionStarted {
            plan_id: renamewright_core::PlanId::new(1),
            source_generation: 1,
            step_count: 0,
            entries: Vec::new(),
        })?;

        assert_eq!(appender.sink.frames.len(), 1);
        assert_eq!(appender.sink.sync_attempts.get(), 1);
        assert_eq!(appender.sink.frames_seen_at_sync.get(), 1);
        assert_eq!(appender.next_sequence, 1);
        Ok(())
    }

    #[test]
    fn completed_step_is_flushed_with_the_next_durable_boundary()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut appender = DurableAppender::new(TestSink::default());
        appender.append_initial(&JournalRecord::TransactionStarted {
            plan_id: PlanId::new(1),
            source_generation: 1,
            step_count: 2,
            entries: vec![JournalEntry::new(
                SourceId::new(1),
                ParentId::new(1),
                JournalNameGraph::new(
                    OsString::from("a.txt"),
                    OsString::from("temporary.tmp"),
                    OsString::from("final-a.txt"),
                ),
                SourceFingerprint::new(EntryKind::File, None, 1, None),
                ExecutionIdentity::new(1, [1; 16]),
            )],
        })?;
        appender.append(&JournalRecord::ForwardStepPrepared { step_index: 0 })?;
        appender.append_buffered_completion(&JournalRecord::ForwardStepCompleted {
            step_index: 0,
            observed_identity: ExecutionIdentity::new(1, [1; 16]),
        })?;

        assert_eq!(appender.sink.frames.len(), 3);
        assert_eq!(appender.sink.sync_attempts.get(), 2);
        assert_eq!(appender.sink.frames_seen_at_sync.get(), 2);

        appender.append(&JournalRecord::ForwardStepPrepared { step_index: 1 })?;

        assert_eq!(appender.sink.frames.len(), 4);
        assert_eq!(appender.sink.sync_attempts.get(), 3);
        assert_eq!(appender.sink.frames_seen_at_sync.get(), 4);
        Ok(())
    }

    #[test]
    fn terminal_record_flushes_the_final_buffered_completion()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut appender = DurableAppender::new(TestSink::default());
        appender.append_initial(&JournalRecord::TransactionStarted {
            plan_id: PlanId::new(1),
            source_generation: 1,
            step_count: 0,
            entries: Vec::new(),
        })?;
        appender.append_buffered_completion(&JournalRecord::ForwardStepCompleted {
            step_index: 0,
            observed_identity: ExecutionIdentity::new(1, [1; 16]),
        })?;
        assert_eq!(appender.sink.sync_attempts.get(), 1);

        appender.append(&JournalRecord::TransactionCompleted)?;

        assert_eq!(appender.sink.sync_attempts.get(), 2);
        assert_eq!(appender.sink.frames_seen_at_sync.get(), 3);
        assert!(appender.terminal);
        Ok(())
    }

    #[test]
    fn successful_two_source_sequence_uses_one_sync_per_prepared_step()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut appender = DurableAppender::new(TestSink::default());
        appender.append_initial(&JournalRecord::TransactionStarted {
            plan_id: PlanId::new(1),
            source_generation: 1,
            step_count: 4,
            entries: Vec::new(),
        })?;
        for step_index in 0..4 {
            appender.append(&JournalRecord::ForwardStepPrepared { step_index })?;
            appender.append_buffered_completion(&JournalRecord::ForwardStepCompleted {
                step_index,
                observed_identity: ExecutionIdentity::new(1, [1; 16]),
            })?;
        }
        appender.append(&JournalRecord::TransactionCompleted)?;

        assert_eq!(appender.sink.frames.len(), 10);
        assert_eq!(appender.sink.sync_attempts.get(), 6);
        assert_eq!(appender.sink.frames_seen_at_sync.get(), 10);
        Ok(())
    }

    #[test]
    fn sync_failure_poisons_writer_without_advancing_sequence()
    -> Result<(), Box<dyn std::error::Error>> {
        let sink = TestSink {
            fail_sync: true,
            ..TestSink::default()
        };
        let mut appender = DurableAppender::new(sink);
        let initial = JournalRecord::TransactionStarted {
            plan_id: renamewright_core::PlanId::new(1),
            source_generation: 1,
            step_count: 0,
            entries: Vec::new(),
        };

        let sync_error = appender
            .append_initial(&initial)
            .err()
            .ok_or("sync failure was accepted")?;
        assert!(matches!(
            sync_error.kind(),
            JournalStorageErrorKind::SyncFailed { .. }
        ));
        assert_eq!(appender.next_sequence, 0);
        assert_eq!(appender.sink.frames.len(), 1);

        let poisoned_error = appender
            .append(&JournalRecord::ForwardStepPrepared { step_index: 0 })
            .err()
            .ok_or("poisoned writer accepted another frame")?;
        assert_eq!(
            poisoned_error.kind(),
            JournalStorageErrorKind::WriterPoisoned
        );
        assert_eq!(appender.sink.frames.len(), 1);
        Ok(())
    }

    #[test]
    fn write_failure_poisons_writer_without_attempting_sync()
    -> Result<(), Box<dyn std::error::Error>> {
        let sink = TestSink {
            fail_write: true,
            ..TestSink::default()
        };
        let mut appender = DurableAppender::new(sink);
        let initial = JournalRecord::TransactionStarted {
            plan_id: renamewright_core::PlanId::new(1),
            source_generation: 1,
            step_count: 0,
            entries: Vec::new(),
        };

        let error = appender
            .append_initial(&initial)
            .err()
            .ok_or("write failure was accepted")?;
        assert!(matches!(
            error.kind(),
            JournalStorageErrorKind::WriteFailed { .. }
        ));
        assert_eq!(appender.next_sequence, 0);
        assert_eq!(appender.sink.sync_attempts.get(), 0);
        Ok(())
    }
}
