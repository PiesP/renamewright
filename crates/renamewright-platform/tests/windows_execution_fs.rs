#![cfg(windows)]

use std::ffi::OsStr;
use std::fs;

use renamewright_platform::{ExecutionFileSystem, ExecutionFsErrorKind, NativeExecutionFileSystem};

#[test]
fn native_adapter_uses_execution_grade_identity_for_rename()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("source.txt"), b"source")?;
    let filesystem = NativeExecutionFileSystem::new();
    let identity = filesystem.identity(directory.path(), OsStr::new("source.txt"))?;

    let observed = filesystem.rename_no_replace(
        directory.path(),
        OsStr::new("source.txt"),
        OsStr::new("destination.txt"),
        identity,
    )?;

    assert_eq!(observed, identity);
    assert_eq!(
        filesystem.identity(directory.path(), OsStr::new("destination.txt"))?,
        identity
    );
    Ok(())
}

#[test]
fn native_adapter_renames_a_directory_with_the_same_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source-directory");
    fs::create_dir(&source)?;
    fs::write(source.join("child.txt"), b"child")?;
    let filesystem = NativeExecutionFileSystem::new();
    let identity = filesystem.identity(root.path(), OsStr::new("source-directory"))?;

    let observed = filesystem.rename_no_replace(
        root.path(),
        OsStr::new("source-directory"),
        OsStr::new("target-directory"),
        identity,
    )?;

    assert_eq!(observed, identity);
    assert_eq!(
        fs::read(root.path().join("target-directory").join("child.txt"))?,
        b"child"
    );
    Ok(())
}

#[test]
fn native_adapter_maps_collision_and_stale_identity_without_mutation()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source.txt");
    let destination = directory.path().join("destination.txt");
    fs::write(&source, b"source")?;
    fs::write(&destination, b"occupant")?;
    let filesystem = NativeExecutionFileSystem::new();
    let identity = filesystem.identity(directory.path(), OsStr::new("source.txt"))?;

    let collision = filesystem
        .rename_no_replace(
            directory.path(),
            OsStr::new("source.txt"),
            OsStr::new("destination.txt"),
            identity,
        )
        .err()
        .ok_or("occupied destination was replaced")?;
    assert_eq!(collision.kind(), ExecutionFsErrorKind::DestinationExists);
    assert_eq!(fs::read(&source)?, b"source");
    assert_eq!(fs::read(&destination)?, b"occupant");

    let stale = renamewright_core::ExecutionIdentity::new(
        identity.volume_serial_number().wrapping_add(1),
        identity.file_id(),
    );
    let stale_error = filesystem
        .rename_no_replace(
            directory.path(),
            OsStr::new("source.txt"),
            OsStr::new("free-name.txt"),
            stale,
        )
        .err()
        .ok_or("stale identity was accepted")?;
    assert_eq!(stale_error.kind(), ExecutionFsErrorKind::StaleIdentity);
    assert!(source.exists());
    assert!(!directory.path().join("free-name.txt").exists());
    Ok(())
}
