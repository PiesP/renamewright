#![forbid(unsafe_code)]

mod execution_fs;
mod executor;
mod journal;
mod ledger;
mod recovery;
mod undo;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use renamewright_core::{
    EntryIdentitySignal, EntryKind, ExecutionIdentity, OccupiedName, ParentId, SourceFingerprint,
    SourceId, SourceSnapshot, ValidationEnvironment,
};

#[cfg(target_os = "linux")]
pub use execution_fs::LinuxExecutionFileSystem;
pub use execution_fs::{
    ExecutionFileSystem, ExecutionFsError, ExecutionFsErrorKind, NativeExecutionFileSystem,
    temporary_name,
};
pub use executor::{
    ExecutionOutcome, ExecutionRecovery, ExecutionRecoveryReason, ExecutionStartError,
    FreezeExecutionError, FreezeExecutionErrorKind, FrozenExecutionPlan,
    MAX_TEMPORARY_NAME_ATTEMPTS, execute_frozen_plan, freeze_execution_plan,
};
pub use journal::{
    JOURNAL_SCHEMA_VERSION, JournalCodecError, JournalCodecErrorKind, JournalFrame,
    JournalInspection, JournalStorageError, JournalStorageErrorKind, JournalWriter,
    MAX_JOURNAL_FILE_BYTES, MAX_JOURNAL_PAYLOAD_BYTES, MIN_SUPPORTED_JOURNAL_SCHEMA_VERSION,
    decode_journal, encode_journal, inspect_journal,
};
pub use ledger::{
    LedgerDiscoveryError, LedgerDiscoveryErrorKind, LedgerEntry, LedgerId, LedgerStatus,
    MAX_DISCOVERED_JOURNAL_BYTES, MAX_DISCOVERED_JOURNALS, RenameLedger,
};
pub use recovery::{
    PreparedStepDisposition, PreparedStepInspection, RecoveryAction, RecoveryActionError,
    RecoveryActionErrorKind, RecoveryInspectionError, RecoveryInspectionErrorKind,
    RecoveryLocation, RecoveryLocationState, RecoveryObservation, RecoveryReadiness,
    RecoveryTransactionInspection, inspect_prepared_step, inspect_recovery_transaction,
    reconcile_prepared_step, recover_transaction,
};
pub use undo::{
    PreparedUndo, UndoBlockReason, UndoError, UndoErrorKind, UndoReadiness,
    UndoTransactionInspection, UndoTransactionSnapshot, execute_prepared_undo,
    inspect_undo_transaction, inspect_undo_transaction_snapshot, prepare_undo_transaction,
    prepare_undo_transaction_from_snapshot,
};

pub const MAX_ADMITTED_SOURCES: usize = 10_000;

/// Applying a newly planned rename is enabled through the application service.
#[must_use]
pub const fn plan_execution_is_enabled() -> bool {
    true
}

/// Startup recovery is available after the recovery safety gates pass.
#[must_use]
pub const fn recovery_execution_is_enabled() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    TooManySources,
    Unavailable(PathBuf),
    MissingFileName(PathBuf),
}

impl Display for AdmissionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySources => formatter.write_str("too many sources were selected"),
            Self::Unavailable(_) => {
                formatter.write_str("a selected source is not an available file")
            }
            Self::MissingFileName(_) => formatter.write_str("a selected source has no file name"),
        }
    }
}

impl Error for AdmissionError {}

#[derive(Debug, Default)]
pub struct SourceRegistry {
    paths: BTreeMap<SourceId, PathBuf>,
    snapshots: BTreeMap<SourceId, SourceSnapshot>,
    execution_identities: BTreeMap<SourceId, ExecutionIdentity>,
    source_ids: BTreeMap<PathBuf, SourceId>,
    parent_ids: BTreeMap<PathBuf, ParentId>,
    next_source_id: u64,
    next_parent_id: u64,
    generation: u64,
}

/// An immutable, generation-bound copy of the data needed to build a preview.
///
/// Capturing this value is intentionally separate from filesystem validation so
/// callers can release the registry lock before directory enumeration and rule
/// evaluation begin.
#[derive(Clone, Debug)]
pub struct PlanningSnapshot {
    paths: BTreeMap<SourceId, PathBuf>,
    snapshots: Vec<SourceSnapshot>,
    parent_ids: BTreeMap<PathBuf, ParentId>,
    generation: u64,
}

impl PlanningSnapshot {
    #[must_use]
    pub fn snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn validation_environment(&self) -> ValidationEnvironment {
        validation_environment(&self.paths, &self.snapshots, &self.parent_ids)
    }
}

impl SourceRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_source_id: 1,
            next_parent_id: 1,
            ..Self::default()
        }
    }

    pub fn admit_paths(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Vec<SourceSnapshot>, AdmissionError> {
        let paths = paths
            .into_iter()
            .take(MAX_ADMITTED_SOURCES.saturating_add(1))
            .collect::<Vec<_>>();
        if paths.len() > MAX_ADMITTED_SOURCES {
            return Err(AdmissionError::TooManySources);
        }
        let mut candidates = Vec::with_capacity(paths.len());

        for path in paths {
            candidates.push(normalize_entry_path(path)?);
        }

        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        candidates.dedup_by(|left, right| left.0 == right.0);
        let new_source_count = candidates
            .iter()
            .filter(|(path, _)| !self.source_ids.contains_key(path))
            .count();
        if self.source_ids.len().saturating_add(new_source_count) > MAX_ADMITTED_SOURCES {
            return Err(AdmissionError::TooManySources);
        }
        self.admit_candidates(candidates)
    }

    fn admit_candidates(
        &mut self,
        candidates: Vec<(PathBuf, SourceFingerprint)>,
    ) -> Result<Vec<SourceSnapshot>, AdmissionError> {
        let mut prepared = Vec::with_capacity(candidates.len());
        for (path, fingerprint) in candidates {
            if self.source_ids.contains_key(&path) {
                continue;
            }

            let execution_identity = admission_execution_identity(&path);
            let current_fingerprint = fs::symlink_metadata(&path)
                .ok()
                .and_then(|metadata| fingerprint_for(&metadata));
            if current_fingerprint.as_ref() != Some(&fingerprint) {
                return Err(AdmissionError::Unavailable(path));
            }
            prepared.push((path, fingerprint, execution_identity));
        }

        let mut changed = false;

        for (path, fingerprint, execution_identity) in prepared {
            let source_id = SourceId::new(self.next_source_id);
            self.next_source_id = self.next_source_id.saturating_add(1);
            let parent = path
                .parent()
                .ok_or_else(|| AdmissionError::MissingFileName(path.clone()))?;
            let parent_id = if let Some(parent_id) = self.parent_ids.get(parent) {
                *parent_id
            } else {
                let parent_id = ParentId::new(self.next_parent_id);
                self.next_parent_id = self.next_parent_id.saturating_add(1);
                self.parent_ids.insert(parent.to_path_buf(), parent_id);
                parent_id
            };
            let name = path
                .file_name()
                .ok_or_else(|| AdmissionError::MissingFileName(path.clone()))?;
            let snapshot = SourceSnapshot::with_fingerprint(
                source_id,
                parent_id,
                name.to_os_string(),
                fingerprint,
            );
            self.source_ids.insert(path.clone(), source_id);
            self.paths.insert(source_id, path);
            self.snapshots.insert(source_id, snapshot);
            if let Some(execution_identity) = execution_identity {
                self.execution_identities
                    .insert(source_id, execution_identity);
            }
            changed = true;
        }

        if changed {
            self.generation = self.generation.saturating_add(1);
        }

        Ok(self.snapshots())
    }

    #[must_use]
    pub fn snapshots(&self) -> Vec<SourceSnapshot> {
        self.snapshots.values().cloned().collect()
    }

    pub fn remove_sources(&mut self, source_ids: &[SourceId]) -> usize {
        let mut removed_count = 0;
        for source_id in source_ids.iter().copied().collect::<BTreeSet<_>>() {
            let Some(path) = self.paths.remove(&source_id) else {
                continue;
            };
            self.source_ids.remove(&path);
            self.snapshots.remove(&source_id);
            self.execution_identities.remove(&source_id);
            removed_count += 1;
        }
        if removed_count > 0 {
            let retained_parent_ids = self
                .snapshots
                .values()
                .map(SourceSnapshot::parent_id)
                .collect::<BTreeSet<_>>();
            self.parent_ids
                .retain(|_, parent_id| retained_parent_ids.contains(parent_id));
            self.generation = self.generation.saturating_add(1);
        }
        removed_count
    }

    #[must_use]
    pub fn planning_snapshot(&self) -> PlanningSnapshot {
        PlanningSnapshot {
            paths: self.paths.clone(),
            snapshots: self.snapshots(),
            parent_ids: self.parent_ids.clone(),
            generation: self.generation,
        }
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn path_for(&self, source_id: SourceId) -> Option<&Path> {
        self.paths.get(&source_id).map(PathBuf::as_path)
    }

    #[must_use]
    pub(crate) fn execution_identity_for(&self, source_id: SourceId) -> Option<ExecutionIdentity> {
        self.execution_identities.get(&source_id).copied()
    }

    #[must_use]
    pub fn validation_environment(&self) -> ValidationEnvironment {
        self.planning_snapshot().validation_environment()
    }
}

fn validation_environment(
    paths: &BTreeMap<SourceId, PathBuf>,
    snapshots: &[SourceSnapshot],
    parent_ids: &BTreeMap<PathBuf, ParentId>,
) -> ValidationEnvironment {
    let snapshots = snapshots
        .iter()
        .map(|snapshot| (snapshot.id(), snapshot))
        .collect::<BTreeMap<_, _>>();
    let stale_sources = paths
        .iter()
        .filter_map(|(source_id, path)| {
            let snapshot = snapshots.get(source_id)?;
            let current = fs::symlink_metadata(path)
                .ok()
                .and_then(|metadata| fingerprint_for(&metadata));
            (current.as_ref() != snapshot.fingerprint()).then_some(*source_id)
        })
        .collect::<BTreeSet<_>>();
    let source_paths = paths.values().cloned().collect::<BTreeSet<_>>();
    let mut unavailable_parents = BTreeSet::new();
    let mut occupied_names = Vec::new();

    for (parent, parent_id) in parent_ids {
        let Ok(entries) = fs::read_dir(parent) else {
            unavailable_parents.insert(*parent_id);
            continue;
        };
        let mut parent_names = Vec::new();
        for entry in entries {
            let Ok(entry) = entry else {
                unavailable_parents.insert(*parent_id);
                parent_names.clear();
                break;
            };
            if !source_paths.contains(&entry.path()) {
                parent_names.push(OccupiedName::new(*parent_id, entry.file_name()));
            }
        }
        occupied_names.extend(parent_names);
    }
    occupied_names.sort_by(|left, right| {
        (left.parent_id(), left.native_name()).cmp(&(right.parent_id(), right.native_name()))
    });

    let ancestor_conflicts = ancestor_conflicts(paths, &snapshots);

    ValidationEnvironment::new(stale_sources, unavailable_parents, occupied_names)
        .with_ancestor_conflicts(ancestor_conflicts)
}

fn ancestor_conflicts(
    paths: &BTreeMap<SourceId, PathBuf>,
    snapshots: &BTreeMap<SourceId, &SourceSnapshot>,
) -> BTreeSet<SourceId> {
    let mut selected = paths
        .iter()
        .map(|(source_id, path)| {
            let is_directory = snapshots
                .get(source_id)
                .and_then(|snapshot| snapshot.entry_kind())
                == Some(EntryKind::Directory);
            (path, *source_id, is_directory)
        })
        .collect::<Vec<_>>();
    selected.sort_unstable_by(|left, right| left.0.cmp(right.0));

    let mut conflicts = BTreeSet::new();
    let mut directory_stack = Vec::<(&Path, SourceId)>::new();
    for (path, source_id, is_directory) in selected {
        while directory_stack
            .last()
            .is_some_and(|(directory, _)| !path.starts_with(directory))
        {
            directory_stack.pop();
        }
        if let Some((_, ancestor_id)) = directory_stack.last() {
            conflicts.insert(*ancestor_id);
            conflicts.insert(source_id);
        }
        if is_directory {
            directory_stack.push((path.as_path(), source_id));
        }
    }
    conflicts
}

#[cfg(target_os = "linux")]
fn admission_execution_identity(path: &Path) -> Option<ExecutionIdentity> {
    let parent = path.parent()?;
    let name = path.file_name()?;
    LinuxExecutionFileSystem::new().identity(parent, name).ok()
}

#[cfg(windows)]
fn admission_execution_identity(path: &Path) -> Option<ExecutionIdentity> {
    let parent = path.parent()?;
    let name = path.file_name()?;
    NativeExecutionFileSystem::new().identity(parent, name).ok()
}

#[cfg(not(any(target_os = "linux", windows)))]
const fn admission_execution_identity(_path: &Path) -> Option<ExecutionIdentity> {
    None
}

fn normalize_entry_path(path: PathBuf) -> Result<(PathBuf, SourceFingerprint), AdmissionError> {
    let absolute =
        std::path::absolute(&path).map_err(|_| AdmissionError::Unavailable(path.clone()))?;
    let name = absolute
        .file_name()
        .ok_or_else(|| AdmissionError::MissingFileName(absolute.clone()))?;
    let parent = absolute
        .parent()
        .ok_or_else(|| AdmissionError::MissingFileName(absolute.clone()))?
        .canonicalize()
        .map_err(|_| AdmissionError::Unavailable(path.clone()))?;
    let normalized = parent.join(name);
    let metadata =
        fs::symlink_metadata(&normalized).map_err(|_| AdmissionError::Unavailable(path.clone()))?;
    let fingerprint = fingerprint_for(&metadata).ok_or(AdmissionError::Unavailable(path))?;
    Ok((normalized, fingerprint))
}

fn fingerprint_for(metadata: &Metadata) -> Option<SourceFingerprint> {
    let entry_kind = if metadata.file_type().is_symlink() {
        EntryKind::Symlink
    } else if metadata.file_type().is_file() {
        EntryKind::File
    } else if metadata.file_type().is_dir() {
        EntryKind::Directory
    } else {
        return None;
    };
    let modified_nanos = (entry_kind != EntryKind::Directory)
        .then(|| {
            metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
        })
        .flatten();
    Some(SourceFingerprint::new(
        entry_kind,
        entry_identity_signal(metadata),
        if entry_kind == EntryKind::Directory {
            0
        } else {
            metadata.len()
        },
        modified_nanos,
    ))
}

#[cfg(unix)]
fn entry_identity_signal(metadata: &Metadata) -> Option<EntryIdentitySignal> {
    use std::os::unix::fs::MetadataExt;

    Some(EntryIdentitySignal::new(metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn entry_identity_signal(metadata: &Metadata) -> Option<EntryIdentitySignal> {
    use std::os::windows::fs::MetadataExt;

    Some(EntryIdentitySignal::new(
        metadata.creation_time(),
        u64::from(metadata.file_attributes()),
    ))
}

#[cfg(not(any(unix, windows)))]
const fn entry_identity_signal(_metadata: &Metadata) -> Option<EntryIdentitySignal> {
    None
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;

    #[cfg(unix)]
    use renamewright_core::EntryKind;
    use renamewright_core::SourceId;

    use super::{
        AdmissionError, MAX_ADMITTED_SOURCES, SourceRegistry, normalize_entry_path,
        plan_execution_is_enabled, recovery_execution_is_enabled,
    };

    #[test]
    fn plan_execution_and_recovery_are_enabled_together() {
        assert!(plan_execution_is_enabled());
        assert!(recovery_execution_is_enabled());
    }

    #[test]
    fn validation_detects_stale_sources_and_excludes_planned_entries() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("a.txt");
        let second = directory.path().join("final-a.txt");
        fs::write(&first, b"a")?;
        fs::write(&second, b"second")?;
        let mut registry = SourceRegistry::new();
        registry.admit_paths([first.clone(), second])?;

        let initial = registry.validation_environment();
        assert!(initial.stale_sources().is_empty());
        assert!(initial.occupied_names().is_empty());

        fs::write(first, b"changed-size")?;
        let changed = registry.validation_environment();
        assert_eq!(
            changed.stale_sources(),
            &std::collections::BTreeSet::from([SourceId::new(1)])
        );
        Ok(())
    }

    #[test]
    fn validation_reports_entries_outside_the_plan() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        fs::write(&source, b"source")?;
        fs::write(directory.path().join("final-report.txt"), b"occupied")?;
        let mut registry = SourceRegistry::new();
        registry.admit_paths([source])?;

        let environment = registry.validation_environment();

        assert_eq!(environment.occupied_names().len(), 1);
        assert_eq!(
            environment.occupied_names()[0].native_name(),
            "final-report.txt"
        );
        Ok(())
    }

    #[test]
    fn source_removal_uses_opaque_ids_and_advances_the_generation() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        fs::write(&first, b"first")?;
        fs::write(&second, b"second")?;
        let mut registry = SourceRegistry::new();
        registry.admit_paths([first.clone(), second])?;
        let admitted_generation = registry.generation();

        assert_eq!(registry.remove_sources(&[SourceId::new(1)]), 1);
        assert_eq!(registry.snapshots().len(), 1);
        assert_eq!(registry.generation(), admitted_generation + 1);
        assert!(registry.path_for(SourceId::new(1)).is_none());

        registry.admit_paths([first])?;
        assert_eq!(registry.snapshots().len(), 2);
        assert!(registry.path_for(SourceId::new(3)).is_some());
        Ok(())
    }

    #[test]
    fn planning_snapshot_remains_bound_to_its_captured_generation() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        fs::write(&first, b"first")?;
        fs::write(&second, b"second")?;
        let mut registry = SourceRegistry::new();
        registry.admit_paths([first])?;
        let snapshot = registry.planning_snapshot();

        registry.admit_paths([second])?;

        assert_eq!(snapshot.generation(), 1);
        assert_eq!(snapshot.snapshots().len(), 1);
        assert_eq!(registry.generation(), 2);
        assert_eq!(registry.snapshots().len(), 2);
        assert!(snapshot.validation_environment().stale_sources().is_empty());
        Ok(())
    }

    #[test]
    fn validation_degrades_an_unavailable_parent_to_plan_data() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let parent = directory.path().join("removed");
        fs::create_dir(&parent)?;
        let source = parent.join("report.txt");
        fs::write(&source, b"source")?;
        let mut registry = SourceRegistry::new();
        registry.admit_paths([source.clone()])?;

        fs::remove_file(source)?;
        fs::remove_dir(parent)?;
        let environment = registry.validation_environment();

        assert_eq!(
            environment.unavailable_parents(),
            &std::collections::BTreeSet::from([renamewright_core::ParentId::new(1)])
        );
        assert_eq!(
            environment.stale_sources(),
            &std::collections::BTreeSet::from([SourceId::new(1)])
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn validation_detects_same_size_entry_replacement() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("report.txt");
        let prior_entry = directory.path().join("prior-report.txt");
        fs::write(&source, b"same-size")?;
        let mut registry = SourceRegistry::new();
        registry.admit_paths([source.clone()])?;

        fs::rename(&source, prior_entry)?;
        fs::write(&source, b"same-size")?;
        let environment = registry.validation_environment();

        assert_eq!(
            environment.stale_sources(),
            &std::collections::BTreeSet::from([SourceId::new(1)])
        );
        Ok(())
    }

    #[test]
    fn admission_failure_does_not_commit_earlier_candidates() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("a.txt");
        let second = directory.path().join("b.txt");
        fs::write(&first, b"first")?;
        fs::write(&second, b"second")?;
        let candidates = vec![
            normalize_entry_path(first)?,
            normalize_entry_path(second.clone())?,
        ];
        fs::write(second, b"changed-after-normalization")?;
        let mut registry = SourceRegistry::new();

        let result = registry.admit_candidates(candidates);

        assert!(matches!(result, Err(super::AdmissionError::Unavailable(_))));
        assert_eq!(registry.generation(), 0);
        assert!(registry.snapshots().is_empty());
        assert!(registry.paths.is_empty());
        assert!(registry.execution_identities.is_empty());
        Ok(())
    }

    #[test]
    fn admission_rejects_oversized_batches_before_filesystem_work() {
        let mut registry = SourceRegistry::new();
        let paths = std::iter::repeat_n(
            std::path::PathBuf::from("unavailable.txt"),
            MAX_ADMITTED_SOURCES + 1,
        );

        let result = registry.admit_paths(paths);

        assert_eq!(result, Err(AdmissionError::TooManySources));
        assert_eq!(registry.generation(), 0);
        assert!(registry.snapshots().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn admission_preserves_a_symlink_entry_instead_of_following_it() -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let target = directory.path().join("target.txt");
        let link = directory.path().join("link.txt");
        fs::write(&target, b"target")?;
        symlink(&target, &link)?;
        let mut registry = SourceRegistry::new();

        let snapshots = registry.admit_paths([link.clone()])?;

        assert_eq!(registry.path_for(SourceId::new(1)), Some(link.as_path()));
        assert_eq!(snapshots[0].native_name(), "link.txt");
        assert_eq!(
            snapshots[0].fingerprint().map(|value| value.entry_kind()),
            Some(EntryKind::Symlink)
        );
        Ok(())
    }
}
