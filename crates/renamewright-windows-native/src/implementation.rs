use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle};
use std::path::Path;
use std::ptr;

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_OPEN, FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION, FILE_SYNCHRONOUS_IO_NONALERT,
    FileRenameInformation, NtCreateFile, NtSetInformationFile, RtlNtStatusToDosErrorNoTeb,
};
use windows_sys::Win32::Foundation::{OBJ_CASE_INSENSITIVE, UNICODE_STRING};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
    GetSysColor,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_TRAVERSE, FileIdInfo, GetFileInformationByHandleEx, SYNCHRONIZE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETHIGHCONTRAST, SystemParametersInfoW};

const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HighContrastPalette {
    pub window: [u8; 3],
    pub window_text: [u8; 3],
    pub highlight: [u8; 3],
    pub highlight_text: [u8; 3],
    pub gray_text: [u8; 3],
}

const fn colorref_to_rgb(color: u32) -> [u8; 3] {
    [
        (color & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        ((color >> 16) & 0xff) as u8,
    ]
}

pub fn high_contrast_palette() -> io::Result<Option<HighContrastPalette>> {
    let mut settings = HIGHCONTRASTW {
        cbSize: u32::try_from(size_of::<HIGHCONTRASTW>()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "high-contrast settings buffer is too large",
            )
        })?,
        dwFlags: 0,
        lpszDefaultScheme: ptr::null_mut(),
    };

    // SAFETY: `settings` is a writable, correctly aligned `HIGHCONTRASTW`
    // whose declared size matches its allocation. The pointer remains valid
    // for the synchronous call and is not retained by Win32. This operation
    // only reads the current accessibility setting; it cannot change it.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let succeeded = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            settings.cbSize,
            ptr::from_mut(&mut settings).cast(),
            0,
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    if settings.dwFlags & HCF_HIGHCONTRASTON == 0 {
        return Ok(None);
    }

    // SAFETY: `GetSysColor` reads immutable process-global Windows theme
    // state for fixed, documented indices and does not dereference pointers.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let palette = unsafe {
        HighContrastPalette {
            window: colorref_to_rgb(GetSysColor(COLOR_WINDOW)),
            window_text: colorref_to_rgb(GetSysColor(COLOR_WINDOWTEXT)),
            highlight: colorref_to_rgb(GetSysColor(COLOR_HIGHLIGHT)),
            highlight_text: colorref_to_rgb(GetSysColor(COLOR_HIGHLIGHTTEXT)),
            gray_text: colorref_to_rgb(GetSysColor(COLOR_GRAYTEXT)),
        }
    };
    Ok(Some(palette))
}

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

#[derive(Debug)]
pub struct ParentHandle {
    file: File,
}

impl ParentHandle {
    pub fn open(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "parent directory must be absolute",
            ));
        }
        let file = OpenOptions::new()
            .access_mode(DELETE | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
            .share_mode(SHARE_ALL)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "parent must be a non-reparse directory",
            ));
        }
        Ok(Self { file })
    }

    #[must_use]
    pub fn as_handle(&self) -> BorrowedHandle<'_> {
        self.file.as_handle()
    }
}

impl EntryHandle {
    pub fn open_relative(parent: &ParentHandle, name: &OsStr) -> io::Result<Self> {
        open_relative(
            parent.as_handle(),
            name,
            DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        )
        .map(|file| Self { file })
    }

    #[must_use]
    pub fn as_handle(&self) -> BorrowedHandle<'_> {
        self.file.as_handle()
    }
}

fn open_relative(
    parent: BorrowedHandle<'_>,
    name: &OsStr,
    desired_access: u32,
) -> io::Result<File> {
    let mut encoded_name = encode_leaf_name(name)?;
    let name_bytes = encoded_name
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "entry name is too large"))?;
    let name_length = u16::try_from(name_bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "entry name is too large"))?;
    let object_name = UNICODE_STRING {
        Length: name_length,
        MaximumLength: name_length,
        Buffer: encoded_name.as_mut_ptr(),
    };
    let object_attributes = OBJECT_ATTRIBUTES {
        Length: u32::try_from(size_of::<OBJECT_ATTRIBUTES>()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "object attributes are too large",
            )
        })?,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: ptr::from_ref(&object_name),
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: ptr::null(),
        SecurityQualityOfService: ptr::null(),
    };
    let mut status_block = IO_STATUS_BLOCK::default();
    let mut handle = ptr::null_mut();

    // SAFETY: `parent` is borrowed for the complete synchronous call.
    // `object_name` references the checked UTF-16 leaf allocation, and both it
    // and `object_attributes` remain initialized and immovable until return.
    // The status and output-handle pointers are writable and correctly aligned.
    // No optional allocation or EA buffers are supplied. On success ownership
    // of the returned handle is transferred exactly once to `File` below.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let status = unsafe {
        NtCreateFile(
            ptr::from_mut(&mut handle),
            desired_access,
            ptr::from_ref(&object_attributes),
            ptr::from_mut(&mut status_block),
            ptr::null(),
            0,
            SHARE_ALL,
            FILE_OPEN,
            FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null(),
            0,
        )
    };
    if status < 0 {
        // SAFETY: converting an NTSTATUS returned by `NtCreateFile` has no
        // pointer or lifetime preconditions and does not retain process state.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        let os_code = unsafe { RtlNtStatusToDosErrorNoTeb(status) };
        return Err(io::Error::from_raw_os_error(
            i32::try_from(os_code).unwrap_or(i32::MAX),
        ));
    }
    if handle.is_null() {
        return Err(io::Error::other("relative entry open returned no handle"));
    }

    // SAFETY: successful `NtCreateFile` returned a new owned handle, checked
    // above for null. This is its only ownership transfer; `File` closes it.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    Ok(unsafe { File::from_raw_handle(handle) })
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
    destination_parent: BorrowedHandle<'_>,
    destination_name: &OsStr,
) -> io::Result<()> {
    reject_existing_destination(destination_parent, destination_name)?;
    let encoded_name = encode_leaf_name(destination_name)?;
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
    let buffer_bytes = offset_of!(FILE_RENAME_INFORMATION, FileName)
        .checked_add(terminated_name_bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename buffer is too large"))?;
    let buffer_size = u32::try_from(buffer_bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "rename buffer is too large"))?;
    let element_count = buffer_bytes.div_ceil(size_of::<FILE_RENAME_INFORMATION>());
    let mut buffer = vec![FILE_RENAME_INFORMATION::default(); element_count];
    let header = &mut buffer[0];
    // Zero flags preserve ordinary no-replace semantics.
    header.Anonymous.Flags = 0;
    header.RootDirectory = destination_parent.as_raw_handle();
    header.FileNameLength = file_name_length;

    // SAFETY: `buffer` is a zero-initialized `Vec<FILE_RENAME_INFORMATION>`, so its
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
            .add(offset_of!(FILE_RENAME_INFORMATION, FileName))
            .cast::<u16>();
        ptr::copy_nonoverlapping(encoded_name.as_ptr(), file_name, encoded_name.len());
    }

    // SAFETY: `source` stays borrowed for the call.
    // `buffer` is correctly aligned, fully initialized for `buffer_size` bytes,
    // has zero rename flags, a borrowed root-directory handle retained for the
    // call, a checked byte-length field excluding the trailing NUL, and a leaf
    // UTF-16 name interpreted only relative to that retained parent authority.
    // Win32 consumes the buffer synchronously and does not retain its pointer.
    // The generic Semgrep unsafe-usage rule is suppressed only at this audited
    // FFI boundary; the safety proof and crate-level unsafe lints remain active.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let mut status_block = IO_STATUS_BLOCK::default();
    // SAFETY: the proof above applies to this synchronous native call.
    let status = unsafe {
        NtSetInformationFile(
            source.as_raw_handle(),
            ptr::from_mut(&mut status_block),
            buffer.as_ptr().cast(),
            buffer_size,
            FileRenameInformation,
        )
    };
    if status < 0 {
        // SAFETY: converting the NTSTATUS returned by `NtSetInformationFile`
        // has no pointer or lifetime preconditions.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        let os_code = unsafe { RtlNtStatusToDosErrorNoTeb(status) };
        Err(io::Error::from_raw_os_error(
            i32::try_from(os_code).unwrap_or(i32::MAX),
        ))
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

fn reject_existing_destination(parent: BorrowedHandle<'_>, name: &OsStr) -> io::Result<()> {
    // Windows can accept a zero-flag rename when the destination is another
    // hard link to the source. Reject every observed entry first to preserve
    // Renamewright's stricter contract; the following native rename still
    // provides the atomic no-replace authority for entries created by a race.
    match open_relative(parent, name, FILE_READ_ATTRIBUTES | SYNCHRONIZE) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "destination already exists",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::colorref_to_rgb;

    #[test]
    fn colorref_conversion_preserves_windows_bgr_layout() {
        assert_eq!(colorref_to_rgb(0x00_33_22_11), [0x11, 0x22, 0x33]);
    }
}
