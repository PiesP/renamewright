#![cfg(windows)]

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::windows::fs::{OpenOptionsExt, symlink_dir, symlink_file};
use std::sync::{Arc, Barrier};
use std::thread;

use renamewright_windows_native::{DirectoryHandle, EntryHandle, file_identity, rename_noreplace};
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

#[test]
fn identity_survives_handle_rename_and_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("source.txt");
    fs::write(&source_path, b"source")?;
    let parent = DirectoryHandle::open(directory.path())?;
    let source = EntryHandle::open_final_component(&source_path)?;
    let before = file_identity(source.as_handle())?;

    rename_noreplace(
        source.as_handle(),
        parent.as_handle(),
        OsStr::new("renamed.txt"),
    )?;

    assert_eq!(file_identity(source.as_handle())?, before);
    let reopened = EntryHandle::open_final_component(&directory.path().join("renamed.txt"))?;
    assert_eq!(file_identity(reopened.as_handle())?, before);
    assert!(!source_path.exists());
    Ok(())
}

#[test]
fn existing_destination_is_never_replaced() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("source.txt");
    let destination_path = directory.path().join("destination.txt");
    fs::write(&source_path, b"source")?;
    fs::write(&destination_path, b"occupant")?;
    let parent = DirectoryHandle::open(directory.path())?;
    let source = EntryHandle::open_final_component(&source_path)?;
    let destination = EntryHandle::open_final_component(&destination_path)?;
    let destination_identity = file_identity(destination.as_handle())?;

    let error = rename_noreplace(
        source.as_handle(),
        parent.as_handle(),
        OsStr::new("destination.txt"),
    )
    .err()
    .ok_or("occupied destination was replaced")?;

    assert!(matches!(
        error.kind(),
        io::ErrorKind::AlreadyExists | io::ErrorKind::PermissionDenied
    ));
    assert_eq!(fs::read(source_path)?, b"source");
    assert_eq!(fs::read(&destination_path)?, b"occupant");
    let destination_after = EntryHandle::open_final_component(&destination_path)?;
    assert_eq!(
        file_identity(destination_after.as_handle())?,
        destination_identity
    );
    Ok(())
}

#[test]
fn synchronized_destination_race_has_one_winner() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let first_path = directory.path().join("first.txt");
    let second_path = directory.path().join("second.txt");
    fs::write(&first_path, b"first")?;
    fs::write(&second_path, b"second")?;
    let first = EntryHandle::open_final_component(&first_path)?;
    let second = EntryHandle::open_final_component(&second_path)?;
    let first_parent = DirectoryHandle::open(directory.path())?;
    let second_parent = DirectoryHandle::open(directory.path())?;
    let barrier = Arc::new(Barrier::new(2));
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);

    let first_thread = thread::spawn(move || {
        first_barrier.wait();
        rename_noreplace(
            first.as_handle(),
            first_parent.as_handle(),
            OsStr::new("winner.txt"),
        )
    });
    let second_thread = thread::spawn(move || {
        second_barrier.wait();
        rename_noreplace(
            second.as_handle(),
            second_parent.as_handle(),
            OsStr::new("winner.txt"),
        )
    });
    let first_result = first_thread
        .join()
        .map_err(|_| io::Error::other("first rename thread failed"))?;
    let second_result = second_thread
        .join()
        .map_err(|_| io::Error::other("second rename thread failed"))?;

    assert_ne!(first_result.is_ok(), second_result.is_ok());
    assert!(directory.path().join("winner.txt").exists());
    if first_result.is_ok() {
        assert!(!first_path.exists());
        assert_eq!(fs::read(second_path)?, b"second");
    } else {
        assert_eq!(fs::read(first_path)?, b"first");
        assert!(!second_path.exists());
    }
    Ok(())
}

#[test]
fn hard_links_share_identity_but_existing_link_still_blocks_rename()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("source.txt");
    let linked_path = directory.path().join("linked.txt");
    fs::write(&source_path, b"shared")?;
    fs::hard_link(&source_path, &linked_path)?;
    let source = EntryHandle::open_final_component(&source_path)?;
    let linked = EntryHandle::open_final_component(&linked_path)?;
    let parent = DirectoryHandle::open(directory.path())?;
    assert_eq!(
        file_identity(source.as_handle())?,
        file_identity(linked.as_handle())?
    );

    let error = rename_noreplace(
        source.as_handle(),
        parent.as_handle(),
        OsStr::new("linked.txt"),
    )
    .err()
    .ok_or("existing hard link was replaced")?;

    assert!(matches!(
        error.kind(),
        io::ErrorKind::AlreadyExists | io::ErrorKind::PermissionDenied
    ));
    assert_eq!(fs::read(source_path)?, b"shared");
    assert_eq!(fs::read(linked_path)?, b"shared");
    Ok(())
}

#[test]
fn final_file_symlink_is_renamed_without_touching_target() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("target.txt");
    let link = directory.path().join("link.txt");
    fs::write(&target, b"target")?;
    symlink_file("target.txt", &link)?;
    let parent = DirectoryHandle::open(directory.path())?;
    let source = EntryHandle::open_final_component(&link)?;
    let identity = file_identity(source.as_handle())?;

    rename_noreplace(
        source.as_handle(),
        parent.as_handle(),
        OsStr::new("renamed-link.txt"),
    )?;

    assert_eq!(fs::read(&target)?, b"target");
    assert!(!link.exists());
    let renamed = directory.path().join("renamed-link.txt");
    assert_eq!(
        fs::read_link(&renamed)?,
        std::path::PathBuf::from("target.txt")
    );
    let reopened = EntryHandle::open_final_component(&renamed)?;
    assert_eq!(file_identity(reopened.as_handle())?, identity);
    Ok(())
}

#[test]
fn final_directory_symlink_is_renamed_without_touching_target()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("target-directory");
    let link = directory.path().join("directory-link");
    fs::create_dir(&target)?;
    fs::write(target.join("child.txt"), b"child")?;
    symlink_dir("target-directory", &link)?;
    let parent = DirectoryHandle::open(directory.path())?;
    let source = EntryHandle::open_final_component(&link)?;

    rename_noreplace(
        source.as_handle(),
        parent.as_handle(),
        OsStr::new("renamed-directory-link"),
    )?;

    assert_eq!(fs::read(target.join("child.txt"))?, b"child");
    assert!(!link.exists());
    assert_eq!(
        fs::read_link(directory.path().join("renamed-directory-link"))?,
        std::path::PathBuf::from("target-directory")
    );
    Ok(())
}

#[test]
fn intermediate_directory_symlink_is_currently_followed() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("actual-parent");
    let redirect = directory.path().join("redirect-parent");
    fs::create_dir(&target)?;
    fs::write(target.join("source.txt"), b"source")?;
    symlink_dir("actual-parent", &redirect)?;
    let parent = DirectoryHandle::open(&redirect)?;
    let source = EntryHandle::open_final_component(&redirect.join("source.txt"))?;

    rename_noreplace(
        source.as_handle(),
        parent.as_handle(),
        OsStr::new("renamed.txt"),
    )?;

    assert_eq!(fs::read(target.join("renamed.txt"))?, b"source");
    assert!(redirect.exists());
    Ok(())
}

#[test]
fn directories_unicode_and_case_only_changes_use_the_same_primitive()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir(directory.path().join("source-directory"))?;
    fs::write(
        directory.path().join("source-directory").join("child.txt"),
        b"child",
    )?;
    let parent = DirectoryHandle::open(directory.path())?;
    let source_directory =
        EntryHandle::open_final_component(&directory.path().join("source-directory"))?;
    rename_noreplace(
        source_directory.as_handle(),
        parent.as_handle(),
        OsStr::new("renamed-directory"),
    )?;
    assert_eq!(
        fs::read(directory.path().join("renamed-directory").join("child.txt"))?,
        b"child"
    );

    let unicode_source = directory.path().join("source-unicode.txt");
    fs::write(&unicode_source, b"unicode")?;
    let unicode = EntryHandle::open_final_component(&unicode_source)?;
    rename_noreplace(
        unicode.as_handle(),
        parent.as_handle(),
        OsStr::new("renamed-🚀.txt"),
    )?;
    assert_eq!(
        fs::read(directory.path().join("renamed-🚀.txt"))?,
        b"unicode"
    );

    let case_source = directory.path().join("CaseName.txt");
    fs::write(&case_source, b"case")?;
    let case_entry = EntryHandle::open_final_component(&case_source)?;
    rename_noreplace(
        case_entry.as_handle(),
        parent.as_handle(),
        OsStr::new("case-temporary.txt"),
    )?;
    rename_noreplace(
        case_entry.as_handle(),
        parent.as_handle(),
        OsStr::new("casename.txt"),
    )?;
    assert_eq!(fs::read(directory.path().join("casename.txt"))?, b"case");
    Ok(())
}

#[test]
fn read_only_source_can_move_but_read_only_destination_is_preserved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("source.txt");
    fs::write(&source_path, b"source")?;
    let mut source_permissions = fs::metadata(&source_path)?.permissions();
    source_permissions.set_readonly(true);
    fs::set_permissions(&source_path, source_permissions)?;
    let parent = DirectoryHandle::open(directory.path())?;
    let source = EntryHandle::open_final_component(&source_path)?;
    rename_noreplace(
        source.as_handle(),
        parent.as_handle(),
        OsStr::new("moved.txt"),
    )?;
    assert_eq!(fs::read(directory.path().join("moved.txt"))?, b"source");

    let second_source = directory.path().join("second-source.txt");
    let destination = directory.path().join("destination.txt");
    fs::write(&second_source, b"second")?;
    fs::write(&destination, b"destination")?;
    let mut destination_permissions = fs::metadata(&destination)?.permissions();
    destination_permissions.set_readonly(true);
    fs::set_permissions(&destination, destination_permissions)?;
    let second = EntryHandle::open_final_component(&second_source)?;
    let error = rename_noreplace(
        second.as_handle(),
        parent.as_handle(),
        OsStr::new("destination.txt"),
    )
    .err()
    .ok_or("read-only destination was replaced")?;
    assert!(matches!(
        error.kind(),
        io::ErrorKind::AlreadyExists | io::ErrorKind::PermissionDenied
    ));
    assert_eq!(fs::read(second_source)?, b"second");
    assert_eq!(fs::read(destination)?, b"destination");
    Ok(())
}

#[test]
fn sharing_violation_prevents_opening_a_rename_handle() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("source.txt");
    fs::write(&source_path, b"source")?;
    let blocker = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&source_path)?;

    let error = EntryHandle::open_final_component(&source_path)
        .err()
        .ok_or("delete-denying handle did not block source admission")?;

    assert_eq!(error.raw_os_error(), Some(32));
    drop(blocker);
    assert!(EntryHandle::open_final_component(&source_path).is_ok());
    Ok(())
}

#[test]
fn invalid_leaf_names_are_rejected_before_ffi() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("source.txt");
    fs::write(&source_path, b"source")?;
    let parent = DirectoryHandle::open(directory.path())?;
    let source = EntryHandle::open_final_component(&source_path)?;

    for invalid in ["", ".", "..", "nested\\name.txt", "stream:name"] {
        let error = rename_noreplace(source.as_handle(), parent.as_handle(), OsStr::new(invalid))
            .err()
            .ok_or("invalid leaf name reached Win32")?;
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    assert_eq!(fs::read(source_path)?, b"source");
    Ok(())
}
