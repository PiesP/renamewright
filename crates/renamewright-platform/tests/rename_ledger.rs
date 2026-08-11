use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use renamewright_core::{
    EntryKind, ExecutionIdentity, JournalEntry, JournalNameGraph, JournalRecord, ParentId, PlanId,
    SourceFingerprint, SourceId,
};
use renamewright_platform::{LedgerStatus, MAX_JOURNAL_FILE_BYTES, RenameLedger, encode_journal};

fn header() -> JournalRecord {
    JournalRecord::TransactionStarted {
        plan_id: PlanId::new(7),
        source_generation: 11,
        step_count: 2,
        entries: vec![JournalEntry::with_native_parent(
            SourceId::new(13),
            ParentId::new(17),
            JournalNameGraph::new(
                OsString::from("original.txt"),
                OsString::from("temporary.tmp"),
                OsString::from("final.txt"),
            ),
            SourceFingerprint::new(EntryKind::File, None, 19, None),
            ExecutionIdentity::new(23, [29; 16]),
            PathBuf::from("native-parent"),
        )],
    }
}

#[test]
fn projects_terminal_and_interrupted_transactions_without_native_paths()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(
        directory.path().join("a-completed.rwj"),
        encode_journal(&[
            header(),
            JournalRecord::ForwardStepPrepared { step_index: 0 },
            JournalRecord::ForwardStepCompleted {
                step_index: 0,
                observed_identity: ExecutionIdentity::new(23, [29; 16]),
            },
            JournalRecord::ForwardStepPrepared { step_index: 1 },
            JournalRecord::ForwardStepCompleted {
                step_index: 1,
                observed_identity: ExecutionIdentity::new(23, [29; 16]),
            },
            JournalRecord::TransactionCompleted,
        ])?,
    )?;
    fs::write(
        directory.path().join("b-interrupted.rwj"),
        encode_journal(&[
            header(),
            JournalRecord::ForwardStepPrepared { step_index: 0 },
        ])?,
    )?;

    let ledger = RenameLedger::discover(directory.path())?;
    let entries = ledger.entries().collect::<Vec<_>>();

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].status(), LedgerStatus::Completed);
    assert!(!entries[0].recovery_available());
    assert_eq!(entries[1].status(), LedgerStatus::ReconciliationRequired);
    assert_eq!(entries[1].attention_step(), Some(0));
    assert!(entries[1].recovery_available());
    assert_eq!(entries[1].plan_id(), Some(PlanId::new(7)));
    assert_eq!(entries[1].source_count(), 1);
    Ok(())
}

#[test]
fn classifies_torn_corrupt_oversized_and_unrelated_entries()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut torn = encode_journal(&[
        header(),
        JournalRecord::ForwardStepPrepared { step_index: 0 },
    ])?;
    torn.pop().ok_or("journal was empty")?;
    fs::write(directory.path().join("a-torn.rwj"), torn)?;

    let mut corrupt = encode_journal(&[header()])?;
    let Some(last) = corrupt.last_mut() else {
        return Err("journal was empty".into());
    };
    *last ^= 0xff;
    fs::write(directory.path().join("b-corrupt.rwj"), corrupt)?;

    let oversized = fs::File::create(directory.path().join("c-oversized.rwj"))?;
    oversized.set_len(MAX_JOURNAL_FILE_BYTES.saturating_add(1))?;
    fs::write(directory.path().join("ignored.txt"), b"not a journal")?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        directory.path().join("a-torn.rwj"),
        directory.path().join("ignored-symlink.rwj"),
    )?;

    let ledger = RenameLedger::discover(directory.path())?;
    let statuses = ledger
        .entries()
        .map(|entry| entry.status())
        .collect::<Vec<_>>();

    assert_eq!(
        statuses,
        vec![
            LedgerStatus::Torn,
            LedgerStatus::Damaged,
            LedgerStatus::TooLarge
        ]
    );
    Ok(())
}

#[test]
fn a_missing_directory_is_an_empty_ledger() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let missing = directory.path().join("missing");

    let ledger = RenameLedger::discover(&missing)?;

    assert_eq!(ledger.entries().len(), 0);
    Ok(())
}
