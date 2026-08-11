use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Display, Formatter};
#[cfg(target_os = "linux")]
use std::path::PathBuf;
use std::path::{Component, Path};

use renamewright_core::{ExecutionIdentity, PlanId, SourceId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionFsErrorKind {
    InvalidName,
    SourceUnavailable,
    UnsupportedEntry,
    UnsupportedPlatform,
    UnsupportedFileSystem,
    StaleIdentity,
    DestinationExists,
    AccessDenied,
    PostRenameIdentityMismatch,
    IoFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionFsError {
    kind: ExecutionFsErrorKind,
    os_code: Option<i32>,
}

impl ExecutionFsError {
    const fn new(kind: ExecutionFsErrorKind, os_code: Option<i32>) -> Self {
        Self { kind, os_code }
    }

    #[must_use]
    pub const fn kind(self) -> ExecutionFsErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn os_code(self) -> Option<i32> {
        self.os_code
    }
}

impl Display for ExecutionFsError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the execution filesystem operation failed ({:?})",
            self.kind
        )
    }
}

impl Error for ExecutionFsError {}

/// Filesystem operations required by the journaled executor.
///
/// Implementations must reject occupied destinations atomically. Native paths
/// remain behind this Rust-owned boundary and must not appear in returned errors.
pub trait ExecutionFileSystem: Send + Sync {
    fn identity(
        &self,
        parent: &Path,
        native_name: &OsStr,
    ) -> Result<ExecutionIdentity, ExecutionFsError>;

    fn rename_no_replace(
        &self,
        parent: &Path,
        source_name: &OsStr,
        target_name: &OsStr,
        expected_identity: ExecutionIdentity,
    ) -> Result<ExecutionIdentity, ExecutionFsError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeExecutionFileSystem;

impl NativeExecutionFileSystem {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Linux-only validation adapter for executor and no-replace integration tests.
///
/// Linux has no handle-based equivalent of the Windows rename contract used by
/// the product. This adapter atomically protects the destination with
/// `RENAME_NOREPLACE`, but its source identity check remains path-based. It must
/// not be treated as the production Windows authority.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Default)]
pub struct LinuxExecutionFileSystem;

#[cfg(target_os = "linux")]
impl LinuxExecutionFileSystem {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

pub fn temporary_name(
    plan_id: PlanId,
    source_id: SourceId,
    attempt: u32,
) -> Result<OsString, ExecutionFsError> {
    let name = OsString::from(format!(
        ".renamewright-{:016x}-{:016x}-{attempt:04x}.tmp",
        plan_id.value(),
        source_id.value()
    ));
    validate_component(&name)?;
    Ok(name)
}

#[cfg(target_os = "linux")]
fn entry_path(parent: &Path, native_name: &OsStr) -> Result<PathBuf, ExecutionFsError> {
    validate_component(native_name)?;
    Ok(parent.join(native_name))
}

fn validate_component(native_name: &OsStr) -> Result<(), ExecutionFsError> {
    let path = Path::new(native_name);
    let mut components = path.components();
    let is_single_normal =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if is_single_normal {
        Ok(())
    } else {
        Err(ExecutionFsError::new(
            ExecutionFsErrorKind::InvalidName,
            None,
        ))
    }
}

#[cfg(target_os = "linux")]
impl ExecutionFileSystem for LinuxExecutionFileSystem {
    fn identity(
        &self,
        parent: &Path,
        native_name: &OsStr,
    ) -> Result<ExecutionIdentity, ExecutionFsError> {
        use std::os::unix::fs::MetadataExt;

        let path = entry_path(parent, native_name)?;
        let metadata = std::fs::symlink_metadata(path).map_err(map_std_io_error)?;
        if !metadata.file_type().is_file() {
            return Err(ExecutionFsError::new(
                ExecutionFsErrorKind::UnsupportedEntry,
                None,
            ));
        }
        let mut file_id = [0_u8; 16];
        file_id[..8].copy_from_slice(&metadata.ino().to_le_bytes());
        Ok(ExecutionIdentity::new(metadata.dev(), file_id))
    }

    fn rename_no_replace(
        &self,
        parent: &Path,
        source_name: &OsStr,
        target_name: &OsStr,
        expected_identity: ExecutionIdentity,
    ) -> Result<ExecutionIdentity, ExecutionFsError> {
        use rustix::fs::{CWD, RenameFlags, renameat_with};

        if source_name == target_name {
            return Err(ExecutionFsError::new(
                ExecutionFsErrorKind::InvalidName,
                None,
            ));
        }
        let source = entry_path(parent, source_name)?;
        let target = entry_path(parent, target_name)?;
        let current_identity = self.identity(parent, source_name)?;
        if current_identity != expected_identity {
            return Err(ExecutionFsError::new(
                ExecutionFsErrorKind::StaleIdentity,
                None,
            ));
        }

        renameat_with(CWD, &source, CWD, &target, RenameFlags::NOREPLACE)
            .map_err(map_rustix_error)?;
        let observed_identity = self.identity(parent, target_name)?;
        if observed_identity != expected_identity {
            return Err(ExecutionFsError::new(
                ExecutionFsErrorKind::PostRenameIdentityMismatch,
                None,
            ));
        }
        Ok(observed_identity)
    }
}

#[cfg(target_os = "linux")]
fn map_rustix_error(error: rustix::io::Errno) -> ExecutionFsError {
    use rustix::io::Errno;

    let kind = if matches!(error, Errno::EXIST | Errno::NOTEMPTY) {
        ExecutionFsErrorKind::DestinationExists
    } else if matches!(error, Errno::ACCESS | Errno::PERM) {
        ExecutionFsErrorKind::AccessDenied
    } else if error == Errno::NOENT {
        ExecutionFsErrorKind::SourceUnavailable
    } else if matches!(
        error,
        Errno::XDEV | Errno::NOSYS | Errno::NOTSUP | Errno::INVAL
    ) {
        ExecutionFsErrorKind::UnsupportedFileSystem
    } else {
        ExecutionFsErrorKind::IoFailure
    };
    ExecutionFsError::new(kind, Some(error.raw_os_error()))
}

#[cfg(target_os = "linux")]
fn map_std_io_error(error: std::io::Error) -> ExecutionFsError {
    let kind = match error.kind() {
        std::io::ErrorKind::NotFound => ExecutionFsErrorKind::SourceUnavailable,
        std::io::ErrorKind::PermissionDenied => ExecutionFsErrorKind::AccessDenied,
        _ => ExecutionFsErrorKind::IoFailure,
    };
    ExecutionFsError::new(kind, error.raw_os_error())
}

impl ExecutionFileSystem for NativeExecutionFileSystem {
    fn identity(
        &self,
        _parent: &Path,
        native_name: &OsStr,
    ) -> Result<ExecutionIdentity, ExecutionFsError> {
        validate_component(native_name)?;
        Err(ExecutionFsError::new(
            ExecutionFsErrorKind::UnsupportedPlatform,
            None,
        ))
    }

    fn rename_no_replace(
        &self,
        _parent: &Path,
        source_name: &OsStr,
        target_name: &OsStr,
        _expected_identity: ExecutionIdentity,
    ) -> Result<ExecutionIdentity, ExecutionFsError> {
        validate_component(source_name)?;
        validate_component(target_name)?;
        Err(ExecutionFsError::new(
            ExecutionFsErrorKind::UnsupportedPlatform,
            None,
        ))
    }
}
