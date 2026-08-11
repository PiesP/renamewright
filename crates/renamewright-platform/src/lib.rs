#![forbid(unsafe_code)]

mod execution_fs;
mod executor;
mod journal;
mod ledger;
mod recovery;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use renamewright_core::{
    EntryIdentitySignal, EntryKind, OccupiedName, ParentId, SourceFingerprint, SourceId,
    SourceSnapshot, ValidationEnvironment,
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
    MAX_DISCOVERED_JOURNALS, RenameLedger,
};
pub use recovery::{
    PreparedStepDisposition, PreparedStepInspection, RecoveryAction, RecoveryActionError,
    RecoveryActionErrorKind, RecoveryInspectionError, RecoveryInspectionErrorKind,
    RecoveryLocation, RecoveryLocationState, RecoveryObservation, RecoveryReadiness,
    RecoveryTransactionInspection, inspect_prepared_step, inspect_recovery_transaction,
    reconcile_prepared_step, recover_transaction,
};

/// Applying a newly planned rename remains unavailable.
#[must_use]
pub const fn plan_execution_is_enabled() -> bool {
    false
}

/// Startup recovery is available after the recovery safety gates pass.
#[must_use]
pub const fn recovery_execution_is_enabled() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Unavailable(PathBuf),
    MissingFileName(PathBuf),
}

impl Display for AdmissionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
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
    source_ids: BTreeMap<PathBuf, SourceId>,
    parent_ids: BTreeMap<PathBuf, ParentId>,
    next_source_id: u64,
    next_parent_id: u64,
    generation: u64,
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
        let mut candidates = Vec::new();

        for path in paths {
            candidates.push(normalize_entry_path(path)?);
        }

        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        candidates.dedup_by(|left, right| left.0 == right.0);
        let mut changed = false;

        for (path, fingerprint) in candidates {
            if self.source_ids.contains_key(&path) {
                continue;
            }

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

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn path_for(&self, source_id: SourceId) -> Option<&Path> {
        self.paths.get(&source_id).map(PathBuf::as_path)
    }

    #[must_use]
    pub fn validation_environment(&self) -> ValidationEnvironment {
        let stale_sources = self
            .paths
            .iter()
            .filter_map(|(source_id, path)| {
                let snapshot = self.snapshots.get(source_id)?;
                let current = fs::symlink_metadata(path)
                    .ok()
                    .and_then(|metadata| fingerprint_for(&metadata));
                (current.as_ref() != snapshot.fingerprint()).then_some(*source_id)
            })
            .collect::<BTreeSet<_>>();
        let source_paths = self.paths.values().cloned().collect::<BTreeSet<_>>();
        let mut unavailable_parents = BTreeSet::new();
        let mut occupied_names = Vec::new();

        for (parent, parent_id) in &self.parent_ids {
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

        ValidationEnvironment::new(stale_sources, unavailable_parents, occupied_names)
    }
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
    } else {
        return None;
    };
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos());
    Some(SourceFingerprint::new(
        entry_kind,
        entry_identity_signal(metadata),
        metadata.len(),
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

    use super::{SourceRegistry, plan_execution_is_enabled, recovery_execution_is_enabled};

    #[test]
    fn plan_execution_remains_locked_while_recovery_is_available() {
        assert!(!plan_execution_is_enabled());
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
