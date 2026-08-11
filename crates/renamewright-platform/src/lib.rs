#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use renamewright_core::{ParentId, SourceId, SourceSnapshot};

/// Filesystem mutation remains unavailable during the planning milestone.
#[must_use]
pub const fn mutation_is_enabled() -> bool {
    false
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Unavailable(PathBuf),
    MissingFileName(PathBuf),
}

impl Display for AdmissionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(path) => write!(
                formatter,
                "the selected source is not an available file: {}",
                path.display()
            ),
            Self::MissingFileName(path) => {
                write!(
                    formatter,
                    "the selected source has no file name: {}",
                    path.display()
                )
            }
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
            let canonical = path
                .canonicalize()
                .map_err(|_| AdmissionError::Unavailable(path.clone()))?;
            if !canonical.is_file() {
                return Err(AdmissionError::Unavailable(path));
            }
            if canonical.file_name().is_none() {
                return Err(AdmissionError::MissingFileName(canonical));
            }
            candidates.push(canonical);
        }

        candidates.sort();
        candidates.dedup();
        let mut changed = false;

        for path in candidates {
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
            let snapshot = SourceSnapshot::new(source_id, parent_id, name.to_os_string());
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
}

#[cfg(test)]
mod tests {
    use super::mutation_is_enabled;

    #[test]
    fn planning_milestone_is_read_only() {
        assert!(!mutation_is_enabled());
    }
}
