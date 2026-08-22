use std::ffi::OsString;
use std::path::PathBuf;

use renamewright_core::{
    EntryIdentitySignal, EntryKind, ExecutionIdentity, JournalEntry, JournalNameGraph,
    JournalRecord, ParentId, PlanId, RollbackCause, SourceFingerprint, SourceId, replay_journal,
};
use renamewright_platform::{
    JOURNAL_SCHEMA_VERSION, JournalCodecErrorKind, MAX_JOURNAL_PAYLOAD_BYTES, decode_journal,
    encode_journal, inspect_journal,
};

const GOLDEN_V1_TRANSACTION_COMPLETED: [u8; 24] = [
    b'R', b'W', b'J', b'R', 1, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf2, 0xc4, 0x8a, 0x71,
];

fn identity(seed: u8) -> ExecutionIdentity {
    ExecutionIdentity::new(u64::from(seed), [seed; 16])
}

fn transaction_started(original_name: OsString) -> JournalRecord {
    JournalRecord::TransactionStarted {
        plan_id: PlanId::new(7),
        source_generation: 11,
        step_count: 2,
        entries: vec![JournalEntry::with_native_parent_identity(
            SourceId::new(13),
            ParentId::new(17),
            JournalNameGraph::new(
                original_name,
                OsString::from(".renamewright-13.tmp"),
                OsString::from("final.txt"),
            ),
            SourceFingerprint::new(
                EntryKind::File,
                Some(EntryIdentitySignal::new(19, 23)),
                29,
                Some(31),
            ),
            identity(37),
            identity(41),
            PathBuf::from("native-parent"),
        )],
    }
}

fn complete_records(original_name: OsString) -> Vec<JournalRecord> {
    vec![
        transaction_started(original_name),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        JournalRecord::ForwardStepCompleted {
            step_index: 0,
            observed_identity: identity(41),
        },
        JournalRecord::ForwardStepPrepared { step_index: 1 },
        JournalRecord::ForwardStepCompleted {
            step_index: 1,
            observed_identity: identity(43),
        },
        JournalRecord::TransactionCompleted,
    ]
}

#[test]
fn round_trips_records_and_preserves_replay_semantics() -> Result<(), Box<dyn std::error::Error>> {
    let records = complete_records(OsString::from("original.txt"));
    let bytes = encode_journal(&records)?;
    let frames = decode_journal(&bytes)?;
    let decoded = frames
        .iter()
        .map(|frame| frame.record().clone())
        .collect::<Vec<_>>();

    assert_eq!(decoded, records);
    assert!(matches!(
        replay_journal(&decoded),
        Ok(renamewright_core::JournalStatus::Completed)
    ));
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.sequence())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5]
    );
    Ok(())
}

#[test]
fn current_schema_round_trips_batched_forward_intent() -> Result<(), Box<dyn std::error::Error>> {
    let record = JournalRecord::ForwardBatchPrepared {
        first_step: 7,
        step_count: 32,
    };

    let frames = decode_journal(&encode_journal(std::slice::from_ref(&record))?)?;

    assert_eq!(frames[0].schema_version(), JOURNAL_SCHEMA_VERSION);
    assert_eq!(frames[0].record(), &record);
    Ok(())
}

#[test]
fn round_trips_undo_lineage_in_the_current_schema() -> Result<(), Box<dyn std::error::Error>> {
    let undo_of = PlanId::new(41);
    let record = match transaction_started(OsString::from("original.txt")) {
        JournalRecord::TransactionStarted {
            plan_id,
            source_generation,
            step_count,
            entries,
        } => JournalRecord::TransactionStarted {
            plan_id,
            source_generation,
            step_count,
            entries: entries
                .into_iter()
                .map(|entry| entry.into_undo_of(undo_of))
                .collect(),
        },
        _ => return Err("transaction helper did not produce a header".into()),
    };

    let frames = decode_journal(&encode_journal(std::slice::from_ref(&record))?)?;
    assert_eq!(frames[0].schema_version(), JOURNAL_SCHEMA_VERSION);
    assert_eq!(frames[0].record(), &record);
    let JournalRecord::TransactionStarted { entries, .. } = frames[0].record() else {
        return Err("decoded undo journal did not contain a header".into());
    };
    assert_eq!(entries[0].undo_of_plan_id(), Some(undo_of));
    Ok(())
}

#[test]
fn current_schema_round_trips_directory_entry_kind() -> Result<(), Box<dyn std::error::Error>> {
    let record = JournalRecord::TransactionStarted {
        plan_id: PlanId::new(43),
        source_generation: 47,
        step_count: 2,
        entries: vec![JournalEntry::with_native_parent_identity(
            SourceId::new(53),
            ParentId::new(59),
            JournalNameGraph::new(
                OsString::from("folder"),
                OsString::from(".renamewright-folder.tmp"),
                OsString::from("renamed-folder"),
            ),
            SourceFingerprint::new(EntryKind::Directory, None, 0, None),
            identity(61),
            identity(67),
            PathBuf::from("native-parent"),
        )],
    };

    let frames = decode_journal(&encode_journal(std::slice::from_ref(&record))?)?;

    assert_eq!(frames[0].schema_version(), JOURNAL_SCHEMA_VERSION);
    assert_eq!(frames[0].record(), &record);
    Ok(())
}

#[test]
fn reads_and_reproduces_the_version_one_golden_frame() -> Result<(), Box<dyn std::error::Error>> {
    let frames = decode_journal(&GOLDEN_V1_TRANSACTION_COMPLETED)?;

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].schema_version(), 1);
    assert_eq!(frames[0].sequence(), 0);
    assert_eq!(frames[0].record(), &JournalRecord::TransactionCompleted);
    let current = encode_journal(&[JournalRecord::TransactionCompleted])?;
    assert_ne!(current, GOLDEN_V1_TRANSACTION_COMPLETED);
    assert_eq!(
        u16::from_le_bytes([current[4], current[5]]),
        JOURNAL_SCHEMA_VERSION
    );
    assert_eq!(
        decode_journal(&current)?[0].schema_version(),
        JOURNAL_SCHEMA_VERSION
    );
    Ok(())
}

#[test]
fn rejects_mixed_schema_versions() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = GOLDEN_V1_TRANSACTION_COMPLETED.to_vec();
    bytes.extend_from_slice(&encode_journal(&[JournalRecord::TransactionCompleted])?);

    let error = decode_journal(&bytes)
        .err()
        .ok_or("mixed journal versions were accepted")?;

    assert_eq!(
        error.kind(),
        JournalCodecErrorKind::MixedVersion {
            expected: 1,
            actual: JOURNAL_SCHEMA_VERSION,
        }
    );
    Ok(())
}

#[test]
fn round_trips_rollback_records() -> Result<(), Box<dyn std::error::Error>> {
    let records = vec![
        transaction_started(OsString::from("original.txt")),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        JournalRecord::ForwardStepCompleted {
            step_index: 0,
            observed_identity: identity(47),
        },
        JournalRecord::RollbackStarted {
            cause: RollbackCause::RecoveryRequested,
        },
        JournalRecord::RollbackStepPrepared { step_index: 0 },
        JournalRecord::RollbackStepFailed { step_index: 0 },
        JournalRecord::RollbackRecoveryStarted { step_index: 0 },
        JournalRecord::RollbackStepPrepared { step_index: 0 },
        JournalRecord::RollbackStepCompleted {
            step_index: 0,
            observed_identity: identity(53),
        },
        JournalRecord::TransactionRolledBack,
    ];

    let frames = decode_journal(&encode_journal(&records)?)?;
    assert_eq!(
        frames
            .into_iter()
            .map(|frame| frame.into_record())
            .collect::<Vec<_>>(),
        records
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn round_trips_non_utf8_native_names_without_loss() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::ffi::OsStringExt;

    let native_name = OsString::from_vec(vec![b'n', b'a', 0x80, b'm', b'e']);
    let records = vec![transaction_started(native_name)];
    let frames = decode_journal(&encode_journal(&records)?)?;

    assert_eq!(frames[0].record(), &records[0]);
    Ok(())
}

#[test]
fn rejects_corrupted_payload() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = encode_journal(&[JournalRecord::ForwardStepPrepared { step_index: 3 }])?;
    let Some(last) = bytes.last_mut() else {
        return Err("encoded frame was empty".into());
    };
    *last ^= 0xff;

    let error = decode_journal(&bytes)
        .err()
        .ok_or("corruption was accepted")?;
    assert_eq!(error.kind(), JournalCodecErrorKind::ChecksumMismatch);
    assert_eq!(error.frame_index(), 0);
    assert!(!error.to_string().contains('/'));
    Ok(())
}

#[test]
fn inspection_retains_only_the_valid_prefix_before_damage() -> Result<(), Box<dyn std::error::Error>>
{
    let records = complete_records(OsString::from("original.txt"));
    let mut bytes = encode_journal(&records)?;
    let first_frame_length = encode_journal(&records[..1])?.len();
    let corrupt_at = first_frame_length
        .checked_add(24)
        .ok_or("corruption offset overflowed")?;
    let Some(byte) = bytes.get_mut(corrupt_at) else {
        return Err("second frame had no payload".into());
    };
    *byte ^= 0xff;

    let inspection = inspect_journal(&bytes);

    assert_eq!(inspection.frames().len(), 1);
    assert!(!inspection.is_torn_tail());
    assert_eq!(
        inspection.issue().map(|issue| issue.kind()),
        Some(JournalCodecErrorKind::ChecksumMismatch)
    );
    Ok(())
}

#[test]
fn inspection_classifies_a_truncated_final_frame_as_a_torn_tail()
-> Result<(), Box<dyn std::error::Error>> {
    let records = complete_records(OsString::from("original.txt"));
    let mut bytes = encode_journal(&records)?;
    bytes.pop().ok_or("journal was empty")?;

    let inspection = inspect_journal(&bytes);

    assert_eq!(inspection.frames().len(), records.len() - 1);
    assert!(inspection.is_torn_tail());
    assert_eq!(
        inspection.issue().map(|issue| issue.kind()),
        Some(JournalCodecErrorKind::TruncatedHeader)
    );
    Ok(())
}

#[test]
fn rejects_every_truncated_prefix() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = encode_journal(&[transaction_started(OsString::from("original.txt"))])?;

    for length in 1..bytes.len() {
        let error = decode_journal(&bytes[..length])
            .err()
            .ok_or("truncated journal was accepted")?;
        assert!(matches!(
            error.kind(),
            JournalCodecErrorKind::TruncatedHeader | JournalCodecErrorKind::TruncatedPayload
        ));
    }
    Ok(())
}

#[test]
fn rejects_unknown_versions_before_payload_decoding() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = encode_journal(&[JournalRecord::TransactionCompleted])?;
    bytes[4..6].copy_from_slice(&JOURNAL_SCHEMA_VERSION.saturating_add(1).to_le_bytes());

    let error = decode_journal(&bytes)
        .err()
        .ok_or("unknown version was accepted")?;
    assert_eq!(
        error.kind(),
        JournalCodecErrorKind::UnsupportedVersion {
            version: JOURNAL_SCHEMA_VERSION.saturating_add(1)
        }
    );
    Ok(())
}

#[test]
fn rejects_oversized_frames_before_allocation() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = encode_journal(&[JournalRecord::TransactionCompleted])?;
    let oversized = u32::try_from(MAX_JOURNAL_PAYLOAD_BYTES)?.saturating_add(1);
    bytes[16..20].copy_from_slice(&oversized.to_le_bytes());

    let error = decode_journal(&bytes)
        .err()
        .ok_or("oversized frame was accepted")?;
    assert_eq!(
        error.kind(),
        JournalCodecErrorKind::FrameTooLarge {
            payload_length: oversized
        }
    );
    Ok(())
}

#[test]
fn rejects_non_contiguous_sequences() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = encode_journal(&[
        JournalRecord::ForwardStepPrepared { step_index: 0 },
        JournalRecord::ForwardStepPrepared { step_index: 1 },
    ])?;
    let first_length = u32::from_le_bytes(bytes[16..20].try_into()?) as usize;
    let second_offset = 24usize
        .checked_add(first_length)
        .ok_or("frame offset overflow")?;
    bytes[second_offset + 8..second_offset + 16].copy_from_slice(&7_u64.to_le_bytes());

    let error = decode_journal(&bytes)
        .err()
        .ok_or("sequence gap was accepted")?;
    assert_eq!(
        error.kind(),
        JournalCodecErrorKind::SequenceMismatch {
            expected: 1,
            actual: 7
        }
    );
    Ok(())
}
