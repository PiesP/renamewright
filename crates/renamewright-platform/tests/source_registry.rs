use std::error::Error;
use std::ffi::OsStr;
use std::fs;

use renamewright_core::EntryKind;
use renamewright_platform::{AdmissionError, SourceRegistry};

#[test]
fn registry_admits_existing_files_without_exposing_paths_in_snapshots() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let alpha = directory.path().join("alpha.txt");
    let beta = directory.path().join("beta.txt");
    fs::write(&alpha, b"alpha")?;
    fs::write(&beta, b"beta")?;

    let mut registry = SourceRegistry::new();
    let admitted = registry.admit_paths([alpha.clone(), beta])?;
    let canonical_alpha = alpha.canonicalize()?;

    assert_eq!(admitted.len(), 2);
    assert_eq!(admitted[0].native_name(), OsStr::new("alpha.txt"));
    assert_eq!(registry.snapshots(), admitted);
    assert_eq!(registry.generation(), 1);
    assert_eq!(
        registry.path_for(admitted[0].id()),
        Some(canonical_alpha.as_path())
    );
    Ok(())
}

#[test]
fn registry_ignores_duplicate_admission_and_rejects_missing_sources() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("report.pdf");
    fs::write(&source, b"report")?;

    let mut registry = SourceRegistry::new();
    registry.admit_paths([source.clone(), source])?;

    assert_eq!(registry.snapshots().len(), 1);
    assert_eq!(registry.generation(), 1);

    let missing = directory.path().join("missing.txt");
    let Err(error) = registry.admit_paths([missing.clone()]) else {
        return Err("missing paths must be rejected".into());
    };
    assert!(matches!(error, AdmissionError::Unavailable(path) if path == missing));
    Ok(())
}

#[test]
fn registry_admits_a_directory_entry_without_enumerating_children() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let selected = root.path().join("selected");
    fs::create_dir(&selected)?;
    fs::write(selected.join("child.txt"), b"child")?;

    let mut registry = SourceRegistry::new();
    let admitted = registry.admit_paths([selected])?;

    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].entry_kind(), Some(EntryKind::Directory));
    assert_eq!(admitted[0].native_name(), OsStr::new("selected"));
    Ok(())
}
