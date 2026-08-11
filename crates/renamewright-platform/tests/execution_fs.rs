#![cfg(target_os = "linux")]

use std::ffi::OsStr;
use std::fs;

use renamewright_core::{PlanId, SourceId};
use renamewright_platform::{
    ExecutionFileSystem, ExecutionFsErrorKind, LinuxExecutionFileSystem, NativeExecutionFileSystem,
    temporary_name,
};
use tempfile::tempdir;

#[test]
fn no_replace_rename_preserves_identity_and_contents() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    fs::write(directory.path().join("source.txt"), b"source contents")?;
    let filesystem = LinuxExecutionFileSystem::new();
    let identity = filesystem.identity(directory.path(), OsStr::new("source.txt"))?;

    let observed = filesystem.rename_no_replace(
        directory.path(),
        OsStr::new("source.txt"),
        OsStr::new("target.txt"),
        identity,
    )?;

    assert_eq!(observed, identity);
    assert!(!directory.path().join("source.txt").exists());
    assert_eq!(
        fs::read(directory.path().join("target.txt"))?,
        b"source contents"
    );
    Ok(())
}

#[test]
fn destination_race_never_replaces_the_occupant() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    fs::write(directory.path().join("source.txt"), b"source contents")?;
    fs::write(directory.path().join("target.txt"), b"unrelated occupant")?;
    let filesystem = LinuxExecutionFileSystem::new();
    let identity = filesystem.identity(directory.path(), OsStr::new("source.txt"))?;

    let error = filesystem
        .rename_no_replace(
            directory.path(),
            OsStr::new("source.txt"),
            OsStr::new("target.txt"),
            identity,
        )
        .err()
        .ok_or("occupied destination was replaced")?;

    assert_eq!(error.kind(), ExecutionFsErrorKind::DestinationExists);
    assert_eq!(
        fs::read(directory.path().join("source.txt"))?,
        b"source contents"
    );
    assert_eq!(
        fs::read(directory.path().join("target.txt"))?,
        b"unrelated occupant"
    );
    assert!(
        !error
            .to_string()
            .contains(directory.path().to_string_lossy().as_ref())
    );
    Ok(())
}

#[test]
fn stale_identity_blocks_rename_before_mutation() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let source = directory.path().join("source.txt");
    fs::write(&source, b"first entry")?;
    let filesystem = LinuxExecutionFileSystem::new();
    let stale_identity = filesystem.identity(directory.path(), OsStr::new("source.txt"))?;
    fs::hard_link(
        &source,
        directory.path().join("original-entry-kept-alive.txt"),
    )?;
    fs::remove_file(&source)?;
    fs::write(&source, b"replacement entry")?;

    let error = filesystem
        .rename_no_replace(
            directory.path(),
            OsStr::new("source.txt"),
            OsStr::new("target.txt"),
            stale_identity,
        )
        .err()
        .ok_or("stale source was renamed")?;

    assert_eq!(error.kind(), ExecutionFsErrorKind::StaleIdentity);
    assert_eq!(fs::read(source)?, b"replacement entry");
    assert!(!directory.path().join("target.txt").exists());
    Ok(())
}

#[test]
fn names_must_be_single_native_components() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    fs::write(directory.path().join("source.txt"), b"source")?;
    let filesystem = LinuxExecutionFileSystem::new();
    let identity = filesystem.identity(directory.path(), OsStr::new("source.txt"))?;

    for invalid in ["", ".", "..", "nested/target.txt", "/absolute.txt"] {
        let error = filesystem
            .rename_no_replace(
                directory.path(),
                OsStr::new("source.txt"),
                OsStr::new(invalid),
                identity,
            )
            .err()
            .ok_or("invalid target component was accepted")?;
        assert_eq!(error.kind(), ExecutionFsErrorKind::InvalidName);
    }
    assert!(directory.path().join("source.txt").exists());
    Ok(())
}

#[test]
fn temporary_names_are_deterministic_valid_components() -> Result<(), Box<dyn std::error::Error>> {
    let first = temporary_name(PlanId::new(11), SourceId::new(13), 0)?;
    let retry = temporary_name(PlanId::new(11), SourceId::new(13), 1)?;

    assert_eq!(
        first,
        ".renamewright-000000000000000b-000000000000000d-0000.tmp"
    );
    assert_ne!(first, retry);
    assert!(!first.to_string_lossy().contains(['/', '\\']));
    Ok(())
}

#[test]
fn production_native_adapter_remains_disabled_without_windows_backend()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    fs::write(directory.path().join("source.txt"), b"source")?;

    let error = NativeExecutionFileSystem::new()
        .identity(directory.path(), OsStr::new("source.txt"))
        .err()
        .ok_or("incomplete native adapter was enabled")?;

    assert_eq!(error.kind(), ExecutionFsErrorKind::UnsupportedPlatform);
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink_sources_are_rejected_without_following_them() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempdir()?;
    fs::write(directory.path().join("target.txt"), b"target")?;
    symlink("target.txt", directory.path().join("link.txt"))?;
    let filesystem = LinuxExecutionFileSystem::new();

    let error = filesystem
        .identity(directory.path(), OsStr::new("link.txt"))
        .err()
        .ok_or("symlink source was accepted")?;

    assert_eq!(error.kind(), ExecutionFsErrorKind::UnsupportedEntry);
    assert!(directory.path().join("link.txt").exists());
    assert_eq!(fs::read(directory.path().join("target.txt"))?, b"target");
    Ok(())
}
