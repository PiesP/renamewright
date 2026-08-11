#[cfg(windows)]
mod implementation;

#[cfg(windows)]
pub use implementation::{EntryHandle, FileIdentity, file_identity, rename_noreplace};
