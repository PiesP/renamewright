import { createSignal, For, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
import { APP_NAME } from './app-meta';
import {
  createPlanningClient,
  type LedgerEntry,
  type LedgerStatus,
  type Plan,
  type PlanningClient,
  type RecoveryCommandAction,
  type RecoveryDisposition,
  type RecoveryInspection,
  type UndoBlockReason,
  type UndoInspection,
} from './planning/client';
import {
  createRule,
  MAX_RULES,
  PlanningError,
  RULE_PIPELINE_SCHEMA_VERSION,
  type RuleKind,
  type RulePipelineRequest,
  type RuleRequest,
  ruleLabel,
} from './planning/rules';
import { VirtualPlanTable } from './planning/VirtualPlanTable';

interface AppProps {
  client?: PlanningClient;
}

interface RuleTextInputProps {
  id: string;
  label: string;
  value: string;
  placeholder: string;
  invalid: boolean;
  onInput: (value: string) => void;
}

function RuleTextInput(props: RuleTextInputProps) {
  return (
    <div class="rule-field">
      <label for={props.id}>{props.label}</label>
      <div class="input-shell" data-invalid={props.invalid ? 'true' : 'false'}>
        <input
          id={props.id}
          type="text"
          value={props.value}
          placeholder={props.placeholder}
          aria-invalid={props.invalid ? 'true' : 'false'}
          onInput={(event) => props.onInput(event.currentTarget.value)}
        />
        <span aria-hidden="true">{props.invalid ? '!' : ''}</span>
      </div>
    </div>
  );
}

export function App(props: AppProps) {
  const planningClient = props.client ?? createPlanningClient();
  const [rules, setRules] = createSignal<RuleRequest[]>([createRule(1, 'prefix')]);
  const [newRuleKind, setNewRuleKind] = createSignal<RuleKind>('suffix');
  const [ruleError, setRuleError] = createSignal<{
    code: string;
    ruleId: number | undefined;
  }>();
  const [plan, setPlan] = createSignal<Plan>();
  const [planDocument, setPlanDocument] = createSignal<string>();
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [notice, setNotice] = createSignal('');
  const [ledger, setLedger] = createSignal<LedgerEntry[]>([]);
  const [recoveryInspection, setRecoveryInspection] = createSignal<RecoveryInspection>();
  const [undoInspection, setUndoInspection] = createSignal<UndoInspection>();
  const [inspectingLedgerId, setInspectingLedgerId] = createSignal<number>();
  const [inspectingUndoLedgerId, setInspectingUndoLedgerId] = createSignal<number>();
  const [recoveryBusyAction, setRecoveryBusyAction] = createSignal<RecoveryCommandAction>();
  const [undoBusy, setUndoBusy] = createSignal(false);
  const [recoveryCancellationState, setRecoveryCancellationState] = createSignal<
    'requesting' | 'accepted' | 'rejected'
  >();
  const [undoCancellationState, setUndoCancellationState] = createSignal<
    'requesting' | 'accepted' | 'rejected'
  >();
  let requestSequence = 0;
  let planInspector: HTMLDialogElement | undefined;
  let planInspectorOpener: HTMLButtonElement | undefined;
  let recoveryInspectionPanel: HTMLDivElement | undefined;
  let undoInspectionPanel: HTMLDivElement | undefined;
  let ledgerHeading: HTMLHeadingElement | undefined;
  let previewTimer: number | undefined;
  let nextRuleId = 2;

  const currentRuleRequest = (): RulePipelineRequest => ({
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: rules(),
  });

  const dismissPlanInspector = () => {
    const inspector = planInspector;
    const opener = planInspectorOpener;
    planInspector = undefined;
    planInspectorOpener = undefined;
    if (inspector?.open && typeof inspector.close === 'function') {
      inspector.close();
    }
    setPlanDocument(undefined);
    queueMicrotask(() => {
      if (opener?.isConnected) {
        opener.focus();
      }
    });
  };

  const setResult = (result: Plan | null) => {
    if (result) {
      if (planDocument()) {
        dismissPlanInspector();
      }
      setPlan(result);
    }
  };

  const run = async (operation: () => Promise<Plan | null>) => {
    const sequence = ++requestSequence;
    setBusy(true);
    setError('');
    setRuleError(undefined);
    setNotice('');
    try {
      const result = await operation();
      if (sequence === requestSequence) {
        setResult(result);
      }
    } catch (cause) {
      if (sequence === requestSequence) {
        if (cause instanceof PlanningError) {
          setRuleError({ code: cause.code, ruleId: cause.ruleId });
        }
        setError(cause instanceof Error ? cause.message : 'The rename plan could not be updated.');
      }
    } finally {
      if (sequence === requestSequence) {
        setBusy(false);
      }
    }
  };

  const schedulePreview = () => {
    if (plan()) {
      if (previewTimer !== undefined) {
        window.clearTimeout(previewTimer);
      }
      previewTimer = window.setTimeout(() => {
        previewTimer = undefined;
        void run(() => planningClient.previewRules(currentRuleRequest()));
      }, 120);
    }
  };

  const replaceRule = (ruleId: number, update: (rule: RuleRequest) => RuleRequest) => {
    setRules((current) =>
      current.map((rule) => {
        if (rule.ruleId !== ruleId) {
          return rule;
        }
        Object.assign(rule, update(rule));
        return rule;
      })
    );
    schedulePreview();
  };

  const addRule = () => {
    if (rules().length >= MAX_RULES) {
      return;
    }
    setRules((current) => [...current, createRule(nextRuleId++, newRuleKind())]);
    schedulePreview();
  };

  const removeRule = (ruleId: number) => {
    setRules((current) => current.filter((rule) => rule.ruleId !== ruleId));
    schedulePreview();
  };

  const moveRule = (ruleId: number, offset: -1 | 1) => {
    setRules((current) => {
      const index = current.findIndex((rule) => rule.ruleId === ruleId);
      const destination = index + offset;
      if (index < 0 || destination < 0 || destination >= current.length) {
        return current;
      }
      const reordered = [...current];
      const currentRule = reordered[index];
      const destinationRule = reordered[destination];
      if (!currentRule || !destinationRule) {
        return current;
      }
      reordered[index] = destinationRule;
      reordered[destination] = currentRule;
      return reordered;
    });
    schedulePreview();
  };

  const loadInitialSources = () =>
    planningClient.nativeSelectionAvailable
      ? planningClient.selectSources(currentRuleRequest())
      : planningClient.loadSample(currentRuleRequest());

  const inspectCurrentPlan = async () => {
    const current = plan();
    if (!current) {
      return;
    }
    setBusy(true);
    setError('');
    setNotice('');
    try {
      setPlanDocument(await planningClient.inspectPlan(current.planId));
    } catch (cause) {
      planInspectorOpener = undefined;
      setError(cause instanceof Error ? cause.message : 'The plan document could not be opened.');
    } finally {
      setBusy(false);
    }
  };

  const exportCurrentPlan = async () => {
    const current = plan();
    if (!current) {
      return;
    }
    setBusy(true);
    setError('');
    setNotice('');
    try {
      const exported = await planningClient.exportPlan(current.planId);
      setNotice(exported ? 'Plan JSON exported.' : 'Plan export cancelled.');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'The plan document could not be exported.');
    } finally {
      setBusy(false);
    }
  };

  const inspectLedgerEntry = async (entry: LedgerEntry) => {
    setInspectingLedgerId(entry.ledgerId);
    setRecoveryInspection(undefined);
    setUndoInspection(undefined);
    setError('');
    try {
      setRecoveryInspection(await planningClient.inspectRecovery(entry.ledgerId));
      queueMicrotask(() => {
        recoveryInspectionPanel?.scrollIntoView?.({ block: 'nearest' });
      });
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : 'The recovery state could not be inspected.'
      );
    } finally {
      setInspectingLedgerId(undefined);
    }
  };

  const inspectUndoEntry = async (entry: LedgerEntry) => {
    setInspectingUndoLedgerId(entry.ledgerId);
    setUndoInspection(undefined);
    setRecoveryInspection(undefined);
    setError('');
    try {
      setUndoInspection(await planningClient.inspectUndo(entry.ledgerId));
      queueMicrotask(() => {
        undoInspectionPanel?.scrollIntoView?.({ block: 'nearest' });
      });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Undo could not be inspected.');
    } finally {
      setInspectingUndoLedgerId(undefined);
    }
  };

  const applyRecoveryAction = async (action: RecoveryCommandAction) => {
    const inspection = recoveryInspection();
    if (!inspection) {
      return;
    }
    setRecoveryBusyAction(action);
    setRecoveryCancellationState(undefined);
    queueMicrotask(() => recoveryInspectionPanel?.scrollIntoView?.({ block: 'center' }));
    setError('');
    setNotice('');
    try {
      const result = await planningClient.applyRecoveryAction(action, inspection);
      setLedger(result.ledger);
      if (!result.performed) {
        setNotice('Recovery action cancelled. No journal or file was changed.');
        return;
      }
      setRecoveryInspection(undefined);
      const messages = {
        cancelled: 'Recovery action cancelled.',
        completed: 'The interrupted rename transaction completed.',
        rolledBack: 'The interrupted rename transaction was rolled back.',
        recoveryRequired: 'Recovery stopped safely. Inspect the transaction again.',
        reconciled: 'Prepared-step observation recorded. Inspect the transaction again.',
      } as const;
      setNotice(messages[result.outcome]);
      queueMicrotask(() => ledgerHeading?.focus({ preventScroll: true }));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'The recovery action could not run.');
    } finally {
      setRecoveryCancellationState(undefined);
      setRecoveryBusyAction(undefined);
    }
  };

  const requestRecoveryCancellation = async () => {
    if (
      recoveryBusyAction() !== 'resume' ||
      recoveryInspection()?.direction !== 'forward' ||
      recoveryCancellationState() === 'requesting' ||
      recoveryCancellationState() === 'accepted'
    ) {
      return;
    }
    setRecoveryCancellationState('requesting');
    setError('');
    try {
      if (await planningClient.cancelRecovery()) {
        setRecoveryCancellationState('accepted');
        queueMicrotask(() => recoveryInspectionPanel?.scrollIntoView?.({ block: 'center' }));
      } else {
        setRecoveryCancellationState('rejected');
      }
    } catch (cause) {
      setRecoveryCancellationState(undefined);
      setError(
        cause instanceof Error ? cause.message : 'Recovery cancellation could not be requested.'
      );
    }
  };

  const applyUndo = async () => {
    const inspection = undoInspection();
    if (!inspection?.undoAvailable) {
      return;
    }
    setUndoBusy(true);
    setUndoCancellationState(undefined);
    queueMicrotask(() => undoInspectionPanel?.scrollIntoView?.({ block: 'center' }));
    setError('');
    setNotice('');
    try {
      const result = await planningClient.applyUndo(inspection);
      setLedger(result.ledger);
      if (!result.performed) {
        setNotice('Undo cancelled. No journal or file was changed.');
        return;
      }
      setUndoInspection(undefined);
      const messages = {
        cancelled: 'Undo was cancelled and rolled back safely.',
        completed: 'The completed rename transaction was undone.',
        rolledBack: 'Undo could not finish and all completed steps were rolled back.',
        recoveryRequired: 'Undo stopped safely. Use the new ledger recovery entry to continue.',
      } as const;
      setNotice(messages[result.outcome]);
      queueMicrotask(() => ledgerHeading?.focus({ preventScroll: true }));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Undo could not run.');
      setUndoInspection(undefined);
      try {
        setLedger(await planningClient.listLedger());
      } catch {
        // Preserve the original Undo error; a later ledger load can be retried.
      }
    } finally {
      setUndoCancellationState(undefined);
      setUndoBusy(false);
    }
  };

  const requestUndoCancellation = async () => {
    if (
      !undoBusy() ||
      undoCancellationState() === 'requesting' ||
      undoCancellationState() === 'accepted'
    ) {
      return;
    }
    setUndoCancellationState('requesting');
    setError('');
    try {
      if (await planningClient.cancelUndo()) {
        setUndoCancellationState('accepted');
        queueMicrotask(() => undoInspectionPanel?.scrollIntoView?.({ block: 'center' }));
      } else {
        setUndoCancellationState('rejected');
      }
    } catch (cause) {
      setUndoCancellationState(undefined);
      setError(
        cause instanceof Error ? cause.message : 'Undo cancellation could not be requested.'
      );
    }
  };

  onMount(() => {
    void planningClient
      .listLedger()
      .then(setLedger)
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : 'The Rename Ledger could not be loaded.');
      });
    const stopWatching = planningClient.watchSourceChanges((change) => {
      if (change.error) {
        setError(change.error);
        return;
      }
      void run(() => planningClient.previewRules(currentRuleRequest()));
    });
    onCleanup(() => {
      stopWatching();
      if (previewTimer !== undefined) {
        window.clearTimeout(previewTimer);
      }
    });
  });

  const statusMessage = () => {
    const current = plan();
    if (error()) {
      return error();
    }
    if (undoCancellationState() === 'accepted') {
      return 'Cancellation requested. Undo will roll back at the next safe step…';
    }
    if (undoCancellationState() === 'requesting') {
      return 'Requesting Undo cancellation…';
    }
    if (undoCancellationState() === 'rejected') {
      return 'Cancellation was not confirmed, or Undo is no longer active. Try again if the operation is still running.';
    }
    if (undoBusy()) {
      return 'Waiting for native confirmation or Undo completion…';
    }
    if (recoveryCancellationState() === 'accepted') {
      return 'Cancellation requested. Renamewright will roll back at the next safe step…';
    }
    if (recoveryCancellationState() === 'requesting') {
      return 'Requesting recovery cancellation…';
    }
    if (recoveryCancellationState() === 'rejected') {
      return 'Cancellation was not confirmed, or forward recovery is no longer active. Try again if the operation is still running.';
    }
    if (recoveryBusyAction()) {
      return 'Waiting for native confirmation or recovery completion…';
    }
    if (busy()) {
      return 'Updating the rename plan…';
    }
    if (notice()) {
      return notice();
    }
    if (!current) {
      return 'No sources are loaded.';
    }
    if (current.blockedCount > 0) {
      return `${current.blockedCount} names are blocked. Review diagnostics before continuing.`;
    }
    return `${current.changedCount} names are ready for review.`;
  };

  const ledgerStatusLabel = (status: LedgerStatus) => {
    const labels: Record<LedgerStatus, string> = {
      completed: 'Completed',
      rolledBack: 'Rolled back',
      forwardPending: 'Forward recovery pending',
      completionPending: 'Completion record pending',
      rollbackPending: 'Rollback pending',
      rollbackCompletionPending: 'Rollback record pending',
      reconciliationRequired: 'Inspection required',
      recoveryRequired: 'Recovery required',
      legacyInspectionRequired: 'Legacy inspection required',
      torn: 'Torn journal',
      damaged: 'Damaged journal',
      unsupportedVersion: 'Unsupported journal',
      tooLarge: 'Journal too large',
      discoveryLimitExceeded: 'Discovery limit reached',
      unreadable: 'Journal unreadable',
    };
    return labels[status];
  };

  const dispositionLabel = (disposition: RecoveryDisposition | null) => {
    if (!disposition) {
      return '';
    }
    const labels: Record<RecoveryDisposition, string> = {
      notApplied: 'The prepared rename was not applied.',
      applied: 'The prepared rename was applied.',
      missing: 'The expected entry is missing.',
      multipleLocations: 'The same identity appears in multiple locations.',
      unexpectedLocation: 'The entry is in an unexpected location.',
    };
    return labels[disposition];
  };

  const recoveryInspectionTitle = () => {
    switch (recoveryInspection()?.readiness) {
      case 'ready':
        return 'Identity checks passed';
      case 'reconciliationRequired':
        return 'Observation ready to record';
      case 'blocked':
        return 'Recovery is blocked';
      default:
        return '';
    }
  };

  const recoveryInspectionDescription = () => {
    const inspection = recoveryInspection();
    if (!inspection) {
      return '';
    }
    if (inspection.readiness === 'reconciliationRequired') {
      return `${dispositionLabel(inspection.disposition)} Recording this observation requires native confirmation.`;
    }
    if (inspection.readiness === 'blocked') {
      return 'The journal no longer matches one expected entry location. No action is available.';
    }
    return inspection.direction === 'forward'
      ? 'Resume or roll back only after native confirmation and a fresh identity check.'
      : 'Continue rollback only after native confirmation and a fresh identity check.';
  };

  const undoBlockDescription = (reason: UndoBlockReason | null) => {
    switch (reason) {
      case 'sourceChanged':
        return 'A renamed source no longer has the recorded identity. No file was changed.';
      case 'destinationOccupied':
        return 'An original name is occupied. Renamewright will not replace that entry.';
      default:
        return 'The transaction no longer passes the Undo safety checks.';
    }
  };

  return (
    <main class="workbench">
      <header class="source-bar">
        <div class="brand">
          <span class="brand-mark" aria-hidden="true">
            <span>R</span>
            <span>W</span>
          </span>
          <div>
            <strong>{APP_NAME}</strong>
            <span class="brand-mode">Plan every rename.</span>
          </div>
        </div>
        <div class="source-actions">
          <span class="read-only-badge">Recovery-only milestone</span>
          <span class="drop-hint">
            {planningClient.nativeSelectionAvailable ? 'Drop files anywhere' : 'Desktop drop ready'}
          </span>
          <button
            class="button button-secondary"
            type="button"
            disabled={!planningClient.nativeSelectionAvailable || busy()}
            aria-describedby="native-selection-note"
            onClick={() => void run(() => planningClient.selectSources(currentRuleRequest()))}
          >
            Add files
          </button>
        </div>
      </header>

      <p id="native-selection-note" class="browser-note">
        <Show
          when={planningClient.nativeSelectionAvailable}
          fallback="File selection is available in the desktop app. Use the sample in this browser preview."
        >
          Native picker and drop paths stay inside the Rust process.
        </Show>
      </p>

      <div class="workbench-grid">
        <aside class="rule-rail" aria-labelledby="rule-heading">
          <div class="rail-heading">
            <h1 id="rule-heading">Rename rules</h1>
            <span>{rules().filter((rule) => rule.enabled).length} active</span>
          </div>
          <div class="rule-list">
            <For
              each={rules()}
              fallback={<p class="empty-rules">No rules. Add one to build a rename pipeline.</p>}
            >
              {(rule, index) => {
                const inputId = (field: string) => `rule-${rule.ruleId}-${field}`;
                const invalid = () => ruleError()?.ruleId === rule.ruleId;
                return (
                  <section
                    class="rule-editor"
                    aria-labelledby={inputId('heading')}
                    data-disabled={rule.enabled ? 'false' : 'true'}
                    data-invalid={invalid() ? 'true' : 'false'}
                  >
                    <div class="rule-title">
                      <div>
                        <span class="rule-order">{String(index() + 1).padStart(2, '0')}</span>
                        <h2 id={inputId('heading')}>{ruleLabel(rule.kind)}</h2>
                      </div>
                      <div class="rule-actions">
                        <button
                          class="rule-icon-button"
                          type="button"
                          disabled={index() === 0}
                          aria-label={`Move ${ruleLabel(rule.kind)} up`}
                          onClick={() => moveRule(rule.ruleId, -1)}
                        >
                          ↑
                        </button>
                        <button
                          class="rule-icon-button"
                          type="button"
                          disabled={index() === rules().length - 1}
                          aria-label={`Move ${ruleLabel(rule.kind)} down`}
                          onClick={() => moveRule(rule.ruleId, 1)}
                        >
                          ↓
                        </button>
                        <button
                          class="rule-icon-button rule-remove-button"
                          type="button"
                          aria-label={`Remove ${ruleLabel(rule.kind)}`}
                          onClick={() => removeRule(rule.ruleId)}
                        >
                          ×
                        </button>
                      </div>
                    </div>
                    <label class="rule-toggle">
                      <input
                        type="checkbox"
                        checked={rule.enabled}
                        onChange={(event) =>
                          replaceRule(rule.ruleId, (current) => ({
                            ...current,
                            enabled: event.currentTarget.checked,
                          }))
                        }
                      />
                      Enabled
                    </label>
                    <Switch>
                      <Match when={rule.kind === 'prefix' && rule}>
                        {(current) => (
                          <RuleTextInput
                            id={inputId('value')}
                            label="Prefix"
                            value={current().value}
                            placeholder="2026-"
                            invalid={invalid()}
                            onInput={(value) =>
                              replaceRule(rule.ruleId, (candidate) =>
                                candidate.kind === 'prefix' ? { ...candidate, value } : candidate
                              )
                            }
                          />
                        )}
                      </Match>
                      <Match when={rule.kind === 'suffix' && rule}>
                        {(current) => (
                          <RuleTextInput
                            id={inputId('value')}
                            label="Suffix"
                            value={current().value}
                            placeholder="-final"
                            invalid={invalid()}
                            onInput={(value) =>
                              replaceRule(rule.ruleId, (candidate) =>
                                candidate.kind === 'suffix' ? { ...candidate, value } : candidate
                              )
                            }
                          />
                        )}
                      </Match>
                      <Match when={rule.kind === 'literalReplace' && rule}>
                        {(current) => (
                          <>
                            <RuleTextInput
                              id={inputId('search')}
                              label="Find"
                              value={current().search}
                              placeholder="draft"
                              invalid={invalid()}
                              onInput={(search) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'literalReplace'
                                    ? { ...candidate, search }
                                    : candidate
                                )
                              }
                            />
                            <RuleTextInput
                              id={inputId('replacement')}
                              label="Replace with"
                              value={current().replacement}
                              placeholder="final"
                              invalid={invalid()}
                              onInput={(replacement) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'literalReplace'
                                    ? { ...candidate, replacement }
                                    : candidate
                                )
                              }
                            />
                          </>
                        )}
                      </Match>
                      <Match when={rule.kind === 'regexReplace' && rule}>
                        {(current) => (
                          <>
                            <RuleTextInput
                              id={inputId('pattern')}
                              label="Rust regex"
                              value={current().pattern}
                              placeholder="^(.*)\\.txt$"
                              invalid={invalid()}
                              onInput={(pattern) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'regexReplace'
                                    ? { ...candidate, pattern }
                                    : candidate
                                )
                              }
                            />
                            <RuleTextInput
                              id={inputId('replacement')}
                              label="Replace with"
                              value={current().replacement}
                              placeholder="$1.md"
                              invalid={invalid()}
                              onInput={(replacement) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'regexReplace'
                                    ? { ...candidate, replacement }
                                    : candidate
                                )
                              }
                            />
                          </>
                        )}
                      </Match>
                    </Switch>
                    <Show when={invalid()}>
                      <p class="field-help field-help-error">{error()}</p>
                    </Show>
                  </section>
                );
              }}
            </For>
          </div>
          <div class="add-rule-controls">
            <label for="new-rule-kind">New rule</label>
            <div>
              <select
                id="new-rule-kind"
                value={newRuleKind()}
                onChange={(event) => setNewRuleKind(event.currentTarget.value as RuleKind)}
              >
                <option value="prefix">Prefix</option>
                <option value="suffix">Suffix</option>
                <option value="literalReplace">Replace text</option>
                <option value="regexReplace">Regular expression</option>
              </select>
              <button
                class="button button-secondary"
                type="button"
                disabled={rules().length >= MAX_RULES}
                onClick={addRule}
              >
                Add rule
              </button>
            </div>
            <p class="field-help">Rules run from top to bottom. Up to {MAX_RULES} are allowed.</p>
          </div>
          <div class="scope-note">
            <strong>Current scope</strong>
            <p>
              New plan execution remains locked. Recovery and Undo require fresh identity checks and
              native confirmation.
            </p>
          </div>
          <Show when={ledger().length > 0}>
            <section class="ledger-panel" aria-labelledby="ledger-heading">
              <div class="ledger-heading">
                <h2 id="ledger-heading" tabIndex={-1} ref={ledgerHeading}>
                  Rename Ledger
                </h2>
                <span>{ledger().length}</span>
              </div>
              <p>
                Inspect interrupted work or a completed rename before any native-confirmed action.
              </p>
              <ul aria-label="Rename journal status">
                {ledger().map((entry) => (
                  <li>
                    <div class="ledger-entry-summary">
                      <strong>
                        {entry.undoOfPlanId !== null
                          ? `Undo of plan ${entry.undoOfPlanId}`
                          : entry.planId === null
                            ? `Ledger ${entry.ledgerId}`
                            : `Plan ${entry.planId}`}
                      </strong>
                      <span>
                        {entry.undoOfPlanId !== null && entry.planId !== null
                          ? `Plan ${entry.planId} · `
                          : ''}
                        {entry.sourceCount} sources
                      </span>
                    </div>
                    <div class="ledger-entry-actions">
                      <span
                        data-recovery={entry.recoveryAvailable ? 'true' : 'false'}
                        data-undo={entry.undoAvailable ? 'true' : 'false'}
                      >
                        {ledgerStatusLabel(entry.status)}
                      </span>
                      <Show when={entry.recoveryAvailable}>
                        <button
                          class="button button-secondary button-compact ledger-inspect-button"
                          type="button"
                          disabled={
                            inspectingLedgerId() !== undefined ||
                            inspectingUndoLedgerId() !== undefined ||
                            recoveryBusyAction() !== undefined ||
                            undoBusy()
                          }
                          aria-label={`Inspect ${entry.planId === null ? `ledger ${entry.ledgerId}` : `plan ${entry.planId}`} recovery`}
                          onClick={() => void inspectLedgerEntry(entry)}
                        >
                          {inspectingLedgerId() === entry.ledgerId ? 'Inspecting…' : 'Inspect'}
                        </button>
                      </Show>
                      <Show when={entry.undoAvailable}>
                        <button
                          class="button button-secondary button-compact ledger-inspect-button"
                          type="button"
                          disabled={
                            inspectingLedgerId() !== undefined ||
                            inspectingUndoLedgerId() !== undefined ||
                            recoveryBusyAction() !== undefined ||
                            undoBusy()
                          }
                          aria-label={`Inspect ${entry.planId === null ? `ledger ${entry.ledgerId}` : `plan ${entry.planId}`} Undo`}
                          onClick={() => void inspectUndoEntry(entry)}
                        >
                          {inspectingUndoLedgerId() === entry.ledgerId
                            ? 'Inspecting…'
                            : 'Inspect Undo'}
                        </button>
                      </Show>
                    </div>
                  </li>
                ))}
              </ul>
              <Show when={recoveryInspection()}>
                <div
                  class="ledger-inspection"
                  role="status"
                  aria-live="polite"
                  ref={recoveryInspectionPanel}
                >
                  <strong>{recoveryInspectionTitle()}</strong>
                  <p>{recoveryInspectionDescription()}</p>
                  <span>
                    {recoveryInspection()?.direction === 'forward' ? 'Forward' : 'Rollback'}
                    {recoveryInspection()?.stepIndex === null
                      ? ' · terminal record'
                      : ` · step ${recoveryInspection()?.stepIndex}`}
                  </span>
                  <fieldset class="ledger-recovery-actions">
                    <legend class="visually-hidden">Available recovery actions</legend>
                    <Show when={recoveryInspection()?.reconcileAvailable}>
                      <button
                        class="button button-primary button-compact"
                        type="button"
                        disabled={recoveryBusyAction() !== undefined}
                        onClick={() => void applyRecoveryAction('reconcile')}
                      >
                        {recoveryBusyAction() === 'reconcile'
                          ? 'Waiting for confirmation…'
                          : 'Record observation'}
                      </button>
                    </Show>
                    <Show when={recoveryInspection()?.resumeAvailable}>
                      <button
                        class="button button-primary button-compact"
                        type="button"
                        disabled={recoveryBusyAction() !== undefined}
                        onClick={() => void applyRecoveryAction('resume')}
                      >
                        {recoveryBusyAction() === 'resume'
                          ? 'Recovering…'
                          : recoveryInspection()?.direction === 'forward'
                            ? 'Resume'
                            : 'Continue rollback'}
                      </button>
                    </Show>
                    <Show
                      when={
                        recoveryBusyAction() === 'resume' &&
                        recoveryInspection()?.direction === 'forward'
                      }
                    >
                      <button
                        class="button button-secondary button-compact"
                        type="button"
                        disabled={
                          recoveryCancellationState() === 'requesting' ||
                          recoveryCancellationState() === 'accepted'
                        }
                        onClick={() => void requestRecoveryCancellation()}
                      >
                        {recoveryCancellationState() === 'accepted'
                          ? 'Cancellation requested'
                          : recoveryCancellationState() === 'requesting'
                            ? 'Requesting cancellation…'
                            : recoveryCancellationState() === 'rejected'
                              ? 'Try cancel again'
                              : 'Cancel and roll back'}
                      </button>
                    </Show>
                    <Show when={recoveryInspection()?.rollbackAvailable}>
                      <button
                        class="button button-secondary button-compact"
                        type="button"
                        disabled={recoveryBusyAction() !== undefined}
                        onClick={() => void applyRecoveryAction('rollback')}
                      >
                        {recoveryBusyAction() === 'rollback' ? 'Rolling back…' : 'Roll back'}
                      </button>
                    </Show>
                  </fieldset>
                </div>
              </Show>
              <Show when={undoInspection()}>
                {(inspection) => (
                  <div
                    class="ledger-inspection ledger-undo-inspection"
                    data-readiness={inspection().readiness}
                    role="status"
                    aria-live="polite"
                    ref={undoInspectionPanel}
                  >
                    <strong>
                      {inspection().readiness === 'ready'
                        ? 'Undo checks passed'
                        : 'Undo is blocked'}
                    </strong>
                    <p>
                      {inspection().readiness === 'ready'
                        ? 'Native confirmation and one more identity check are required before the reverse transaction starts.'
                        : undoBlockDescription(inspection().blockReason)}
                    </p>
                    <span>
                      Plan {inspection().originalPlanId} · {inspection().sourceCount} sources
                    </span>
                    <Show when={inspection().undoAvailable}>
                      <fieldset class="ledger-recovery-actions">
                        <legend class="visually-hidden">Available Undo actions</legend>
                        <button
                          class="button button-primary button-compact ledger-action-button"
                          data-state={undoBusy() ? 'loading' : 'default'}
                          type="button"
                          disabled={undoBusy() || recoveryBusyAction() !== undefined}
                          onClick={() => void applyUndo()}
                        >
                          {undoBusy() ? 'Undoing…' : 'Undo rename'}
                        </button>
                        <Show when={undoBusy()}>
                          <button
                            class="button button-secondary button-compact ledger-action-button"
                            data-state={
                              undoCancellationState() === 'accepted' ? 'success' : 'default'
                            }
                            type="button"
                            disabled={
                              undoCancellationState() === 'requesting' ||
                              undoCancellationState() === 'accepted'
                            }
                            onClick={() => void requestUndoCancellation()}
                          >
                            {undoCancellationState() === 'accepted'
                              ? 'Cancellation requested'
                              : undoCancellationState() === 'requesting'
                                ? 'Requesting cancellation…'
                                : undoCancellationState() === 'rejected'
                                  ? 'Try cancel again'
                                  : 'Cancel and roll back'}
                          </button>
                        </Show>
                      </fieldset>
                    </Show>
                  </div>
                )}
              </Show>
            </section>
          </Show>
        </aside>

        <section class="preview-pane" aria-labelledby="preview-heading">
          <div class="preview-heading">
            <div>
              <h2 id="preview-heading">Proposed names</h2>
              <p>Original names remain untouched while you inspect this plan.</p>
            </div>
            <div class="plan-heading-actions">
              <span class="generation">Generation {plan()?.generation ?? 0}</span>
              <button
                class="button button-secondary button-compact"
                type="button"
                disabled={!plan() || busy()}
                onClick={(event) => {
                  planInspectorOpener = event.currentTarget;
                  void inspectCurrentPlan();
                }}
              >
                Inspect JSON
              </button>
            </div>
          </div>

          <Show
            when={plan()?.rows.length}
            fallback={
              <div class="empty-state">
                <span class="empty-mark" aria-hidden="true">
                  A → B
                </span>
                <h2>No sources in this plan</h2>
                <p>
                  Load the local sample to test this rule pipeline, or select and drop files in the
                  desktop app.
                </p>
                <button
                  class="button button-primary"
                  type="button"
                  disabled={busy()}
                  onClick={() => void run(loadInitialSources)}
                >
                  {busy()
                    ? planningClient.nativeSelectionAvailable
                      ? 'Opening…'
                      : 'Loading…'
                    : planningClient.nativeSelectionAvailable
                      ? 'Add files'
                      : 'Load sample'}
                </button>
              </div>
            }
          >
            <VirtualPlanTable rows={plan()?.rows ?? []} />
          </Show>
        </section>
      </div>

      <footer class="review-bar">
        <div class="plan-summary" aria-hidden={!plan()}>
          <span>
            <strong>{plan()?.rows.length ?? 0}</strong> sources
          </span>
          <span>
            <strong>{plan()?.changedCount ?? 0}</strong> changes
          </span>
          <span class={plan()?.blockedCount ? 'blocked-count' : ''}>
            <strong>{plan()?.blockedCount ?? 0}</strong> blocked
          </span>
        </div>
        <div class="execution-lock">
          <span>New plan execution remains locked. Ledger Recovery and Undo stay available.</span>
          <button class="button button-locked" type="button" disabled>
            Execution unavailable
          </button>
        </div>
      </footer>

      <Show when={planDocument()}>
        {(document) => (
          <dialog
            class="plan-inspector"
            aria-labelledby="plan-inspector-heading"
            ref={(element) => {
              planInspector = element;
              if (typeof element.showModal === 'function') {
                queueMicrotask(() => {
                  if (element.isConnected && !element.open) {
                    element.showModal();
                  }
                });
              } else {
                element.setAttribute('open', '');
              }
            }}
            onCancel={(event) => {
              event.preventDefault();
              dismissPlanInspector();
            }}
            onClose={dismissPlanInspector}
          >
            <div class="inspector-heading">
              <div>
                <span class="inspector-kicker">Versioned plan document</span>
                <h2 id="plan-inspector-heading">Plan {plan()?.planId}</h2>
              </div>
              <button
                class="button button-secondary button-compact"
                type="button"
                onClick={dismissPlanInspector}
              >
                Close
              </button>
            </div>
            <p>Display projections and opaque IDs only. Native paths are never included.</p>
            <pre>{document()}</pre>
            <div class="inspector-actions">
              <button
                class="button button-primary"
                type="button"
                disabled={!planningClient.nativeSelectionAvailable || busy()}
                title={
                  planningClient.nativeSelectionAvailable
                    ? undefined
                    : 'Export is available in the desktop app.'
                }
                onClick={() => void exportCurrentPlan()}
              >
                Export JSON…
              </button>
            </div>
          </dialog>
        )}
      </Show>

      <p
        class={
          error()
            ? 'live-status live-status-error'
            : notice()
              ? 'live-status live-status-active'
              : 'live-status'
        }
        role="status"
        aria-live="polite"
      >
        {statusMessage()}
      </p>
    </main>
  );
}
