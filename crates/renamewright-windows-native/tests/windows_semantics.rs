#![cfg(windows)]

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::windows::fs::{OpenOptionsExt, symlink_dir, symlink_file};
use std::sync::{Arc, Barrier};
use std::thread;

use renamewright_windows_native::{EntryHandle, ParentHandle, file_identity, rename_noreplace};
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

#[test]
fn identity_survives_handle_rename_and_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("source.txt");
    fs::write(&source_path, b"source")?;
    let parent = ParentHandle::open(directory.path())?;
    let source = EntryHandle::open_relative(&parent, OsStr::new("source.txt"))?;
    let before = file_identity(source.as_handle())?;

    rename_noreplace(
        source.as_handle(),
        parent.as_handle(),
        OsStr::new("renamed.txt"),
    )?;

    assert_eq!(file_identity(source.as_handle())?, before);
    let reopened = EntryHandle::open_relative(&parent, OsStr::new("renamed.txt"))?;
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
    let parent = ParentHandle::open(directory.path())?;
    let source = EntryHandle::open_relative(&parent, OsStr::new("source.txt"))?;
    let destination = EntryHandle::open_relative(&parent, OsStr::new("destination.txt"))?;
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
    let destination_after = EntryHandle::open_relative(&parent, OsStr::new("destination.txt"))?;
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
    let first_parent = ParentHandle::open(directory.path())?;
    let second_parent = ParentHandle::open(directory.path())?;
    let first = EntryHandle::open_relative(&first_parent, OsStr::new("first.txt"))?;
    let second = EntryHandle::open_relative(&second_parent, OsStr::new("second.txt"))?;
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
    let parent = ParentHandle::open(directory.path())?;
    let source = EntryHandle::open_relative(&parent, OsStr::new("source.txt"))?;
    let linked = EntryHandle::open_relative(&parent, OsStr::new("linked.txt"))?;
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
    let parent = ParentHandle::open(directory.path())?;
    let source = EntryHandle::open_relative(&parent, OsStr::new("link.txt"))?;
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
    let reopened = EntryHandle::open_relative(&parent, OsStr::new("renamed-link.txt"))?;
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
    let parent = ParentHandle::open(directory.path())?;
    let source = EntryHandle::open_relative(&parent, OsStr::new("directory-link"))?;

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
fn reparse_parent_is_rejected_instead_of_followed() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("actual-parent");
    let redirect = directory.path().join("redirect-parent");
    fs::create_dir(&target)?;
    fs::write(target.join("source.txt"), b"source")?;
    symlink_dir("actual-parent", &redirect)?;
    let error = ParentHandle::open(&redirect)
        .err()
        .ok_or("a reparse parent was followed")?;

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read(target.join("source.txt"))?, b"source");
    assert!(!target.join("renamed.txt").exists());
    assert!(redirect.exists());
    Ok(())
}

#[test]
fn retained_parent_handle_prevents_directory_name_swap_redirection()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let selected = root.path().join("selected");
    let retained = root.path().join("retained");
    let replacement = root.path().join("replacement");
    fs::create_dir(&selected)?;
    fs::create_dir(&replacement)?;
    fs::write(selected.join("source.txt"), b"authorized")?;
    fs::write(replacement.join("source.txt"), b"replacement")?;
    let parent = ParentHandle::open(&selected)?;

    fs::rename(&selected, &retained)?;
    fs::rename(&replacement, &selected)?;
    let source = EntryHandle::open_relative(&parent, OsStr::new("source.txt"))?;
    rename_noreplace(
        source.as_handle(),
        parent.as_handle(),
        OsStr::new("renamed.txt"),
    )?;

    assert_eq!(fs::read(retained.join("renamed.txt"))?, b"authorized");
    assert!(!retained.join("source.txt").exists());
    assert_eq!(fs::read(selected.join("source.txt"))?, b"replacement");
    assert!(!selected.join("renamed.txt").exists());
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
    let parent = ParentHandle::open(directory.path())?;
    let source_directory = EntryHandle::open_relative(&parent, OsStr::new("source-directory"))?;
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
    let unicode = EntryHandle::open_relative(&parent, OsStr::new("source-unicode.txt"))?;
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
    let case_entry = EntryHandle::open_relative(&parent, OsStr::new("CaseName.txt"))?;
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
    let parent = ParentHandle::open(directory.path())?;
    let source = EntryHandle::open_relative(&parent, OsStr::new("source.txt"))?;
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
    let second = EntryHandle::open_relative(&parent, OsStr::new("second-source.txt"))?;
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
    let parent = ParentHandle::open(directory.path())?;

    let error = EntryHandle::open_relative(&parent, OsStr::new("source.txt"))
        .err()
        .ok_or("delete-denying handle did not block source admission")?;

    assert_eq!(error.raw_os_error(), Some(32));
    drop(blocker);
    assert!(EntryHandle::open_relative(&parent, OsStr::new("source.txt")).is_ok());
    Ok(())
}

#[test]
fn cross_volume_rename_fails_without_changing_either_side() -> Result<(), Box<dyn std::error::Error>>
{
    let source_directory = tempfile::tempdir()?;
    let source_path = source_directory.path().join("source.txt");
    fs::write(&source_path, b"source")?;
    let source_parent = ParentHandle::open(source_directory.path())?;
    let source = EntryHandle::open_relative(&source_parent, OsStr::new("source.txt"))?;
    let source_identity = file_identity(source.as_handle())?;

    let Ok(destination_directory) = tempfile::Builder::new()
        .prefix("renamewright-cross-volume-")
        .tempdir_in(std::env::current_dir()?)
    else {
        return Ok(());
    };
    let marker_path = destination_directory.path().join("marker.txt");
    fs::write(&marker_path, b"marker")?;
    let destination_parent = ParentHandle::open(destination_directory.path())?;
    let marker = EntryHandle::open_relative(&destination_parent, OsStr::new("marker.txt"))?;
    let marker_identity = file_identity(marker.as_handle())?;
    if source_identity.volume_serial_number() == marker_identity.volume_serial_number() {
        return Ok(());
    }

    let error = rename_noreplace(
        source.as_handle(),
        destination_parent.as_handle(),
        OsStr::new("destination.txt"),
    )
    .err()
    .ok_or("cross-volume rename unexpectedly succeeded")?;

    assert!(error.raw_os_error().is_some());
    assert_eq!(fs::read(&source_path)?, b"source");
    assert!(
        !destination_directory
            .path()
            .join("destination.txt")
            .exists()
    );
    assert_eq!(fs::read(marker_path)?, b"marker");
    Ok(())
}

#[test]
fn invalid_leaf_names_are_rejected_before_ffi() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("source.txt");
    fs::write(&source_path, b"source")?;
    let parent = ParentHandle::open(directory.path())?;
    let source = EntryHandle::open_relative(&parent, OsStr::new("source.txt"))?;

    for invalid in ["", ".", "..", "nested\\name.txt", "stream:name"] {
        let error = rename_noreplace(source.as_handle(), parent.as_handle(), OsStr::new(invalid))
            .err()
            .ok_or("invalid leaf name reached Win32")?;
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    let relative_parent_error = ParentHandle::open(std::path::Path::new("relative-parent"))
        .err()
        .ok_or("relative parent was accepted")?;
    assert_eq!(relative_parent_error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read(source_path)?, b"source");
    Ok(())
}
