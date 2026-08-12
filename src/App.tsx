import { createEffect, createSignal, For, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
import { APP_NAME } from './app-meta';
import {
  formatNumber,
  type Locale,
  type LocaleStorage,
  localizedError,
  type MessageKey,
  message,
  persistLocale,
  resolveLocale,
} from './i18n/catalog';
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
  addPreset,
  deletePreset,
  emptyPresetDocument,
  MAX_PRESETS,
  type PresetDocument,
  type PresetStorage,
  readPresetDocument,
  writePresetDocument,
} from './planning/presets';
import {
  createRule,
  type FilenamePart,
  MAX_RULES,
  PlanningError,
  RULE_PIPELINE_SCHEMA_VERSION,
  type RuleKind,
  type RulePipelineRequest,
  type RuleRequest,
  type SourceOverride,
} from './planning/rules';
import { VirtualPlanTable } from './planning/VirtualPlanTable';

interface AppProps {
  client?: PlanningClient;
  presetStorage?: PresetStorage;
  localeStorage?: LocaleStorage;
}

interface RuleTextInputProps {
  id: string;
  label: string;
  value: string;
  placeholder: string;
  invalid: boolean;
  onInput: (value: string) => void;
}

interface RuleNumberInputProps {
  id: string;
  label: string;
  value: number;
  min: number;
  max: number;
  invalid: boolean;
  disabled?: boolean;
  onInput: (value: number) => void;
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

function RuleNumberInput(props: RuleNumberInputProps) {
  return (
    <div class="rule-field">
      <label for={props.id}>{props.label}</label>
      <div class="input-shell" data-invalid={props.invalid ? 'true' : 'false'}>
        <input
          id={props.id}
          type="number"
          inputMode="numeric"
          min={props.min}
          max={props.max}
          step="1"
          value={props.value}
          aria-invalid={props.invalid ? 'true' : 'false'}
          disabled={props.disabled}
          onInput={(event) => {
            const value = event.currentTarget.valueAsNumber;
            props.onInput(Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : 0);
          }}
        />
        <span aria-hidden="true">{props.invalid ? '!' : ''}</span>
      </div>
    </div>
  );
}

function FilenamePartSelect(props: {
  id: string;
  value: FilenamePart;
  locale: Locale;
  onChange: (value: FilenamePart) => void;
}) {
  return (
    <div class="rule-field">
      <label for={props.id}>{message(props.locale, 'fieldApplyTo')}</label>
      <select
        id={props.id}
        value={props.value}
        onChange={(event) => props.onChange(event.currentTarget.value as FilenamePart)}
      >
        <option value="wholeName">{message(props.locale, 'partWholeName')}</option>
        <option value="stem">{message(props.locale, 'partStem')}</option>
        <option value="extension">{message(props.locale, 'partExtension')}</option>
      </select>
    </div>
  );
}

export function App(props: AppProps) {
  const planningClient = props.client ?? createPlanningClient();
  const localeStorage = props.localeStorage ?? window.localStorage;
  const [locale, setLocale] = createSignal<Locale>(
    resolveLocale(localeStorage, typeof navigator === 'undefined' ? [] : navigator.languages)
  );
  const [rules, setRules] = createSignal<RuleRequest[]>([createRule(1, 'prefix')]);
  const [overrides, setOverrides] = createSignal<SourceOverride[]>([]);
  const [presetDocument, setPresetDocument] = createSignal<PresetDocument>(emptyPresetDocument());
  const [presetName, setPresetName] = createSignal('');
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
  let recoveryInspectionPanel: HTMLElement | undefined;
  let undoInspectionPanel: HTMLElement | undefined;
  let ledgerHeading: HTMLHeadingElement | undefined;
  let previewTimer: number | undefined;
  let nextRuleId = 2;

  const text = (key: MessageKey, values?: Readonly<Record<string, string | number>>) =>
    message(locale(), key, values);
  const count = (value: number) => formatNumber(locale(), value);
  const ruleLabelKey = (kind: RuleKind): MessageKey => {
    const keys: Record<RuleKind, MessageKey> = {
      prefix: 'rulePrefix',
      suffix: 'ruleSuffix',
      literalReplace: 'ruleLiteralReplace',
      regexReplace: 'ruleRegexReplace',
      sequence: 'ruleSequence',
      extension: 'ruleExtension',
      case: 'ruleCase',
      whitespaceCleanup: 'ruleWhitespaceCleanup',
      unicodeNormalization: 'ruleUnicodeNormalization',
      range: 'ruleRange',
      characterClass: 'ruleCharacterClass',
    };
    return keys[kind];
  };
  const localizedRuleLabel = (kind: RuleKind) => text(ruleLabelKey(kind));
  const selectLocale = (nextLocale: Locale) => {
    setLocale(nextLocale);
    setError('');
    if (!persistLocale(localeStorage, nextLocale)) {
      setError(text('errorLocaleStorageUnavailable'));
    }
  };

  const originalDocumentLanguage = document.documentElement.lang;
  createEffect(() => {
    document.documentElement.lang = locale();
  });
  onCleanup(() => {
    document.documentElement.lang = originalDocumentLanguage;
  });

  const currentRuleRequest = (): RulePipelineRequest => ({
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: rules(),
    overrides: overrides(),
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
        setError(localizedError(locale(), cause, 'errorPlanningFailed'));
      }
    } finally {
      if (sequence === requestSequence) {
        setBusy(false);
      }
    }
  };

  const schedulePreview = (successNotice?: string) => {
    if (plan()) {
      if (previewTimer !== undefined) {
        window.clearTimeout(previewTimer);
      }
      previewTimer = window.setTimeout(() => {
        previewTimer = undefined;
        void run(() => planningClient.previewRules(currentRuleRequest())).then(() => {
          if (successNotice && !error()) {
            setNotice(successNotice);
          }
        });
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

  const savePreset = () => {
    setError('');
    setNotice('');
    try {
      const next = addPreset(presetDocument(), presetName(), rules());
      writePresetDocument(props.presetStorage ?? window.localStorage, next);
      setPresetDocument(next);
      setPresetName('');
      setNotice(text('noticePresetSaved'));
    } catch (cause) {
      setError(localizedError(locale(), cause, 'errorPresetSaveFailed'));
    }
  };

  const applyPreset = (presetId: number) => {
    const preset = presetDocument().presets.find((candidate) => candidate.presetId === presetId);
    if (!preset) {
      return;
    }
    const nextRules = structuredClone(preset.rules);
    setRules(nextRules);
    nextRuleId = nextRules.reduce((maximum, rule) => Math.max(maximum, rule.ruleId), 0) + 1;
    setRuleError(undefined);
    setError('');
    setNotice('');
    const appliedNotice = text('noticePresetApplied', { name: preset.name });
    if (plan()) {
      schedulePreview(appliedNotice);
    } else {
      setNotice(appliedNotice);
    }
  };

  const removePreset = (presetId: number) => {
    setError('');
    setNotice('');
    try {
      const next = deletePreset(presetDocument(), presetId);
      writePresetDocument(props.presetStorage ?? window.localStorage, next);
      setPresetDocument(next);
      setNotice(text('noticePresetDeleted'));
    } catch (cause) {
      setError(localizedError(locale(), cause, 'errorPresetDeleteFailed'));
    }
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
      setError(localizedError(locale(), cause, 'errorPlanOpenFailed'));
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
      setNotice(exported ? text('noticePlanJsonExported') : text('noticePlanExportCancelled'));
    } catch (cause) {
      setError(localizedError(locale(), cause, 'errorPlanExportFailed'));
    } finally {
      setBusy(false);
    }
  };

  const exportCurrentPlanCsv = async () => {
    const current = plan();
    if (!current) {
      return;
    }
    setBusy(true);
    setError('');
    setNotice('');
    try {
      const exported = await planningClient.exportPlanCsv(current.planId);
      setNotice(exported ? text('noticePlanCsvExported') : text('noticePlanCsvExportCancelled'));
    } catch (cause) {
      setError(localizedError(locale(), cause, 'errorPlanCsvExportFailed'));
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
      setError(localizedError(locale(), cause, 'errorRecoveryInspectFailed'));
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
      setError(localizedError(locale(), cause, 'errorUndoInspectFailed'));
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
        setNotice(text('noticeRecoveryCancelledNoChange'));
        return;
      }
      setRecoveryInspection(undefined);
      const messageKeys = {
        cancelled: 'noticeRecoveryCancelled',
        completed: 'noticeRecoveryCompleted',
        rolledBack: 'noticeRecoveryRolledBack',
        recoveryRequired: 'noticeRecoveryRequired',
        reconciled: 'noticeRecoveryReconciled',
      } as const satisfies Record<typeof result.outcome, MessageKey>;
      setNotice(text(messageKeys[result.outcome]));
      queueMicrotask(() => ledgerHeading?.focus({ preventScroll: true }));
    } catch (cause) {
      setError(localizedError(locale(), cause, 'errorRecoveryActionFailed'));
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
      setError(localizedError(locale(), cause, 'errorRecoveryCancelFailed'));
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
        setNotice(text('noticeUndoCancelledNoChange'));
        return;
      }
      setUndoInspection(undefined);
      const messageKeys = {
        cancelled: 'noticeUndoCancelled',
        completed: 'noticeUndoCompleted',
        rolledBack: 'noticeUndoRolledBack',
        recoveryRequired: 'noticeUndoRecoveryRequired',
      } as const satisfies Record<typeof result.outcome, MessageKey>;
      setNotice(text(messageKeys[result.outcome]));
      queueMicrotask(() => ledgerHeading?.focus({ preventScroll: true }));
    } catch (cause) {
      setError(localizedError(locale(), cause, 'errorUndoRunFailed'));
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
      setError(localizedError(locale(), cause, 'errorUndoCancelFailed'));
    }
  };

  onMount(() => {
    try {
      const loaded = readPresetDocument(props.presetStorage ?? window.localStorage);
      setPresetDocument(loaded.document);
      if (loaded.migrated) {
        setNotice(text('noticePresetMigrated'));
      }
    } catch (cause) {
      setError(localizedError(locale(), cause, 'errorPresetLoadFailed'));
    }
    void planningClient
      .listLedger()
      .then(setLedger)
      .catch((cause: unknown) => {
        setError(localizedError(locale(), cause, 'errorLedgerLoadFailed'));
      });
    const stopWatching = planningClient.watchSourceChanges((change) => {
      if (change.error) {
        setError(text('errorSourceChangesFailed'));
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
      return text('statusUndoCancelAccepted');
    }
    if (undoCancellationState() === 'requesting') {
      return text('statusUndoCancelRequesting');
    }
    if (undoCancellationState() === 'rejected') {
      return text('statusUndoCancelRejected');
    }
    if (undoBusy()) {
      return text('statusUndoBusy');
    }
    if (recoveryCancellationState() === 'accepted') {
      return text('statusRecoveryCancelAccepted');
    }
    if (recoveryCancellationState() === 'requesting') {
      return text('statusRecoveryCancelRequesting');
    }
    if (recoveryCancellationState() === 'rejected') {
      return text('statusRecoveryCancelRejected');
    }
    if (recoveryBusyAction()) {
      return text('statusRecoveryBusy');
    }
    if (busy()) {
      return text('statusUpdatingPlan');
    }
    if (notice()) {
      return notice();
    }
    if (!current) {
      return text('statusNoSources');
    }
    if (current.blockedCount > 0) {
      return text('statusBlockedNames', { count: count(current.blockedCount) });
    }
    return text('statusReadyNames', { count: count(current.changedCount) });
  };

  const ledgerStatusLabel = (status: LedgerStatus) => {
    const labels: Record<LedgerStatus, MessageKey> = {
      completed: 'ledgerCompleted',
      rolledBack: 'ledgerRolledBack',
      forwardPending: 'ledgerForwardPending',
      completionPending: 'ledgerCompletionPending',
      rollbackPending: 'ledgerRollbackPending',
      rollbackCompletionPending: 'ledgerRollbackCompletionPending',
      reconciliationRequired: 'ledgerReconciliationRequired',
      recoveryRequired: 'ledgerRecoveryRequired',
      legacyInspectionRequired: 'ledgerLegacyInspectionRequired',
      torn: 'ledgerTorn',
      damaged: 'ledgerDamaged',
      unsupportedVersion: 'ledgerUnsupportedVersion',
      tooLarge: 'ledgerTooLarge',
      discoveryLimitExceeded: 'ledgerDiscoveryLimitExceeded',
      unreadable: 'ledgerUnreadable',
    };
    return text(labels[status]);
  };

  const ledgerStatusDescription = (status: LedgerStatus) => {
    if (status === 'completed') {
      return text('ledgerTerminalCompletedDescription');
    }
    if (status === 'rolledBack') {
      return text('ledgerTerminalRolledBackDescription');
    }
    if (status === 'reconciliationRequired') {
      return text('ledgerReconciliationDescription');
    }
    if (
      status === 'forwardPending' ||
      status === 'completionPending' ||
      status === 'rollbackPending' ||
      status === 'rollbackCompletionPending' ||
      status === 'recoveryRequired'
    ) {
      return text('ledgerInterruptedDescription');
    }
    return text('ledgerUnavailableDescription');
  };

  const dispositionLabel = (disposition: RecoveryDisposition | null) => {
    if (!disposition) {
      return '';
    }
    const labels: Record<RecoveryDisposition, MessageKey> = {
      notApplied: 'dispositionNotApplied',
      applied: 'dispositionApplied',
      missing: 'dispositionMissing',
      multipleLocations: 'dispositionMultiple',
      unexpectedLocation: 'dispositionUnexpected',
    };
    return text(labels[disposition]);
  };

  const recoveryInspectionTitle = () => {
    switch (recoveryInspection()?.readiness) {
      case 'ready':
        return text('identityChecksPassed');
      case 'reconciliationRequired':
        return text('observationReady');
      case 'blocked':
        return text('recoveryBlocked');
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
      return text('recoveryObservationConfirmation', {
        disposition: dispositionLabel(inspection.disposition),
      });
    }
    if (inspection.readiness === 'blocked') {
      return text('recoveryBlockedDescription');
    }
    return inspection.direction === 'forward'
      ? text('recoveryForwardDescription')
      : text('recoveryRollbackDescription');
  };

  const undoBlockDescription = (reason: UndoBlockReason | null) => {
    switch (reason) {
      case 'sourceChanged':
        return text('undoSourceChanged');
      case 'destinationOccupied':
        return text('undoDestinationOccupied');
      default:
        return text('undoSafetyFailed');
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
            <span class="brand-mode">{text('appTagline')}</span>
          </div>
        </div>
        <div class="source-actions">
          <label class="locale-control">
            <span>{text('localeLabel')}</span>
            <select
              aria-label={text('localeLabel')}
              value={locale()}
              onChange={(event) => selectLocale(event.currentTarget.value as Locale)}
            >
              <option value="en">{text('localeEnglish')}</option>
              <option value="ko">{text('localeKorean')}</option>
            </select>
          </label>
          <span class="read-only-badge">{text('recoveryOnlyMilestone')}</span>
          <span class="drop-hint">
            {planningClient.nativeSelectionAvailable
              ? text('dropFilesAnywhere')
              : text('desktopDropReady')}
          </span>
          <button
            class="button button-secondary"
            type="button"
            disabled={!planningClient.nativeSelectionAvailable || busy()}
            aria-describedby="native-selection-note"
            onClick={() => void run(() => planningClient.selectSources(currentRuleRequest()))}
          >
            {text('addFiles')}
          </button>
        </div>
      </header>

      <p id="native-selection-note" class="browser-note">
        <Show
          when={planningClient.nativeSelectionAvailable}
          fallback={text('fileSelectionDesktopOnly')}
        >
          {text('nativePathsStayInRust')}
        </Show>
      </p>

      <div class="workbench-grid">
        <aside class="rule-rail" aria-labelledby="rule-heading">
          <div class="rail-heading">
            <h1 id="rule-heading">{text('renameRules')}</h1>
            <span>
              {text('activeRules', {
                count: count(rules().filter((rule) => rule.enabled).length),
              })}
            </span>
          </div>
          <div class="rule-list">
            <For each={rules()} fallback={<p class="empty-rules">{text('noRules')}</p>}>
              {(rule, index) => {
                const inputId = (field: string) => `rule-${rule.ruleId}-${field}`;
                const invalid = () => ruleError()?.ruleId === rule.ruleId;
                const enabled = () =>
                  rules().find((candidate) => candidate.ruleId === rule.ruleId)?.enabled ?? false;
                const extensionReplacementEnabled = () => {
                  const current = rules().find((candidate) => candidate.ruleId === rule.ruleId);
                  return current?.kind === 'extension' && current.operation === 'replace';
                };
                const rangeOpenEnded = () => {
                  const current = rules().find((candidate) => candidate.ruleId === rule.ruleId);
                  return current?.kind === 'range' && current.length === null;
                };
                return (
                  <section
                    class="rule-editor"
                    aria-labelledby={inputId('heading')}
                    data-disabled={enabled() ? 'false' : 'true'}
                    data-invalid={invalid() ? 'true' : 'false'}
                  >
                    <div class="rule-title">
                      <div>
                        <span class="rule-order">{String(index() + 1).padStart(2, '0')}</span>
                        <h2 id={inputId('heading')}>{localizedRuleLabel(rule.kind)}</h2>
                      </div>
                      <div class="rule-actions">
                        <button
                          class="rule-icon-button"
                          type="button"
                          disabled={index() === 0}
                          aria-label={text('moveRuleUp', { rule: localizedRuleLabel(rule.kind) })}
                          onClick={() => moveRule(rule.ruleId, -1)}
                        >
                          ↑
                        </button>
                        <button
                          class="rule-icon-button"
                          type="button"
                          disabled={index() === rules().length - 1}
                          aria-label={text('moveRuleDown', { rule: localizedRuleLabel(rule.kind) })}
                          onClick={() => moveRule(rule.ruleId, 1)}
                        >
                          ↓
                        </button>
                        <button
                          class="rule-icon-button rule-remove-button"
                          type="button"
                          aria-label={text('removeRule', { rule: localizedRuleLabel(rule.kind) })}
                          onClick={() => removeRule(rule.ruleId)}
                        >
                          ×
                        </button>
                      </div>
                    </div>
                    <label class="rule-toggle">
                      <input
                        type="checkbox"
                        checked={enabled()}
                        onChange={(event) =>
                          replaceRule(rule.ruleId, (current) => ({
                            ...current,
                            enabled: event.currentTarget.checked,
                          }))
                        }
                      />
                      {text('enabled')}
                    </label>
                    <Switch>
                      <Match when={rule.kind === 'prefix' && rule}>
                        {(current) => (
                          <RuleTextInput
                            id={inputId('value')}
                            label={text('fieldPrefix')}
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
                            label={text('fieldSuffix')}
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
                              label={text('fieldFind')}
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
                              label={text('fieldReplaceWith')}
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
                              label={text('fieldRustRegex')}
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
                              label={text('fieldReplaceWith')}
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
                      <Match when={rule.kind === 'sequence' && rule}>
                        {(current) => (
                          <>
                            <div class="rule-field">
                              <label for={inputId('scope')}>{text('fieldNumberingScope')}</label>
                              <select
                                id={inputId('scope')}
                                value={current().scope}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'sequence'
                                      ? {
                                          ...candidate,
                                          scope: event.currentTarget.value as
                                            | 'allSources'
                                            | 'perParent',
                                        }
                                      : candidate
                                  )
                                }
                              >
                                <option value="allSources">{text('scopeAllSources')}</option>
                                <option value="perParent">{text('scopePerParent')}</option>
                              </select>
                            </div>
                            <div class="rule-field">
                              <label for={inputId('order')}>{text('fieldNumberBy')}</label>
                              <select
                                id={inputId('order')}
                                value={current().order}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'sequence'
                                      ? {
                                          ...candidate,
                                          order: event.currentTarget.value as
                                            | 'sourceOrder'
                                            | 'nameAscending',
                                        }
                                      : candidate
                                  )
                                }
                              >
                                <option value="sourceOrder">{text('orderSource')}</option>
                                <option value="nameAscending">{text('orderName')}</option>
                              </select>
                            </div>
                            <div class="sequence-number-fields">
                              <RuleNumberInput
                                id={inputId('start')}
                                label={text('fieldStart')}
                                value={current().start}
                                min={0}
                                max={Number.MAX_SAFE_INTEGER}
                                invalid={invalid()}
                                onInput={(start) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'sequence'
                                      ? { ...candidate, start }
                                      : candidate
                                  )
                                }
                              />
                              <RuleNumberInput
                                id={inputId('step')}
                                label={text('fieldStep')}
                                value={current().step}
                                min={1}
                                max={Number.MAX_SAFE_INTEGER}
                                invalid={invalid()}
                                onInput={(step) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'sequence'
                                      ? { ...candidate, step }
                                      : candidate
                                  )
                                }
                              />
                              <RuleNumberInput
                                id={inputId('padding')}
                                label={text('fieldPadding')}
                                value={current().padding}
                                min={1}
                                max={20}
                                invalid={invalid()}
                                onInput={(padding) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'sequence'
                                      ? { ...candidate, padding }
                                      : candidate
                                  )
                                }
                              />
                            </div>
                            <div class="rule-field">
                              <label for={inputId('placement')}>{text('fieldPlacement')}</label>
                              <select
                                id={inputId('placement')}
                                value={current().placement}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'sequence'
                                      ? {
                                          ...candidate,
                                          placement: event.currentTarget.value as
                                            | 'prefix'
                                            | 'suffix',
                                        }
                                      : candidate
                                  )
                                }
                              >
                                <option value="prefix">{text('placementPrefix')}</option>
                                <option value="suffix">{text('placementSuffix')}</option>
                              </select>
                            </div>
                            <RuleTextInput
                              id={inputId('separator')}
                              label={text('fieldSeparator')}
                              value={current().separator}
                              placeholder="-"
                              invalid={invalid()}
                              onInput={(separator) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'sequence'
                                    ? { ...candidate, separator }
                                    : candidate
                                )
                              }
                            />
                            <p class="field-help">{text('sequenceHelp')}</p>
                          </>
                        )}
                      </Match>
                      <Match when={rule.kind === 'extension' && rule}>
                        {(current) => (
                          <>
                            <div class="rule-field">
                              <label for={inputId('operation')}>{text('fieldOperation')}</label>
                              <select
                                id={inputId('operation')}
                                value={current().operation}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'extension'
                                      ? {
                                          ...candidate,
                                          operation: event.currentTarget.value as
                                            | 'remove'
                                            | 'replace',
                                        }
                                      : candidate
                                  )
                                }
                              >
                                <option value="remove">{text('extensionRemove')}</option>
                                <option value="replace">{text('extensionReplace')}</option>
                              </select>
                            </div>
                            <Show when={extensionReplacementEnabled()}>
                              <RuleTextInput
                                id={inputId('value')}
                                label={text('fieldNewExtension')}
                                value={current().value}
                                placeholder="txt"
                                invalid={invalid()}
                                onInput={(value) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'extension'
                                      ? { ...candidate, value }
                                      : candidate
                                  )
                                }
                              />
                            </Show>
                            <p class="field-help">{text('extensionHelp')}</p>
                          </>
                        )}
                      </Match>
                      <Match when={rule.kind === 'case' && rule}>
                        {(current) => (
                          <>
                            <FilenamePartSelect
                              id={inputId('target')}
                              value={current().target}
                              locale={locale()}
                              onChange={(target) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'case' ? { ...candidate, target } : candidate
                                )
                              }
                            />
                            <div class="rule-field">
                              <label for={inputId('mode')}>{text('fieldCase')}</label>
                              <select
                                id={inputId('mode')}
                                value={current().mode}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'case'
                                      ? {
                                          ...candidate,
                                          mode: event.currentTarget.value as
                                            | 'lowercase'
                                            | 'uppercase',
                                        }
                                      : candidate
                                  )
                                }
                              >
                                <option value="lowercase">{text('caseLowercase')}</option>
                                <option value="uppercase">{text('caseUppercase')}</option>
                              </select>
                            </div>
                          </>
                        )}
                      </Match>
                      <Match when={rule.kind === 'whitespaceCleanup' && rule}>
                        {(current) => (
                          <>
                            <FilenamePartSelect
                              id={inputId('target')}
                              value={current().target}
                              locale={locale()}
                              onChange={(target) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'whitespaceCleanup'
                                    ? { ...candidate, target }
                                    : candidate
                                )
                              }
                            />
                            <RuleTextInput
                              id={inputId('replacement')}
                              label={text('fieldCollapseRunsTo')}
                              value={current().replacement}
                              placeholder="-"
                              invalid={invalid()}
                              onInput={(replacement) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'whitespaceCleanup'
                                    ? { ...candidate, replacement }
                                    : candidate
                                )
                              }
                            />
                            <p class="field-help">{text('whitespaceHelp')}</p>
                          </>
                        )}
                      </Match>
                      <Match when={rule.kind === 'unicodeNormalization' && rule}>
                        {(current) => (
                          <>
                            <FilenamePartSelect
                              id={inputId('target')}
                              value={current().target}
                              locale={locale()}
                              onChange={(target) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'unicodeNormalization'
                                    ? { ...candidate, target }
                                    : candidate
                                )
                              }
                            />
                            <div class="rule-field">
                              <label for={inputId('form')}>{text('fieldNormalizationForm')}</label>
                              <select
                                id={inputId('form')}
                                value={current().form}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'unicodeNormalization'
                                      ? {
                                          ...candidate,
                                          form: event.currentTarget.value as
                                            | 'nfc'
                                            | 'nfd'
                                            | 'nfkc'
                                            | 'nfkd',
                                        }
                                      : candidate
                                  )
                                }
                              >
                                <option value="nfc">{text('normalizationNfc')}</option>
                                <option value="nfd">{text('normalizationNfd')}</option>
                                <option value="nfkc">{text('normalizationNfkc')}</option>
                                <option value="nfkd">{text('normalizationNfkd')}</option>
                              </select>
                            </div>
                            <p class="field-help">{text('normalizationHelp')}</p>
                          </>
                        )}
                      </Match>
                      <Match when={rule.kind === 'range' && rule}>
                        {(current) => (
                          <>
                            <FilenamePartSelect
                              id={inputId('target')}
                              value={current().target}
                              locale={locale()}
                              onChange={(target) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'range' ? { ...candidate, target } : candidate
                                )
                              }
                            />
                            <div class="rule-field">
                              <label for={inputId('operation')}>{text('fieldRangeAction')}</label>
                              <select
                                id={inputId('operation')}
                                value={current().operation}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'range'
                                      ? {
                                          ...candidate,
                                          operation: event.currentTarget.value as 'keep' | 'remove',
                                        }
                                      : candidate
                                  )
                                }
                              >
                                <option value="keep">{text('rangeKeep')}</option>
                                <option value="remove">{text('rangeRemove')}</option>
                              </select>
                            </div>
                            <div class="rule-field">
                              <label for={inputId('origin')}>{text('fieldCountFrom')}</label>
                              <select
                                id={inputId('origin')}
                                value={current().origin}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'range'
                                      ? {
                                          ...candidate,
                                          origin: event.currentTarget.value as 'start' | 'end',
                                        }
                                      : candidate
                                  )
                                }
                              >
                                <option value="start">{text('originStart')}</option>
                                <option value="end">{text('originEnd')}</option>
                              </select>
                            </div>
                            <div class="sequence-number-fields">
                              <RuleNumberInput
                                id={inputId('offset')}
                                label={text('fieldSkipCharacters')}
                                value={current().offset}
                                min={0}
                                max={4_294_967_295}
                                invalid={invalid()}
                                onInput={(offset) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'range'
                                      ? { ...candidate, offset }
                                      : candidate
                                  )
                                }
                              />
                              <RuleNumberInput
                                id={inputId('length')}
                                label={text('fieldRangeLength')}
                                value={current().length ?? 1}
                                min={1}
                                max={4_294_967_295}
                                invalid={invalid()}
                                disabled={rangeOpenEnded()}
                                onInput={(length) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'range'
                                      ? { ...candidate, length }
                                      : candidate
                                  )
                                }
                              />
                            </div>
                            <label class="rule-toggle">
                              <input
                                type="checkbox"
                                checked={rangeOpenEnded()}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'range'
                                      ? {
                                          ...candidate,
                                          length: event.currentTarget.checked ? null : 1,
                                        }
                                      : candidate
                                  )
                                }
                              />
                              {text('rangeOpenEnded')}
                            </label>
                            <p class="field-help">{text('rangeHelp')}</p>
                          </>
                        )}
                      </Match>
                      <Match when={rule.kind === 'characterClass' && rule}>
                        {(current) => (
                          <>
                            <FilenamePartSelect
                              id={inputId('target')}
                              value={current().target}
                              locale={locale()}
                              onChange={(target) =>
                                replaceRule(rule.ruleId, (candidate) =>
                                  candidate.kind === 'characterClass'
                                    ? { ...candidate, target }
                                    : candidate
                                )
                              }
                            />
                            <div class="rule-field">
                              <label for={inputId('operation')}>{text('fieldClassAction')}</label>
                              <select
                                id={inputId('operation')}
                                value={current().operation}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'characterClass'
                                      ? {
                                          ...candidate,
                                          operation: event.currentTarget.value as 'keep' | 'remove',
                                        }
                                      : candidate
                                  )
                                }
                              >
                                <option value="keep">{text('classKeep')}</option>
                                <option value="remove">{text('classRemove')}</option>
                              </select>
                            </div>
                            <div class="rule-field">
                              <label for={inputId('class')}>{text('fieldUnicodeClass')}</label>
                              <select
                                id={inputId('class')}
                                value={current().class}
                                onChange={(event) =>
                                  replaceRule(rule.ruleId, (candidate) =>
                                    candidate.kind === 'characterClass'
                                      ? {
                                          ...candidate,
                                          class: event.currentTarget.value as
                                            | 'decimalNumber'
                                            | 'letter'
                                            | 'whitespace'
                                            | 'punctuation'
                                            | 'symbol',
                                        }
                                      : candidate
                                  )
                                }
                              >
                                <option value="decimalNumber">{text('classDecimalNumber')}</option>
                                <option value="letter">{text('classLetter')}</option>
                                <option value="whitespace">{text('classWhitespace')}</option>
                                <option value="punctuation">{text('classPunctuation')}</option>
                                <option value="symbol">{text('classSymbol')}</option>
                              </select>
                            </div>
                            <p class="field-help">{text('characterClassHelp')}</p>
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
            <label for="new-rule-kind">{text('newRule')}</label>
            <div>
              <select
                id="new-rule-kind"
                value={newRuleKind()}
                onChange={(event) => setNewRuleKind(event.currentTarget.value as RuleKind)}
              >
                <option value="prefix">{text('newRulePrefix')}</option>
                <option value="suffix">{text('newRuleSuffix')}</option>
                <option value="literalReplace">{text('newRuleLiteralReplace')}</option>
                <option value="regexReplace">{text('newRuleRegexReplace')}</option>
                <option value="sequence">{text('newRuleSequence')}</option>
                <option value="extension">{text('newRuleExtension')}</option>
                <option value="case">{text('newRuleCase')}</option>
                <option value="whitespaceCleanup">{text('newRuleWhitespaceCleanup')}</option>
                <option value="unicodeNormalization">{text('newRuleUnicodeNormalization')}</option>
                <option value="range">{text('newRuleRange')}</option>
                <option value="characterClass">{text('newRuleCharacterClass')}</option>
              </select>
              <button
                class="button button-secondary"
                type="button"
                disabled={rules().length >= MAX_RULES}
                onClick={addRule}
              >
                {text('addRule')}
              </button>
            </div>
            <p class="field-help">{text('ruleLimitHelp', { count: count(MAX_RULES) })}</p>
          </div>
          <section class="preset-panel" aria-labelledby="preset-heading">
            <div class="preset-heading">
              <div>
                <h2 id="preset-heading">{text('localPresets')}</h2>
                <span>
                  {presetDocument().presets.length}/{MAX_PRESETS}
                </span>
              </div>
              <p>{text('presetScopeHelp')}</p>
            </div>
            <form
              class="preset-save"
              onSubmit={(event) => {
                event.preventDefault();
                savePreset();
              }}
            >
              <label for="preset-name">{text('presetName')}</label>
              <div>
                <input
                  id="preset-name"
                  type="text"
                  value={presetName()}
                  autocomplete="off"
                  onInput={(event) => setPresetName(event.currentTarget.value)}
                />
                <button
                  class="button button-secondary"
                  type="submit"
                  disabled={presetDocument().presets.length >= MAX_PRESETS}
                >
                  {text('savePreset')}
                </button>
              </div>
            </form>
            <Show
              when={presetDocument().presets.length > 0}
              fallback={<p class="preset-empty">{text('noLocalPresets')}</p>}
            >
              <ul aria-label={text('savedLocalPresets')}>
                <For each={presetDocument().presets}>
                  {(preset) => (
                    <li>
                      <div>
                        <strong>{preset.name}</strong>
                        <span>
                          {text('presetRuleCount', { count: count(preset.rules.length) })}
                        </span>
                      </div>
                      <div class="preset-actions">
                        <button
                          class="button button-secondary button-compact"
                          type="button"
                          onClick={() => applyPreset(preset.presetId)}
                        >
                          {text('applyPreset')}
                        </button>
                        <button
                          class="button button-secondary button-compact preset-delete"
                          type="button"
                          aria-label={text('deleteNamedPreset', { name: preset.name })}
                          onClick={() => removePreset(preset.presetId)}
                        >
                          {text('deletePreset')}
                        </button>
                      </div>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </section>
          <div class="scope-note">
            <strong>{text('currentScope')}</strong>
            <p>{text('currentScopeDescription')}</p>
          </div>
        </aside>
        <Show when={ledger().length > 0}>
          <section class="ledger-panel" aria-labelledby="ledger-heading">
            <div class="ledger-heading">
              <div>
                <span class="transaction-kicker">{text('renameLedger')}</span>
                <h2 id="ledger-heading" tabIndex={-1} ref={ledgerHeading}>
                  {text('transactionActivity')}
                </h2>
              </div>
              <span>{ledger().length}</span>
            </div>
            <p>{text('ledgerDescription')}</p>
            <p class="transaction-safety-boundary">{text('transactionSafetyBoundary')}</p>
            <ul aria-label={text('renameJournalStatus')}>
              {ledger().map((entry) => (
                <li>
                  <div class="ledger-entry-summary">
                    <strong>
                      {entry.undoOfPlanId !== null
                        ? text('undoOfPlan', { planId: entry.undoOfPlanId })
                        : entry.planId === null
                          ? text('ledgerNumber', { ledgerId: entry.ledgerId })
                          : text('planNumber', { planId: entry.planId })}
                    </strong>
                    <span>
                      {entry.undoOfPlanId !== null && entry.planId !== null
                        ? `${text('planNumber', { planId: entry.planId })} · `
                        : ''}
                      {text('sourceCount', { count: count(entry.sourceCount) })}
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
                        aria-label={text('inspectLedgerRecovery', {
                          subject:
                            entry.planId === null
                              ? text('subjectLedger', { id: entry.ledgerId })
                              : text('subjectPlan', { id: entry.planId }),
                        })}
                        onClick={() => void inspectLedgerEntry(entry)}
                      >
                        {inspectingLedgerId() === entry.ledgerId
                          ? text('inspecting')
                          : text('inspect')}
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
                        aria-label={text('inspectLedgerUndo', {
                          subject:
                            entry.planId === null
                              ? text('subjectLedger', { id: entry.ledgerId })
                              : text('subjectPlan', { id: entry.planId }),
                        })}
                        onClick={() => void inspectUndoEntry(entry)}
                      >
                        {inspectingUndoLedgerId() === entry.ledgerId
                          ? text('inspecting')
                          : text('inspectUndo')}
                      </button>
                    </Show>
                  </div>
                  <p class="ledger-entry-description">{ledgerStatusDescription(entry.status)}</p>
                </li>
              ))}
            </ul>
            <Show when={recoveryInspection()}>
              <section
                class="ledger-inspection"
                aria-labelledby="recovery-inspection-heading"
                ref={recoveryInspectionPanel}
              >
                <h3 id="recovery-inspection-heading">{recoveryInspectionTitle()}</h3>
                <p>{recoveryInspectionDescription()}</p>
                <span>
                  {recoveryInspection()?.direction === 'forward'
                    ? text('recoveryDirectionForward')
                    : text('recoveryDirectionRollback')}
                  {recoveryInspection()?.stepIndex === null
                    ? ` · ${text('recoveryTerminalRecord')}`
                    : ` · ${text('recoveryStep', { step: recoveryInspection()?.stepIndex ?? 0 })}`}
                </span>
                <fieldset class="ledger-recovery-actions">
                  <legend class="visually-hidden">{text('availableRecoveryActions')}</legend>
                  <Show when={recoveryInspection()?.reconcileAvailable}>
                    <button
                      class="button button-primary button-compact"
                      type="button"
                      disabled={recoveryBusyAction() !== undefined}
                      onClick={() => void applyRecoveryAction('reconcile')}
                    >
                      {recoveryBusyAction() === 'reconcile'
                        ? text('waitingForConfirmation')
                        : text('recordObservation')}
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
                        ? text('recovering')
                        : recoveryInspection()?.direction === 'forward'
                          ? text('resume')
                          : text('continueRollback')}
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
                        ? text('cancellationRequested')
                        : recoveryCancellationState() === 'requesting'
                          ? text('requestingCancellation')
                          : recoveryCancellationState() === 'rejected'
                            ? text('tryCancelAgain')
                            : text('cancelAndRollback')}
                    </button>
                  </Show>
                  <Show when={recoveryInspection()?.rollbackAvailable}>
                    <button
                      class="button button-secondary button-compact"
                      type="button"
                      disabled={recoveryBusyAction() !== undefined}
                      onClick={() => void applyRecoveryAction('rollback')}
                    >
                      {recoveryBusyAction() === 'rollback' ? text('rollingBack') : text('rollBack')}
                    </button>
                  </Show>
                </fieldset>
              </section>
            </Show>
            <Show when={undoInspection()}>
              {(inspection) => (
                <section
                  class="ledger-inspection ledger-undo-inspection"
                  data-readiness={inspection().readiness}
                  aria-labelledby="undo-inspection-heading"
                  ref={undoInspectionPanel}
                >
                  <h3 id="undo-inspection-heading">
                    {inspection().readiness === 'ready'
                      ? text('undoChecksPassed')
                      : text('undoBlocked')}
                  </h3>
                  <p>
                    {inspection().readiness === 'ready'
                      ? text('undoReadyDescription')
                      : undoBlockDescription(inspection().blockReason)}
                  </p>
                  <span>
                    {text('planNumber', { planId: inspection().originalPlanId })} ·{' '}
                    {text('sourceCount', { count: count(inspection().sourceCount) })}
                  </span>
                  <Show when={inspection().undoAvailable}>
                    <fieldset class="ledger-recovery-actions">
                      <legend class="visually-hidden">{text('availableUndoActions')}</legend>
                      <button
                        class="button button-primary button-compact ledger-action-button"
                        data-state={undoBusy() ? 'loading' : 'default'}
                        type="button"
                        disabled={undoBusy() || recoveryBusyAction() !== undefined}
                        onClick={() => void applyUndo()}
                      >
                        {undoBusy() ? text('undoing') : text('undoRename')}
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
                            ? text('cancellationRequested')
                            : undoCancellationState() === 'requesting'
                              ? text('requestingCancellation')
                              : undoCancellationState() === 'rejected'
                                ? text('tryCancelAgain')
                                : text('cancelAndRollback')}
                        </button>
                      </Show>
                    </fieldset>
                  </Show>
                </section>
              )}
            </Show>
          </section>
        </Show>

        <section class="preview-pane" aria-labelledby="preview-heading">
          <div class="preview-heading">
            <div>
              <h2 id="preview-heading">{text('proposedNames')}</h2>
              <p>{text('proposedNamesDescription')}</p>
            </div>
            <div class="plan-heading-actions">
              <span class="generation">
                {text('generation', { generation: count(plan()?.generation ?? 0) })}
              </span>
              <button
                class="button button-secondary button-compact"
                type="button"
                disabled={!plan() || busy()}
                onClick={(event) => {
                  planInspectorOpener = event.currentTarget;
                  void inspectCurrentPlan();
                }}
              >
                {text('inspectJson')}
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
                <h2>{text('noSourcesInPlan')}</h2>
                <p>{text('noSourcesDescription')}</p>
                <button
                  class="button button-primary"
                  type="button"
                  disabled={busy()}
                  onClick={() => void run(loadInitialSources)}
                >
                  {busy()
                    ? planningClient.nativeSelectionAvailable
                      ? text('opening')
                      : text('loading')
                    : planningClient.nativeSelectionAvailable
                      ? text('addFiles')
                      : text('loadSample')}
                </button>
              </div>
            }
          >
            <VirtualPlanTable
              rows={plan()?.rows ?? []}
              overrides={overrides()}
              locale={locale()}
              onOverride={(sourceId, value) => {
                setOverrides((current) => {
                  const retained = current.filter((item) => item.sourceId !== sourceId);
                  return value === undefined ? retained : [...retained, { sourceId, value }];
                });
                schedulePreview();
              }}
            />
          </Show>
        </section>
      </div>

      <footer class="review-bar">
        <div class="plan-summary" aria-hidden={!plan()}>
          <span>
            <strong>{count(plan()?.rows.length ?? 0)}</strong> {text('summarySources')}
          </span>
          <span>
            <strong>{count(plan()?.changedCount ?? 0)}</strong> {text('summaryChanges')}
          </span>
          <span class={plan()?.blockedCount ? 'blocked-count' : ''}>
            <strong>{count(plan()?.blockedCount ?? 0)}</strong> {text('summaryBlocked')}
          </span>
        </div>
        <div class="execution-lock">
          <span>{text('executionLockedDescription')}</span>
          <button class="button button-locked" type="button" disabled>
            {text('executionUnavailable')}
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
                <span class="inspector-kicker">{text('versionedPlanDocument')}</span>
                <h2 id="plan-inspector-heading">
                  {text('planNumber', { planId: plan()?.planId ?? 0 })}
                </h2>
              </div>
              <button
                class="button button-secondary button-compact"
                type="button"
                onClick={dismissPlanInspector}
              >
                {text('close')}
              </button>
            </div>
            <p>{text('planProjectionDescription')}</p>
            <pre>{document()}</pre>
            <div class="inspector-actions">
              <button
                class="button button-primary"
                type="button"
                disabled={!planningClient.nativeSelectionAvailable || busy()}
                title={
                  planningClient.nativeSelectionAvailable ? undefined : text('exportDesktopOnly')
                }
                onClick={() => void exportCurrentPlan()}
              >
                {text('exportJson')}
              </button>
              <button
                class="button button-secondary"
                type="button"
                disabled={!planningClient.nativeSelectionAvailable || busy()}
                title={
                  planningClient.nativeSelectionAvailable ? undefined : text('csvExportDesktopOnly')
                }
                onClick={() => void exportCurrentPlanCsv()}
              >
                {text('exportCsv')}
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
