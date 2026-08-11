use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Display, Formatter};

use renamewright_core::{
    EntryIdentitySignal, EntryKind, ExecutionIdentity, JournalEntry, JournalNameGraph,
    JournalRecord, ParentId, PlanId, RollbackCause, SourceFingerprint, SourceId,
};

pub const JOURNAL_SCHEMA_VERSION: u16 = 1;
pub const MAX_JOURNAL_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

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
    sequence: u64,
    record: JournalRecord,
}

impl JournalFrame {
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
    let mut frames = Vec::new();
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
        if version != JOURNAL_SCHEMA_VERSION {
            return Err(JournalCodecError::new(
                frame_index,
                JournalCodecErrorKind::UnsupportedVersion { version },
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
        let record = decode_record(record_kind, payload, frame_index)?;
        frames.push(JournalFrame { sequence, record });
        offset = payload_end;
    }

    Ok(frames)
}

pub(crate) fn encode_frame(
    sequence: u64,
    record: &JournalRecord,
    frame_index: usize,
) -> Result<Vec<u8>, JournalCodecError> {
    let mut payload = Vec::new();
    let record_kind = encode_record(record, &mut payload, frame_index)?;
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
    frame.extend_from_slice(&JOURNAL_SCHEMA_VERSION.to_le_bytes());
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
                encode_entry(payload, entry, frame_index)?;
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
        JournalRecord::RollbackStepFailed { step_index } => {
            put_usize(payload, *step_index, frame_index)?;
            Ok(7)
        }
        JournalRecord::TransactionCompleted => Ok(8),
        JournalRecord::TransactionRolledBack => Ok(9),
    }
}

fn decode_record(
    kind: u8,
    payload: &[u8],
    frame_index: usize,
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
                entries.push(decode_entry(&mut cursor)?);
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
        7 => JournalRecord::RollbackStepFailed {
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
) -> Result<(), JournalCodecError> {
    put_u64(payload, entry.source_id().value());
    put_u64(payload, entry.parent_id().value());
    encode_native_name(payload, entry.names().original_name(), frame_index)?;
    encode_native_name(payload, entry.names().temporary_name(), frame_index)?;
    encode_native_name(payload, entry.names().final_name(), frame_index)?;
    encode_fingerprint(payload, entry.admission_fingerprint());
    encode_execution_identity(payload, entry.execution_identity());
    Ok(())
}

fn decode_entry(cursor: &mut PayloadCursor<'_>) -> Result<JournalEntry, JournalCodecError> {
    let source_id = SourceId::new(cursor.read_u64()?);
    let parent_id = ParentId::new(cursor.read_u64()?);
    let names = JournalNameGraph::new(
        decode_native_name(cursor)?,
        decode_native_name(cursor)?,
        decode_native_name(cursor)?,
    );
    let fingerprint = decode_fingerprint(cursor)?;
    let execution_identity = decode_execution_identity(cursor)?;
    Ok(JournalEntry::new(
        source_id,
        parent_id,
        names,
        fingerprint,
        execution_identity,
    ))
}

fn encode_fingerprint(payload: &mut Vec<u8>, fingerprint: &SourceFingerprint) {
    payload.push(match fingerprint.entry_kind() {
        EntryKind::File => 1,
        EntryKind::Symlink => 2,
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
) -> Result<SourceFingerprint, JournalCodecError> {
    let entry_kind = match cursor.read_u8()? {
        1 => EntryKind::File,
        2 => EntryKind::Symlink,
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
    if byte_length % 2 != 0 {
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
    use super::crc32_parts;

    #[test]
    fn crc32_matches_the_ieee_check_value() {
        assert_eq!(crc32_parts(&[b"123456789"]), 0xcbf4_3926);
    }
}
