#[cfg(windows)]
mod implementation;

#[cfg(windows)]
pub use implementation::{
    DirectoryHandle, EntryHandle, FileIdentity, file_identity, rename_noreplace,
};
