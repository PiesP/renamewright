#![forbid(unsafe_code)]

use std::sync::Mutex;

use renamewright_core::{
    DiagnosticCode, NameStatus, PlanId, RenamePlan, RenameRule, TargetPolicy, build_plan,
};
use renamewright_platform::SourceRegistry;
use serde::Serialize;
use tauri::{DragDropEvent, Manager, State, WindowEvent};

#[derive(Debug)]
struct AppState {
    registry: Mutex<SourceRegistry>,
    next_plan_id: Mutex<u64>,
    source_changes: Mutex<SourceChanges>,
}

#[derive(Debug, Default)]
struct SourceChanges {
    revision: u64,
    error: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            registry: Mutex::new(SourceRegistry::new()),
            next_plan_id: Mutex::new(1),
            source_changes: Mutex::new(SourceChanges::default()),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceChangeDto {
    revision: u64,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanDto {
    generation: u64,
    rows: Vec<PlanRowDto>,
    changed_count: usize,
    blocked_count: usize,
    can_apply: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanRowDto {
    source_id: u64,
    original_name: String,
    proposed_name: String,
    status: &'static str,
    diagnostics: Vec<&'static str>,
}

#[tauri::command]
async fn select_sources(
    prefix: String,
    state: State<'_, AppState>,
) -> Result<Option<PlanDto>, String> {
    let paths = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Add sources to Renamewright")
            .pick_files()
    })
    .await
    .map_err(|error| format!("the file picker did not complete: {error}"))?;

    let Some(paths) = paths else {
        return Ok(None);
    };

    let mut registry = state
        .registry
        .lock()
        .map_err(|_| "the source registry is unavailable".to_owned())?;
    registry
        .admit_paths(paths)
        .map_err(|error| error.to_string())?;
    plan_from_registry(&mut registry, &prefix, &state).map(Some)
}

#[tauri::command]
fn preview_prefix(prefix: String, state: State<'_, AppState>) -> Result<PlanDto, String> {
    let mut registry = state
        .registry
        .lock()
        .map_err(|_| "the source registry is unavailable".to_owned())?;
    plan_from_registry(&mut registry, &prefix, &state)
}

#[tauri::command]
fn poll_source_changes(
    since: u64,
    state: State<'_, AppState>,
) -> Result<Option<SourceChangeDto>, String> {
    let changes = state
        .source_changes
        .lock()
        .map_err(|_| "the source change tracker is unavailable".to_owned())?;
    if changes.revision <= since {
        return Ok(None);
    }

    Ok(Some(SourceChangeDto {
        revision: changes.revision,
        error: changes.error.clone(),
    }))
}

fn admit_dropped_sources(state: &AppState, paths: &[std::path::PathBuf]) {
    let outcome = state
        .registry
        .lock()
        .map_err(|_| "the source registry is unavailable".to_owned())
        .and_then(|mut registry| {
            registry
                .admit_paths(paths.iter().cloned())
                .map(|_| ())
                .map_err(|error| error.to_string())
        });

    if let Ok(mut changes) = state.source_changes.lock() {
        changes.revision = changes.revision.saturating_add(1);
        changes.error = outcome.err();
    }
}

fn plan_from_registry(
    registry: &mut SourceRegistry,
    prefix: &str,
    state: &State<'_, AppState>,
) -> Result<PlanDto, String> {
    let mut next_plan_id = state
        .next_plan_id
        .lock()
        .map_err(|_| "the plan sequence is unavailable".to_owned())?;
    let plan_id = PlanId::new(*next_plan_id);
    *next_plan_id = next_plan_id.saturating_add(1);
    let plan = build_plan(
        plan_id,
        registry.generation(),
        &registry.snapshots(),
        &[RenameRule::prefix(prefix)],
        TargetPolicy::windows(),
    );
    Ok(PlanDto::from(plan))
}

impl From<RenamePlan> for PlanDto {
    fn from(plan: RenamePlan) -> Self {
        Self {
            generation: plan.generation(),
            rows: plan
                .rows()
                .iter()
                .map(|row| PlanRowDto {
                    source_id: row.source_id().value(),
                    original_name: row.original_display().to_owned(),
                    proposed_name: row.proposed_display().to_owned(),
                    status: match row.status() {
                        NameStatus::Changed => "changed",
                        NameStatus::Unchanged => "unchanged",
                        NameStatus::Blocked => "blocked",
                    },
                    diagnostics: row
                        .diagnostics()
                        .iter()
                        .map(|diagnostic| diagnostic_name(diagnostic.code()))
                        .collect(),
                })
                .collect(),
            changed_count: plan.changed_count(),
            blocked_count: plan.blocked_count(),
            can_apply: plan.can_apply(),
        }
    }
}

const fn diagnostic_name(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::Unchanged => "unchanged",
        DiagnosticCode::EmptyName => "emptyName",
        DiagnosticCode::IllegalCharacter => "illegalCharacter",
        DiagnosticCode::TrailingDotOrSpace => "trailingDotOrSpace",
        DiagnosticCode::ReservedName => "reservedName",
        DiagnosticCode::NameTooLong => "nameTooLong",
        DiagnosticCode::DuplicateDestination => "duplicateDestination",
        DiagnosticCode::UnsupportedEncoding => "unsupportedEncoding",
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) = event {
                let app_handle = window.app_handle().clone();
                let paths = paths.clone();
                std::thread::spawn(move || {
                    admit_dropped_sources(&app_handle.state::<AppState>(), &paths);
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            select_sources,
            preview_prefix,
            poll_source_changes
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("failed to run Renamewright: {error}");
            std::process::exit(1);
        });
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;

    use super::{AppState, admit_dropped_sources};

    #[test]
    fn dropped_sources_update_only_the_rust_owned_registry() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("drop.txt");
        fs::write(&source, b"drop")?;
        let state = AppState::default();

        admit_dropped_sources(&state, &[source]);

        let registry = state.registry.lock().map_err(|_| "registry lock failed")?;
        let changes = state
            .source_changes
            .lock()
            .map_err(|_| "change lock failed")?;
        assert_eq!(registry.snapshots().len(), 1);
        assert_eq!(changes.revision, 1);
        assert_eq!(changes.error, None);
        Ok(())
    }
}
