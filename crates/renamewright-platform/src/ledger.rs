use std::collections::BTreeSet;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use renamewright_core::{JournalRecord, JournalStatus, PlanId, replay_journal};

use crate::{JournalCodecErrorKind, JournalInspection, MAX_JOURNAL_FILE_BYTES, inspect_journal};

pub const MAX_DISCOVERED_JOURNALS: usize = 1_024;

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
    inspection: Option<JournalInspection>,
}

#[derive(Debug, Default)]
pub struct RenameLedger {
    root: Option<PathBuf>,
    items: Vec<CatalogItem>,
}

impl RenameLedger {
    pub fn discover(root: &Path) -> Result<Self, LedgerDiscoveryError> {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    root: Some(root.to_path_buf()),
                    items: Vec::new(),
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
        let mut candidates = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension() == Some(OsStr::new("rwj")))
            .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_file()))
            .collect::<Vec<_>>();
        candidates.sort_by_key(fs::DirEntry::file_name);
        if candidates.len() > MAX_DISCOVERED_JOURNALS {
            return Err(LedgerDiscoveryError {
                kind: LedgerDiscoveryErrorKind::TooManyJournals,
            });
        }

        let mut items = Vec::with_capacity(candidates.len());
        for (index, candidate) in candidates.into_iter().enumerate() {
            let ledger_id = LedgerId(u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1));
            let native_path = candidate.path();
            let (projection, inspection) = match read_bounded_journal(&native_path) {
                Ok(bytes) => {
                    let inspection = inspect_journal(&bytes);
                    (project_inspection(ledger_id, &inspection), Some(inspection))
                }
                Err(ReadJournalError::TooLarge) => {
                    (empty_projection(ledger_id, LedgerStatus::TooLarge), None)
                }
                Err(ReadJournalError::Io) => {
                    (empty_projection(ledger_id, LedgerStatus::Unreadable), None)
                }
            };
            items.push(CatalogItem {
                projection,
                native_path,
                inspection,
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
        Ok(Self {
            root: Some(root.to_path_buf()),
            items,
        })
    }

    #[must_use]
    pub fn entries(&self) -> impl ExactSizeIterator<Item = LedgerEntry> + '_ {
        self.items.iter().map(|item| item.projection)
    }

    pub fn refresh(&mut self) -> Result<(), LedgerDiscoveryError> {
        let root = self.root.clone().ok_or(LedgerDiscoveryError {
            kind: LedgerDiscoveryErrorKind::RootUnavailable {
                io_kind: io::ErrorKind::InvalidInput,
            },
        })?;
        *self = Self::discover(&root)?;
        Ok(())
    }

    pub(crate) fn item(&self, ledger_id: LedgerId) -> Option<(&Path, &JournalInspection)> {
        self.items
            .iter()
            .find(|item| item.projection.ledger_id == ledger_id)
            .and_then(|item| {
                item.inspection
                    .as_ref()
                    .map(|inspection| (item.native_path.as_path(), inspection))
            })
    }

    pub(crate) fn entry(&self, ledger_id: LedgerId) -> Option<LedgerEntry> {
        self.items
            .iter()
            .find(|item| item.projection.ledger_id == ledger_id)
            .map(|item| item.projection)
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

    let records = inspection
        .frames()
        .iter()
        .map(|frame| frame.record().clone())
        .collect::<Vec<_>>();
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
    let Ok(journal_status) = replay_journal(&records) else {
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
    let (mut status, attention_step, mut recovery_available) = project_status(journal_status);
    if !matches!(status, LedgerStatus::Completed | LedgerStatus::RolledBack)
        && !has_recovery_locators
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
            && has_native_parents,
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
    Io,
}

fn read_bounded_journal(path: &Path) -> Result<Vec<u8>, ReadJournalError> {
    let mut file = open_journal_no_follow(path).map_err(|_| ReadJournalError::Io)?;
    let metadata = file.metadata().map_err(|_| ReadJournalError::Io)?;
    if !metadata.is_file() {
        return Err(ReadJournalError::Io);
    }
    if metadata.len() > MAX_JOURNAL_FILE_BYTES {
        return Err(ReadJournalError::TooLarge);
    }
    let limit = MAX_JOURNAL_FILE_BYTES.saturating_add(1);
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadJournalError::Io)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_JOURNAL_FILE_BYTES {
        Err(ReadJournalError::TooLarge)
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
