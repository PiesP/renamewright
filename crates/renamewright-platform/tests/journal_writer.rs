use std::ffi::OsString;
use std::fs;

use renamewright_core::{
    EntryKind, ExecutionIdentity, JournalEntry, JournalNameGraph, JournalRecord, JournalStatus,
    ParentId, PlanId, SourceFingerprint, SourceId, replay_journal,
};
use renamewright_platform::{JournalStorageErrorKind, JournalWriter, decode_journal};
use tempfile::tempdir;

fn transaction_started() -> JournalRecord {
    JournalRecord::TransactionStarted {
        plan_id: PlanId::new(1),
        source_generation: 2,
        step_count: 2,
        entries: vec![JournalEntry::new(
            SourceId::new(3),
            ParentId::new(4),
            JournalNameGraph::new(
                OsString::from("source.txt"),
                OsString::from(".renamewright-3.tmp"),
                OsString::from("target.txt"),
            ),
            SourceFingerprint::new(EntryKind::File, None, 5, None),
            ExecutionIdentity::new(6, [7; 16]),
        )],
    }
}

#[test]
fn create_new_and_append_produce_a_replayable_journal() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("transaction.rwj");
    let mut writer = JournalWriter::create_new(&path, &transaction_started())?;

    writer.append(&JournalRecord::ForwardStepPrepared { step_index: 0 })?;
    writer.append(&JournalRecord::ForwardStepCompleted {
        step_index: 0,
        observed_identity: ExecutionIdentity::new(6, [7; 16]),
    })?;
    writer.append(&JournalRecord::ForwardStepPrepared { step_index: 1 })?;
    writer.append(&JournalRecord::ForwardStepCompleted {
        step_index: 1,
        observed_identity: ExecutionIdentity::new(6, [7; 16]),
    })?;
    writer.append(&JournalRecord::TransactionCompleted)?;

    assert_eq!(writer.next_sequence(), 6);
    assert!(writer.is_terminal());
    let records = decode_journal(&fs::read(path)?)?
        .into_iter()
        .map(|frame| frame.into_record())
        .collect::<Vec<_>>();
    assert_eq!(replay_journal(&records)?, JournalStatus::Completed);
    Ok(())
}

#[test]
fn create_new_never_truncates_an_existing_file() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("transaction.rwj");
    fs::write(&path, b"existing journal")?;

    let error = JournalWriter::create_new(&path, &transaction_started())
        .err()
        .ok_or("existing file was replaced")?;

    assert_eq!(error.kind(), JournalStorageErrorKind::AlreadyExists);
    assert_eq!(fs::read(path)?, b"existing journal");
    assert!(
        !error
            .to_string()
            .contains(directory.path().to_string_lossy().as_ref())
    );
    Ok(())
}

#[test]
fn invalid_initial_record_does_not_create_a_file() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("transaction.rwj");

    let error = JournalWriter::create_new(&path, &JournalRecord::TransactionCompleted)
        .err()
        .ok_or("terminal record was accepted as a header")?;

    assert_eq!(error.kind(), JournalStorageErrorKind::InvalidInitialRecord);
    assert!(!path.exists());
    Ok(())
}

#[test]
fn writer_rejects_second_header_and_records_after_terminal()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("transaction.rwj");
    let mut writer = JournalWriter::create_new(&path, &transaction_started())?;
    let length_after_header = fs::metadata(&path)?.len();

    let header_error = writer
        .append(&transaction_started())
        .err()
        .ok_or("second header was accepted")?;
    assert_eq!(
        header_error.kind(),
        JournalStorageErrorKind::HeaderAfterStart
    );
    assert_eq!(fs::metadata(&path)?.len(), length_after_header);

    writer.append(&JournalRecord::TransactionCompleted)?;
    let length_after_terminal = fs::metadata(&path)?.len();
    let terminal_error = writer
        .append(&JournalRecord::ForwardStepPrepared { step_index: 0 })
        .err()
        .ok_or("post-terminal record was accepted")?;
    assert_eq!(
        terminal_error.kind(),
        JournalStorageErrorKind::RecordAfterTerminal
    );
    assert_eq!(fs::metadata(path)?.len(), length_after_terminal);
    Ok(())
}
