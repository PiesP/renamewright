#![forbid(unsafe_code)]

use renamewright_application::{
    ApplicationService, LedgerEntryDto, PlanDto, PlanningCommandErrorDto, RecoveryCommandAction,
    RecoveryCommandErrorDto, RecoveryCommandErrorKind, RecoveryCommandResultDto,
    RecoveryInspectionDto, RecoveryRequestDto, RulePipelineRequestDto, SourceChangeDto,
    UndoCommandErrorDto, UndoCommandErrorKind, UndoCommandResultDto, UndoInspectionDto,
    UndoRequestDto,
};
use renamewright_core::ExecutionDirection;
use renamewright_platform::{
    NativeExecutionFileSystem, RecoveryTransactionInspection, UndoTransactionInspection,
};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use tauri::{AppHandle, DragDropEvent, Manager, State, WindowEvent};

#[tauri::command]
async fn select_sources(
    prefix: String,
    state: State<'_, ApplicationService>,
) -> Result<Option<PlanDto>, String> {
    select_sources_for_rules(ApplicationService::prefix_rule_request(prefix), state)
        .await
        .map_err(|error| error.code().to_owned())
}

#[tauri::command]
async fn select_sources_with_rules(
    request: RulePipelineRequestDto,
    state: State<'_, ApplicationService>,
) -> Result<Option<PlanDto>, PlanningCommandErrorDto> {
    select_sources_for_rules(request, state).await
}

async fn select_sources_for_rules(
    request: RulePipelineRequestDto,
    state: State<'_, ApplicationService>,
) -> Result<Option<PlanDto>, PlanningCommandErrorDto> {
    state.validate_rule_request(&request)?;
    let paths = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Add sources to Renamewright")
            .pick_files()
    })
    .await
    .map_err(|_| PlanningCommandErrorDto::new("pickerUnavailable"))?;

    let Some(paths) = paths else {
        return Ok(None);
    };

    state.admit_sources_with_rules(paths, request).map(Some)
}

#[tauri::command]
fn preview_prefix(prefix: String, state: State<'_, ApplicationService>) -> Result<PlanDto, String> {
    state.preview_prefix(prefix)
}

#[tauri::command]
fn preview_rules(
    request: RulePipelineRequestDto,
    state: State<'_, ApplicationService>,
) -> Result<PlanDto, PlanningCommandErrorDto> {
    state.preview_rules(request)
}

#[tauri::command]
fn poll_source_changes(
    since: u64,
    state: State<'_, ApplicationService>,
) -> Result<Option<SourceChangeDto>, String> {
    state.poll_source_changes(since)
}

#[tauri::command]
fn list_ledger(state: State<'_, ApplicationService>) -> Result<Vec<LedgerEntryDto>, String> {
    state.list_ledger()
}

#[tauri::command]
fn inspect_recovery(
    ledger_id: u64,
    state: State<'_, ApplicationService>,
) -> Result<RecoveryInspectionDto, String> {
    state.inspect_recovery(ledger_id, &NativeExecutionFileSystem::new())
}

#[tauri::command]
fn inspect_undo(
    ledger_id: u64,
    state: State<'_, ApplicationService>,
) -> Result<UndoInspectionDto, UndoCommandErrorDto> {
    state.inspect_undo(ledger_id, &NativeExecutionFileSystem::new())
}

#[tauri::command]
async fn apply_recovery_action(
    request: RecoveryRequestDto,
    app: AppHandle,
) -> Result<RecoveryCommandResultDto, RecoveryCommandErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationService>();
        state.apply_recovery_action(
            &request,
            &NativeExecutionFileSystem::new(),
            |action, inspection| confirm_recovery_action(&app, action, inspection),
        )
    })
    .await
    .map_err(|_| RecoveryCommandErrorDto::from(RecoveryCommandErrorKind::StateUnavailable))?
}

#[tauri::command]
async fn apply_undo(
    request: UndoRequestDto,
    app: AppHandle,
) -> Result<UndoCommandResultDto, UndoCommandErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationService>();
        state.apply_undo(&request, &NativeExecutionFileSystem::new(), |inspection| {
            confirm_undo(&app, inspection)
        })
    })
    .await
    .map_err(|_| UndoCommandErrorDto::from(UndoCommandErrorKind::StateUnavailable))?
}

#[tauri::command]
async fn cancel_recovery(app: AppHandle) -> Result<bool, RecoveryCommandErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationService>();
        state.request_confirmed_cancellation(|| {
            confirm_cancellation(&app, CancellationKind::Recovery)
        })
    })
    .await
    .map_err(|_| RecoveryCommandErrorDto::from(RecoveryCommandErrorKind::StateUnavailable))?
}

#[tauri::command]
async fn cancel_undo(app: AppHandle) -> Result<bool, UndoCommandErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ApplicationService>();
        state
            .request_confirmed_cancellation(|| confirm_cancellation(&app, CancellationKind::Undo))
            .map_err(|_| UndoCommandErrorDto::from(UndoCommandErrorKind::StateUnavailable))
    })
    .await
    .map_err(|_| UndoCommandErrorDto::from(UndoCommandErrorKind::StateUnavailable))?
}

#[tauri::command]
fn inspect_plan(plan_id: u64, state: State<'_, ApplicationService>) -> Result<String, String> {
    state.inspect_plan_json(plan_id)
}

#[tauri::command]
async fn export_plan(plan_id: u64, state: State<'_, ApplicationService>) -> Result<bool, String> {
    let document = state.inspect_plan_json(plan_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = rfd::FileDialog::new()
            .set_title("Export Renamewright plan")
            .add_filter("JSON plan", &["json"])
            .set_file_name(format!("renamewright-plan-{plan_id}.json"))
            .save_file();
        let Some(path) = path else {
            return Ok(false);
        };
        ApplicationService::export_document(&path, &document)?;
        Ok(true)
    })
    .await
    .map_err(|error| format!("the plan export did not complete: {error}"))?
}

#[tauri::command]
async fn export_plan_csv(
    plan_id: u64,
    state: State<'_, ApplicationService>,
) -> Result<bool, String> {
    let document = state.inspect_plan_csv(plan_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = rfd::FileDialog::new()
            .set_title("Export Renamewright plan CSV")
            .add_filter("CSV inspection", &["csv"])
            .set_file_name(format!("renamewright-plan-{plan_id}.csv"))
            .save_file();
        let Some(path) = path else {
            return Ok(false);
        };
        ApplicationService::export_document(&path, &document)?;
        Ok(true)
    })
    .await
    .map_err(|_| "the CSV export did not complete".to_owned())?
}

fn confirm_recovery_action(
    app: &AppHandle,
    action: RecoveryCommandAction,
    inspection: RecoveryTransactionInspection,
) -> bool {
    let description = match action {
        RecoveryCommandAction::Resume if inspection.direction() == ExecutionDirection::Forward => {
            "Continue this interrupted rename transaction? Renamewright will recheck every file identity and will not replace existing destinations."
        }
        RecoveryCommandAction::Resume => {
            "Continue rolling back this interrupted transaction? Renamewright will recheck every file identity before each step."
        }
        RecoveryCommandAction::Rollback => {
            "Roll back this interrupted rename transaction? Renamewright will recheck every file identity and will not replace existing destinations."
        }
        RecoveryCommandAction::Reconcile => {
            "Record the inspected step observation in the local journal? Inspect the transaction again before any rename continues."
        }
    };
    let mut dialog = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title("Confirm Renamewright recovery")
        .set_description(description)
        .set_buttons(MessageButtons::YesNo);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.show() == MessageDialogResult::Yes
}

fn confirm_undo(app: &AppHandle, inspection: UndoTransactionInspection) -> bool {
    let description = format!(
        "Undo this completed rename transaction for {} source(s)? Renamewright will recheck every file identity, create a separate recovery journal, and will not replace existing destinations.",
        inspection.source_count()
    );
    let mut dialog = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title("Confirm Renamewright Undo")
        .set_description(description)
        .set_buttons(MessageButtons::YesNo);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.show() == MessageDialogResult::Yes
}

#[derive(Clone, Copy)]
enum CancellationKind {
    Recovery,
    Undo,
}

fn confirm_cancellation(app: &AppHandle, kind: CancellationKind) -> bool {
    let description = match kind {
        CancellationKind::Recovery => {
            "Cancel this active forward recovery and roll back its completed steps? Renamewright will stop only at a safe step boundary."
        }
        CancellationKind::Undo => {
            "Cancel this active Undo and roll back its completed steps? Renamewright will stop only at a safe step boundary."
        }
    };
    let mut dialog = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title("Confirm Renamewright cancellation")
        .set_description(description)
        .set_buttons(MessageButtons::YesNo);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.show() == MessageDialogResult::Yes
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(ApplicationService::default())
        .setup(|app| {
            let journal_root = app
                .path()
                .app_data_dir()
                .map_err(|_| "the application data directory is unavailable")?
                .join("journals");
            app.state::<ApplicationService>()
                .initialize(&journal_root)
                .map_err(std::io::Error::other)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) = event {
                let app_handle = window.app_handle().clone();
                let paths = paths.clone();
                std::thread::spawn(move || {
                    app_handle
                        .state::<ApplicationService>()
                        .admit_dropped_sources(&paths);
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            select_sources,
            select_sources_with_rules,
            preview_prefix,
            preview_rules,
            poll_source_changes,
            list_ledger,
            inspect_recovery,
            inspect_undo,
            apply_recovery_action,
            apply_undo,
            cancel_recovery,
            cancel_undo,
            inspect_plan,
            export_plan,
            export_plan_csv
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("failed to run Renamewright: {error}");
            std::process::exit(1);
        });
}
