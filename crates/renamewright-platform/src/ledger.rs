use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::ffi::OsStr;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use renamewright_core::{JournalRecord, JournalStatus, PlanId, replay_journal};

use crate::{JournalCodecErrorKind, JournalInspection, MAX_JOURNAL_FILE_BYTES, inspect_journal};

pub const MAX_DISCOVERED_JOURNALS: usize = 1_024;
pub const MAX_DISCOVERED_JOURNAL_BYTES: u64 = MAX_JOURNAL_FILE_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LedgerId(u64);

impl LedgerId {
    #[must_use]
    pub const fn from_value(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerStatus {
    Completed,
    RolledBack,
    ForwardPending,
    CompletionPending,
    RollbackPending,
    RollbackCompletionPending,
    ReconciliationRequired,
    RecoveryRequired,
    LegacyInspectionRequired,
    Torn,
    Damaged,
    UnsupportedVersion,
    TooLarge,
    DiscoveryLimitExceeded,
    Unreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LedgerEntry {
    ledger_id: LedgerId,
    plan_id: Option<PlanId>,
    source_generation: Option<u64>,
    schema_version: Option<u16>,
    source_count: usize,
    status: LedgerStatus,
    attention_step: Option<usize>,
    recovery_available: bool,
    undo_of_plan_id: Option<PlanId>,
    undo_available: bool,
}

impl LedgerEntry {
    #[must_use]
    pub const fn ledger_id(self) -> LedgerId {
        self.ledger_id
    }

    #[must_use]
    pub const fn plan_id(self) -> Option<PlanId> {
        self.plan_id
    }

    #[must_use]
    pub const fn source_generation(self) -> Option<u64> {
        self.source_generation
    }

    #[must_use]
    pub const fn schema_version(self) -> Option<u16> {
        self.schema_version
    }

    #[must_use]
    pub const fn source_count(self) -> usize {
        self.source_count
    }

    #[must_use]
    pub const fn status(self) -> LedgerStatus {
        self.status
    }

    #[must_use]
    pub const fn attention_step(self) -> Option<usize> {
        self.attention_step
    }

    #[must_use]
    pub const fn recovery_available(self) -> bool {
        self.recovery_available
    }

    #[must_use]
    pub const fn undo_of_plan_id(self) -> Option<PlanId> {
        self.undo_of_plan_id
    }

    #[must_use]
    pub const fn undo_available(self) -> bool {
        self.undo_available
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedgerDiscoveryErrorKind {
    RootUnavailable { io_kind: io::ErrorKind },
    TooManyJournals,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LedgerDiscoveryError {
    kind: LedgerDiscoveryErrorKind,
}

impl LedgerDiscoveryError {
    #[must_use]
    pub const fn kind(self) -> LedgerDiscoveryErrorKind {
        self.kind
    }
}

impl Display for LedgerDiscoveryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the rename ledger could not be discovered ({:?})",
            self.kind
        )
    }
}

impl Error for LedgerDiscoveryError {}

#[derive(Debug)]
struct CatalogItem {
    projection: LedgerEntry,
    native_path: PathBuf,
    inspectable: bool,
}

#[derive(Debug, Default)]
pub struct RenameLedger {
    root: Option<PathBuf>,
    items: Vec<CatalogItem>,
    latest_plan_id: Option<PlanId>,
}

impl RenameLedger {
    pub fn discover(root: &Path) -> Result<Self, LedgerDiscoveryError> {
        Self::discover_with_limits(root, MAX_DISCOVERED_JOURNALS, MAX_DISCOVERED_JOURNAL_BYTES)
    }

    fn discover_with_limits(
        root: &Path,
        max_journals: usize,
        max_total_bytes: u64,
    ) -> Result<Self, LedgerDiscoveryError> {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    root: Some(root.to_path_buf()),
                    items: Vec::new(),
                    latest_plan_id: None,
                });
            }
            Err(error) => {
                return Err(LedgerDiscoveryError {
                    kind: LedgerDiscoveryErrorKind::RootUnavailable {
                        io_kind: error.kind(),
                    },
                });
            }
        };
        let (candidates, count_limited, latest_named_plan_id) =
            bounded_journal_candidates(entries, max_journals);

        let mut items =
            Vec::with_capacity(candidates.len().saturating_add(usize::from(count_limited)));
        let mut remaining_bytes = max_total_bytes;
        for (index, native_path) in candidates.into_iter().enumerate() {
            let ledger_id = LedgerId(u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1));
            let (projection, inspectable) =
                match read_bounded_journal(&native_path, remaining_bytes) {
                    Ok(bytes) => {
                        remaining_bytes = remaining_bytes
                            .saturating_sub(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                        let inspection = inspect_journal(&bytes);
                        (project_inspection(ledger_id, &inspection), true)
                    }
                    Err(ReadJournalError::TooLarge) => {
                        (empty_projection(ledger_id, LedgerStatus::TooLarge), false)
                    }
                    Err(ReadJournalError::DiscoveryLimitExceeded) => (
                        empty_projection(ledger_id, LedgerStatus::DiscoveryLimitExceeded),
                        false,
                    ),
                    Err(ReadJournalError::Io) => {
                        (empty_projection(ledger_id, LedgerStatus::Unreadable), false)
                    }
                };
            items.push(CatalogItem {
                projection,
                native_path,
                inspectable,
            });
        }
        if count_limited {
            let ledger_id = LedgerId(
                u64::try_from(items.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(1),
            );
            items.push(CatalogItem {
                projection: empty_projection(ledger_id, LedgerStatus::DiscoveryLimitExceeded),
                native_path: root.to_path_buf(),
                inspectable: false,
            });
        }
        let superseded_plan_ids = items
            .iter()
            .filter(|item| item.projection.status != LedgerStatus::RolledBack)
            .filter_map(|item| item.projection.undo_of_plan_id)
            .collect::<BTreeSet<_>>();
        for item in &mut items {
            if item
                .projection
                .plan_id
                .is_some_and(|plan_id| superseded_plan_ids.contains(&plan_id))
            {
                item.projection.undo_available = false;
            }
        }
        let latest_plan_id = items
            .iter()
            .filter_map(|item| item.projection.plan_id)
            .chain(latest_named_plan_id)
            .max();
        Ok(Self {
            root: Some(root.to_path_buf()),
            items,
            latest_plan_id,
        })
    }

    #[must_use]
    pub fn entries(&self) -> impl ExactSizeIterator<Item = LedgerEntry> + '_ {
        self.items.iter().map(|item| item.projection)
    }

    #[must_use]
    pub const fn latest_plan_id(&self) -> Option<PlanId> {
        self.latest_plan_id
    }

    pub fn refresh(&mut self) -> Result<(), LedgerDiscoveryError> {
        let root = self.root.clone().ok_or(LedgerDiscoveryError {
            kind: LedgerDiscoveryErrorKind::RootUnavailable {
                io_kind: io::ErrorKind::InvalidInput,
            },
        })?;
        let retained_ids = self
            .items
            .iter()
            .map(|item| (item.native_path.clone(), item.projection.ledger_id))
            .collect::<BTreeMap<_, _>>();
        let mut next_ledger_id = self
            .items
            .iter()
            .map(|item| item.projection.ledger_id.value())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut refreshed = Self::discover(&root)?;
        for item in &mut refreshed.items {
            item.projection.ledger_id = retained_ids
                .get(&item.native_path)
                .copied()
                .unwrap_or_else(|| {
                    let ledger_id = LedgerId::from_value(next_ledger_id);
                    next_ledger_id = next_ledger_id.saturating_add(1);
                    ledger_id
                });
        }
        *self = refreshed;
        Ok(())
    }

    pub(crate) fn item(&self, ledger_id: LedgerId) -> Option<(PathBuf, JournalInspection)> {
        let item = self
            .items
            .iter()
            .find(|item| item.projection.ledger_id == ledger_id && item.inspectable)?;
        let bytes = read_bounded_journal(&item.native_path, MAX_DISCOVERED_JOURNAL_BYTES).ok()?;
        Some((item.native_path.clone(), inspect_journal(&bytes)))
    }

    pub(crate) fn journal_path(&self, ledger_id: LedgerId) -> Option<&Path> {
        self.items
            .iter()
            .find(|item| item.projection.ledger_id == ledger_id && item.inspectable)
            .map(|item| item.native_path.as_path())
    }

    pub(crate) fn projection_matches_header(
        &self,
        ledger_id: LedgerId,
        plan_id: PlanId,
        source_generation: u64,
        source_count: usize,
    ) -> bool {
        self.items.iter().any(|item| {
            item.projection.ledger_id == ledger_id
                && item.projection.plan_id == Some(plan_id)
                && item.projection.source_generation == Some(source_generation)
                && item.projection.source_count == source_count
        })
    }

    pub(crate) fn journal_path_for_plan(&self, plan_id: PlanId) -> Option<PathBuf> {
        self.root
            .as_ref()
            .map(|root| root.join(format!("undo-{:016x}.rwj", plan_id.value())))
    }
}

fn empty_projection(ledger_id: LedgerId, status: LedgerStatus) -> LedgerEntry {
    LedgerEntry {
        ledger_id,
        plan_id: None,
        source_generation: None,
        schema_version: None,
        source_count: 0,
        status,
        attention_step: None,
        recovery_available: false,
        undo_of_plan_id: None,
        undo_available: false,
    }
}

fn project_inspection(ledger_id: LedgerId, inspection: &JournalInspection) -> LedgerEntry {
    let header = inspection.frames().first().and_then(|frame| {
        let JournalRecord::TransactionStarted {
            plan_id,
            source_generation,
            entries,
            ..
        } = frame.record()
        else {
            return None;
        };
        Some((*plan_id, *source_generation, entries))
    });
    let schema_version = inspection
        .frames()
        .first()
        .map(|frame| frame.schema_version());
    let (
        plan_id,
        source_generation,
        source_count,
        undo_of_plan_id,
        has_native_parents,
        has_consistent_lineage,
    ) = header
        .map(|(plan_id, generation, entries)| {
            let undo_of_plan_id = entries.first().and_then(|entry| entry.undo_of_plan_id());
            let has_consistent_lineage = entries
                .iter()
                .all(|entry| entry.undo_of_plan_id() == undo_of_plan_id);
            (
                Some(plan_id),
                Some(generation),
                entries.len(),
                undo_of_plan_id,
                !entries.is_empty() && entries.iter().all(|entry| entry.native_parent().is_some()),
                has_consistent_lineage,
            )
        })
        .unwrap_or((None, None, 0, None, false, true));

    if let Some(issue) = inspection.issue() {
        let status = match issue.kind() {
            JournalCodecErrorKind::UnsupportedVersion { .. } => LedgerStatus::UnsupportedVersion,
            JournalCodecErrorKind::TruncatedHeader | JournalCodecErrorKind::TruncatedPayload => {
                LedgerStatus::Torn
            }
            _ => LedgerStatus::Damaged,
        };
        return LedgerEntry {
            ledger_id,
            plan_id,
            source_generation,
            schema_version,
            source_count,
            status,
            attention_step: Some(issue.frame_index()),
            recovery_available: false,
            undo_of_plan_id,
            undo_available: false,
        };
    }

    if !has_consistent_lineage {
        return LedgerEntry {
            ledger_id,
            plan_id,
            source_generation,
            schema_version,
            source_count,
            status: LedgerStatus::Damaged,
            attention_step: None,
            recovery_available: false,
            undo_of_plan_id,
            undo_available: false,
        };
    }
    let Ok(journal_status) = replay_journal(inspection.frames().iter().map(|frame| frame.record()))
    else {
        return LedgerEntry {
            ledger_id,
            plan_id,
            source_generation,
            schema_version,
            source_count,
            status: LedgerStatus::Damaged,
            attention_step: None,
            recovery_available: false,
            undo_of_plan_id,
            undo_available: false,
        };
    };
    let has_recovery_locators = header
        .is_some_and(|(_, _, entries)| entries.iter().all(|entry| entry.native_parent().is_some()));
    let has_parent_execution_identities = header.is_some_and(|(_, _, entries)| {
        !entries.is_empty()
            && entries
                .iter()
                .all(|entry| entry.parent_execution_identity().is_some())
    });
    let (mut status, attention_step, mut recovery_available) = project_status(journal_status);
    if status != LedgerStatus::RolledBack
        && (!has_recovery_locators || !has_parent_execution_identities)
    {
        status = LedgerStatus::LegacyInspectionRequired;
        recovery_available = false;
    }

    LedgerEntry {
        ledger_id,
        plan_id,
        source_generation,
        schema_version,
        source_count,
        status,
        attention_step,
        recovery_available,
        undo_of_plan_id,
        undo_available: status == LedgerStatus::Completed
            && undo_of_plan_id.is_none()
            && has_native_parents
            && has_parent_execution_identities,
    }
}

const fn project_status(status: JournalStatus) -> (LedgerStatus, Option<usize>, bool) {
    match status {
        JournalStatus::ForwardPending { next_step } => {
            (LedgerStatus::ForwardPending, Some(next_step), true)
        }
        JournalStatus::CompletionPending => (LedgerStatus::CompletionPending, None, true),
        JournalStatus::RollbackPending { next_step, .. } => {
            (LedgerStatus::RollbackPending, Some(next_step), true)
        }
        JournalStatus::RollbackCompletionPending { .. } => {
            (LedgerStatus::RollbackCompletionPending, None, true)
        }
        JournalStatus::ReconciliationRequired { step_index, .. } => {
            (LedgerStatus::ReconciliationRequired, Some(step_index), true)
        }
        JournalStatus::RecoveryRequired { failed_step, .. } => {
            (LedgerStatus::RecoveryRequired, Some(failed_step), true)
        }
        JournalStatus::Completed => (LedgerStatus::Completed, None, false),
        JournalStatus::RolledBack { .. } => (LedgerStatus::RolledBack, None, false),
    }
}

enum ReadJournalError {
    TooLarge,
    DiscoveryLimitExceeded,
    Io,
}

fn bounded_journal_candidates(
    entries: fs::ReadDir,
    max_journals: usize,
) -> (Vec<PathBuf>, bool, Option<PlanId>) {
    let mut candidates = BTreeMap::new();
    let mut count_limited = false;
    let mut latest_named_plan_id = None;
    for entry in entries.filter_map(Result::ok) {
        if entry.path().extension() != Some(OsStr::new("rwj"))
            || !entry.file_type().is_ok_and(|file_type| file_type.is_file())
        {
            continue;
        }
        let file_name = entry.file_name();
        let named_plan_id = plan_id_from_native_journal_name(&file_name);
        latest_named_plan_id = latest_named_plan_id.into_iter().chain(named_plan_id).max();
        candidates.insert((named_plan_id.map(PlanId::value), file_name), entry.path());
        if candidates.len() > max_journals {
            count_limited = true;
            if let Some(name) = candidates.keys().next().cloned() {
                candidates.remove(&name);
            }
        }
    }
    (
        candidates.into_values().collect(),
        count_limited,
        latest_named_plan_id,
    )
}

fn plan_id_from_native_journal_name(name: &OsStr) -> Option<PlanId> {
    let name = name.to_str()?;
    let encoded = name
        .strip_prefix("plan-")
        .or_else(|| name.strip_prefix("undo-"))?
        .strip_suffix(".rwj")?;
    if encoded.len() != 16 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(encoded, 16).ok().map(PlanId::new)
}

fn read_bounded_journal(path: &Path, remaining_bytes: u64) -> Result<Vec<u8>, ReadJournalError> {
    let mut file = open_journal_no_follow(path).map_err(|_| ReadJournalError::Io)?;
    let metadata = file.metadata().map_err(|_| ReadJournalError::Io)?;
    if !metadata.is_file() {
        return Err(ReadJournalError::Io);
    }
    if metadata.len() > MAX_JOURNAL_FILE_BYTES {
        return Err(ReadJournalError::TooLarge);
    }
    if metadata.len() > remaining_bytes {
        return Err(ReadJournalError::DiscoveryLimitExceeded);
    }
    let limit = remaining_bytes
        .min(MAX_JOURNAL_FILE_BYTES)
        .saturating_add(1);
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadJournalError::Io)?;
    let observed_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if observed_bytes > MAX_JOURNAL_FILE_BYTES {
        Err(ReadJournalError::TooLarge)
    } else if observed_bytes > remaining_bytes {
        Err(ReadJournalError::DiscoveryLimitExceeded)
    } else {
        Ok(bytes)
    }
}

#[cfg(target_os = "linux")]
fn open_journal_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let flags = i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    OpenOptions::new().read(true).custom_flags(flags).open(path)
}

#[cfg(windows)]
fn open_journal_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn open_journal_no_follow(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;

    use renamewright_core::{
        EntryKind, ExecutionIdentity, JournalEntry, JournalNameGraph, JournalRecord, ParentId,
        PlanId, SourceFingerprint, SourceId,
    };

    use super::{LedgerStatus, RenameLedger};

    #[test]
    fn aggregate_byte_limit_defers_later_journals() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join("a-first.rwj"), b"12345678")?;
        fs::write(directory.path().join("b-second.rwj"), b"abcdefgh")?;

        let ledger = RenameLedger::discover_with_limits(directory.path(), 2, 8)?;
        let statuses = ledger
            .entries()
            .map(|entry| entry.status())
            .collect::<Vec<_>>();

        assert_eq!(
            statuses,
            vec![LedgerStatus::Torn, LedgerStatus::DiscoveryLimitExceeded]
        );
        Ok(())
    }

    #[test]
    fn schema_four_journal_without_parent_identity_is_legacy_and_non_mutating()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let record = JournalRecord::TransactionStarted {
            plan_id: PlanId::new(9),
            source_generation: 1,
            step_count: 2,
            entries: vec![JournalEntry::with_native_parent(
                SourceId::new(1),
                ParentId::new(1),
                JournalNameGraph::new(
                    OsString::from("source.txt"),
                    OsString::from("temporary.tmp"),
                    OsString::from("final.txt"),
                ),
                SourceFingerprint::new(EntryKind::File, None, 1, None),
                ExecutionIdentity::new(1, [2; 16]),
                PathBuf::from("native-parent"),
            )],
        };
        fs::write(
            directory.path().join("legacy.rwj"),
            crate::journal::encode_frame_for_version(0, &record, 0, 4)?,
        )?;

        let ledger = RenameLedger::discover(directory.path())?;
        let entry = ledger.entries().next().ok_or("ledger was empty")?;

        assert_eq!(entry.status(), LedgerStatus::LegacyInspectionRequired);
        assert!(!entry.recovery_available());
        assert!(!entry.undo_available());
        Ok(())
    }
}
