#[cfg(windows)]
mod implementation;

#[cfg(windows)]
pub use implementation::{
    EntryHandle, FileIdentity, HighContrastPalette, file_identity, high_contrast_palette,
    rename_noreplace,
};
