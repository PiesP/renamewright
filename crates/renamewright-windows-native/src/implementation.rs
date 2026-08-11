use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle};
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
    FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FileIdInfo, FileRenameInfoEx, GetFileInformationByHandleEx, SetFileInformationByHandle,
};

const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    volume_serial_number: u64,
    file_id: [u8; 16],
}

impl FileIdentity {
    #[must_use]
    pub const fn volume_serial_number(self) -> u64 {
        self.volume_serial_number
    }

    #[must_use]
    pub const fn file_id(self) -> [u8; 16] {
        self.file_id
    }
}

#[derive(Debug)]
pub struct EntryHandle {
    file: File,
}

impl EntryHandle {
    pub fn open_final_component(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .access_mode(DELETE | FILE_READ_ATTRIBUTES)
            .share_mode(SHARE_ALL)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        Ok(Self { file })
    }

    #[must_use]
    pub fn as_handle(&self) -> BorrowedHandle<'_> {
        self.file.as_handle()
    }
}

pub fn file_identity(source: BorrowedHandle<'_>) -> io::Result<FileIdentity> {
    let mut info = FILE_ID_INFO::default();
    let buffer_size = u32::try_from(size_of::<FILE_ID_INFO>())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "identity buffer is too large"))?;

    // SAFETY: `source` is borrowed for the full call and is not owned or closed
    // by Win32. `info` is a writable, correctly aligned `FILE_ID_INFO`, and the
    // supplied byte count is exactly its initialized allocation size. The API
    // does not retain the pointer after returning.
    // The generic Semgrep unsafe-usage rule is suppressed only at this audited
    // FFI boundary; the safety proof and crate-level unsafe lints remain active.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let succeeded = unsafe {
        GetFileInformationByHandleEx(
            source.as_raw_handle(),
            FileIdInfo,
            ptr::from_mut(&mut info).cast(),
            buffer_size,
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(FileIdentity {
        volume_serial_number: info.VolumeSerialNumber,
        file_id: info.FileId.Identifier,
    })
}

pub fn rename_noreplace(
    source: BorrowedHandle<'_>,
    destination_parent: &Path,
    destination_name: &OsStr,
) -> io::Result<()> {
    let destination = destination_path(destination_parent, destination_name)?;
    reject_existing_destination(&destination)?;
    let encoded_name = encode_destination(&destination)?;
    let name_bytes = encoded_name
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "destination name is too large")
        })?;
    let file_name_length = u32::try_from(name_bytes).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination name is too large")
    })?;
    let terminated_name_bytes = name_bytes.checked_add(size_of::<u16>()).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination name is too large")
    })?;
    let buffer_bytes = offset_of!(FILE_RENAME_INFO, FileName)
        .checked_add(terminated_name_bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename buffer is too large"))?;
    let buffer_size = u32::try_from(buffer_bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "rename buffer is too large"))?;
    let element_count = buffer_bytes.div_ceil(size_of::<FILE_RENAME_INFO>());
    let mut buffer = vec![FILE_RENAME_INFO::default(); element_count];
    let header = &mut buffer[0];
    // Zero flags preserve ordinary no-replace semantics.
    header.Anonymous.Flags = 0;
    header.RootDirectory = ptr::null_mut();
    header.FileNameLength = file_name_length;

    // SAFETY: `buffer` is a zero-initialized `Vec<FILE_RENAME_INFO>`, so its
    // allocation has the required header alignment. `element_count` covers
    // `buffer_bytes`, including a trailing UTF-16 NUL. `FileName` begins at the
    // standard-layout offset used above, and `encoded_name.len()` initialized
    // u16 values fit before that NUL. The vector does not move during the copy.
    // The generic Semgrep unsafe-usage rule is suppressed only at this audited
    // buffer boundary; the safety proof and crate-level unsafe lints remain active.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    unsafe {
        let file_name = buffer
            .as_mut_ptr()
            .cast::<u8>()
            .add(offset_of!(FILE_RENAME_INFO, FileName))
            .cast::<u16>();
        ptr::copy_nonoverlapping(encoded_name.as_ptr(), file_name, encoded_name.len());
    }

    // SAFETY: `source` stays borrowed for the call.
    // `buffer` is correctly aligned, fully initialized for `buffer_size` bytes,
    // has zero rename flags, a null root-directory handle, a checked byte-length
    // field excluding the trailing NUL, and a matching absolute UTF-16 path.
    // Win32 consumes the buffer synchronously and does not retain its pointer.
    // The generic Semgrep unsafe-usage rule is suppressed only at this audited
    // FFI boundary; the safety proof and crate-level unsafe lints remain active.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let succeeded = unsafe {
        SetFileInformationByHandle(
            source.as_raw_handle(),
            FileRenameInfoEx,
            buffer.as_ptr().cast(),
            buffer_size,
        )
    };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn encode_leaf_name(name: &OsStr) -> io::Result<Vec<u16>> {
    let encoded = name.encode_wide().collect::<Vec<_>>();
    let invalid = encoded.is_empty()
        || encoded == [u16::from(b'.')]
        || encoded == [u16::from(b'.'), u16::from(b'.')]
        || encoded.iter().any(|unit| {
            *unit == 0
                || *unit == u16::from(b'/')
                || *unit == u16::from(b'\\')
                || *unit == u16::from(b':')
        });
    if invalid {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination name must be one native leaf component",
        ))
    } else {
        Ok(encoded)
    }
}

fn destination_path(parent: &Path, name: &OsStr) -> io::Result<std::path::PathBuf> {
    let _ = encode_leaf_name(name)?;
    if !parent.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination parent must be absolute",
        ));
    }
    Ok(parent.join(name))
}

fn reject_existing_destination(destination: &Path) -> io::Result<()> {
    // Windows can accept a zero-flag rename when the destination is another
    // hard link to the source. Reject every observed entry first to preserve
    // Renamewright's stricter contract; the following native rename still
    // provides the atomic no-replace authority for entries created by a race.
    match std::fs::symlink_metadata(destination) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "destination already exists",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn encode_destination(destination: &Path) -> io::Result<Vec<u16>> {
    let encoded = destination.as_os_str().encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination path contains an interior NUL",
        ))
    } else {
        Ok(encoded)
    }
}
