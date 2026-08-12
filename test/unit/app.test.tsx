import { cleanup, render, screen, within } from '@solidjs/testing-library';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import { App as RenamewrightApp } from '../../src/App';
import { LOCALE_STORAGE_KEY, type LocaleStorage } from '../../src/i18n/catalog';
import type {
  Plan,
  PlanningClient,
  RecoveryCommandResult,
  SourceChange,
} from '../../src/planning/client';
import { PRESET_STORAGE_KEY, type PresetStorage } from '../../src/planning/presets';
import {
  compileBrowserRulePipeline,
  createRule,
  RULE_PIPELINE_SCHEMA_VERSION,
  type RulePipelineRequest,
} from '../../src/planning/rules';

const sources = ['invoice.pdf', 'CON.txt', 'notes.txt'];

class TestPresetStorage implements PresetStorage {
  private readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }

  clear(): void {
    this.values.clear();
  }
}

const presetStorage = new TestPresetStorage();
const localeStorage = new TestPresetStorage();

function App(props: { client: PlanningClient; localeStorage?: LocaleStorage }) {
  return (
    <RenamewrightApp
      client={props.client}
      presetStorage={presetStorage}
      localeStorage={props.localeStorage ?? localeStorage}
    />
  );
}

afterEach(() => {
  cleanup();
  presetStorage.clear();
  localeStorage.clear();
  vi.useRealTimers();
});

const emptyRequest = (): RulePipelineRequest => ({
  schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
  overrides: [],
  rules: [createRule(1, 'prefix')],
});

function makePlan(request: RulePipelineRequest): Plan {
  const ruleSources = sources.map((originalName, index) => ({
    sourceId: index + 1,
    parentId: 1,
    originalName,
  }));
  const apply = compileBrowserRulePipeline(request, ruleSources);
  const rows = ruleSources.map((source) => {
    const { sourceId, originalName } = source;
    const { proposedName, diagnostic, overrideApplied } = apply(source);
    const blocked =
      diagnostic !== undefined ||
      proposedName.includes('?') ||
      proposedName.toUpperCase() === 'CON.TXT';
    return {
      sourceId,
      originalName,
      proposedName,
      overrideApplied,
      status: blocked
        ? ('blocked' as const)
        : proposedName === originalName
          ? ('unchanged' as const)
          : ('changed' as const),
      diagnostics: diagnostic ? [diagnostic] : blocked ? ['illegalCharacter'] : [],
    };
  });

  return {
    planId: 9,
    generation: 1,
    rows,
    changedCount: rows.filter((row) => row.status === 'changed').length,
    blockedCount: rows.filter((row) => row.status === 'blocked').length,
    canApply: rows.every((row) => row.status !== 'blocked'),
  };
}

function fakeClient(): PlanningClient {
  return {
    nativeSelectionAvailable: false,
    loadSample: async (request) => makePlan(request),
    selectSources: async (request) => makePlan(request),
    previewRules: async (request) => makePlan(request),
    inspectPlan: async (planId) =>
      JSON.stringify({ schemaVersion: 5, planId, rows: makePlan(emptyRequest()).rows }, null, 2),
    exportPlan: async () => false,
    exportPlanCsv: async () => false,
    listLedger: async () => [],
    inspectRecovery: async () => {
      throw new Error('No recovery fixture was configured.');
    },
    applyRecoveryAction: async () => {
      throw new Error('No recovery action fixture was configured.');
    },
    cancelRecovery: async () => false,
    inspectUndo: async () => {
      throw new Error('No Undo fixture was configured.');
    },
    applyUndo: async () => {
      throw new Error('No Undo action fixture was configured.');
    },
    cancelUndo: async () => false,
    watchSourceChanges: () => () => undefined,
  };
}

test('runs a path-free startup ledger action only after inspection', async () => {
  const user = userEvent.setup();
  const client = fakeClient();
  client.listLedger = async () => [
    {
      ledgerId: 1,
      planId: 67,
      sourceGeneration: 3,
      schemaVersion: 2,
      sourceCount: 4,
      status: 'reconciliationRequired',
      attentionStep: 2,
      recoveryAvailable: true,
      undoOfPlanId: null,
      undoAvailable: false,
    },
  ];
  const inspection = {
    ledgerId: 1,
    direction: 'forward',
    stepIndex: 2,
    readiness: 'reconciliationRequired',
    disposition: 'notApplied',
    resumeAvailable: false,
    rollbackAvailable: false,
    reconcileAvailable: true,
  } as const;
  client.inspectRecovery = async () => inspection;
  const applyRecoveryAction = vi.fn(async () => ({
    performed: true,
    outcome: 'reconciled' as const,
    ledger: [
      {
        ledgerId: 1,
        planId: 67,
        sourceGeneration: 3,
        schemaVersion: 2,
        sourceCount: 4,
        status: 'forwardPending' as const,
        attentionStep: 2,
        recoveryAvailable: true,
        undoOfPlanId: null,
        undoAvailable: false,
      },
    ],
  }));
  client.applyRecoveryAction = applyRecoveryAction;

  render(() => <App client={client} />);

  expect(await screen.findByRole('heading', { name: 'Rename Ledger' })).toBeInTheDocument();
  expect(screen.getByText('Plan 67')).toBeInTheDocument();
  expect(screen.getByText('4 sources')).toBeInTheDocument();
  expect(screen.getByText('Inspection required')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Inspect plan 67 recovery' }));
  expect(await screen.findByText('Observation ready to record')).toBeInTheDocument();
  expect(screen.getByText(/prepared rename was not applied/iu)).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Record observation' }));
  expect(applyRecoveryAction).toHaveBeenCalledWith('reconcile', inspection);
  expect(await screen.findByText('Forward recovery pending')).toBeInTheDocument();
  expect(screen.getByRole('status')).toHaveTextContent(
    'Prepared-step observation recorded. Inspect the transaction again.'
  );
  expect(screen.queryByText('Observation ready to record')).not.toBeInTheDocument();
});

test('offers resume and rollback only when the fresh inspection allows them', async () => {
  const user = userEvent.setup();
  const client = fakeClient();
  client.listLedger = async () => [
    {
      ledgerId: 2,
      planId: 72,
      sourceGeneration: 4,
      schemaVersion: 2,
      sourceCount: 2,
      status: 'forwardPending',
      attentionStep: 1,
      recoveryAvailable: true,
      undoOfPlanId: null,
      undoAvailable: false,
    },
  ];
  client.inspectRecovery = async () => ({
    ledgerId: 2,
    direction: 'forward',
    stepIndex: 1,
    readiness: 'ready',
    disposition: null,
    resumeAvailable: true,
    rollbackAvailable: true,
    reconcileAvailable: false,
  });

  render(() => <App client={client} />);

  await user.click(await screen.findByRole('button', { name: 'Inspect plan 72 recovery' }));
  expect(await screen.findByRole('button', { name: 'Resume' })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Roll back' })).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Record observation' })).not.toBeInTheDocument();
});

test('requests cancellation only while forward recovery is active', async () => {
  const user = userEvent.setup();
  const client = fakeClient();
  client.listLedger = async () => [
    {
      ledgerId: 3,
      planId: 73,
      sourceGeneration: 5,
      schemaVersion: 2,
      sourceCount: 2,
      status: 'forwardPending',
      attentionStep: 1,
      recoveryAvailable: true,
      undoOfPlanId: null,
      undoAvailable: false,
    },
  ];
  const inspection = {
    ledgerId: 3,
    direction: 'forward',
    stepIndex: 1,
    readiness: 'ready',
    disposition: null,
    resumeAvailable: true,
    rollbackAvailable: true,
    reconcileAvailable: false,
  } as const;
  client.inspectRecovery = async () => inspection;
  let finishRecovery: ((result: RecoveryCommandResult) => void) | undefined;
  client.applyRecoveryAction = vi.fn(
    () =>
      new Promise<RecoveryCommandResult>((resolve) => {
        finishRecovery = resolve;
      })
  );
  const cancelRecovery = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
  client.cancelRecovery = cancelRecovery;

  render(() => <App client={client} />);

  await user.click(await screen.findByRole('button', { name: 'Inspect plan 73 recovery' }));
  await user.click(await screen.findByRole('button', { name: 'Resume' }));
  await user.click(await screen.findByRole('button', { name: 'Cancel and roll back' }));

  expect(
    screen.getByText(
      'Cancellation was not confirmed, or forward recovery is no longer active. Try again if the operation is still running.',
      { selector: '.live-status' }
    )
  ).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Try cancel again' }));

  expect(cancelRecovery).toHaveBeenCalledTimes(2);
  expect(screen.getByRole('button', { name: 'Cancellation requested' })).toBeDisabled();
  expect(
    screen.getByText('Cancellation requested. Renamewright will roll back at the next safe step…', {
      selector: '.live-status',
    })
  ).toBeInTheDocument();

  finishRecovery?.({
    performed: true,
    outcome: 'rolledBack',
    ledger: [
      {
        ledgerId: 3,
        planId: 73,
        sourceGeneration: 5,
        schemaVersion: 2,
        sourceCount: 2,
        status: 'rolledBack',
        attentionStep: null,
        recoveryAvailable: false,
        undoOfPlanId: null,
        undoAvailable: false,
      },
    ],
  });
  expect(await screen.findByText('Rolled back')).toBeInTheDocument();
  expect(
    screen.getByText('The interrupted rename transaction was rolled back.', {
      selector: '.live-status',
    })
  ).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Cancel and roll back' })).not.toBeInTheDocument();
});

test('runs Undo only from a fresh path-free inspection', async () => {
  const user = userEvent.setup();
  const client = fakeClient();
  client.listLedger = async () => [
    {
      ledgerId: 4,
      planId: 80,
      sourceGeneration: 6,
      schemaVersion: 3,
      sourceCount: 3,
      status: 'completed',
      attentionStep: null,
      recoveryAvailable: false,
      undoOfPlanId: null,
      undoAvailable: true,
    },
  ];
  const inspection = {
    ledgerId: 4,
    originalPlanId: 80,
    sourceCount: 3,
    readiness: 'ready',
    blockReason: null,
    undoAvailable: true,
  } as const;
  client.inspectUndo = async () => inspection;
  const applyUndo = vi.fn(async () => ({
    performed: true,
    outcome: 'completed' as const,
    ledger: [
      {
        ledgerId: 4,
        planId: 80,
        sourceGeneration: 6,
        schemaVersion: 3,
        sourceCount: 3,
        status: 'completed' as const,
        attentionStep: null,
        recoveryAvailable: false,
        undoOfPlanId: null,
        undoAvailable: false,
      },
      {
        ledgerId: 5,
        planId: 81,
        sourceGeneration: 6,
        schemaVersion: 3,
        sourceCount: 3,
        status: 'completed' as const,
        attentionStep: null,
        recoveryAvailable: false,
        undoOfPlanId: 80,
        undoAvailable: false,
      },
    ],
  }));
  client.applyUndo = applyUndo;

  render(() => <App client={client} />);

  await user.click(await screen.findByRole('button', { name: 'Inspect plan 80 Undo' }));
  expect(await screen.findByText('Undo checks passed')).toBeInTheDocument();
  expect(screen.getByText('Plan 80 · 3 sources')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Undo rename' }));

  expect(applyUndo).toHaveBeenCalledWith(inspection);
  expect(await screen.findByText('Undo of plan 80')).toBeInTheDocument();
  expect(screen.getByRole('status')).toHaveTextContent(
    'The completed rename transaction was undone.'
  );
  expect(screen.queryByText('Undo checks passed')).not.toBeInTheDocument();
});

test('explains why a fresh Undo inspection is blocked', async () => {
  const user = userEvent.setup();
  const client = fakeClient();
  client.listLedger = async () => [
    {
      ledgerId: 5,
      planId: 82,
      sourceGeneration: 7,
      schemaVersion: 3,
      sourceCount: 1,
      status: 'completed',
      attentionStep: null,
      recoveryAvailable: false,
      undoOfPlanId: null,
      undoAvailable: true,
    },
  ];
  client.inspectUndo = async () => ({
    ledgerId: 5,
    originalPlanId: 82,
    sourceCount: 1,
    readiness: 'blocked',
    blockReason: 'destinationOccupied',
    undoAvailable: false,
  });

  render(() => <App client={client} />);

  await user.click(await screen.findByRole('button', { name: 'Inspect plan 82 Undo' }));
  expect(await screen.findByText('Undo is blocked')).toBeInTheDocument();
  expect(screen.getByText(/original name is occupied/iu)).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Undo rename' })).not.toBeInTheDocument();
});

test('refreshes the ledger after an Undo command error', async () => {
  const user = userEvent.setup();
  const client = fakeClient();
  const original = {
    ledgerId: 6,
    planId: 84,
    sourceGeneration: 8,
    schemaVersion: 3,
    sourceCount: 1,
    status: 'completed' as const,
    attentionStep: null,
    recoveryAvailable: false,
    undoOfPlanId: null,
    undoAvailable: true,
  };
  const interruptedUndo = {
    ledgerId: 7,
    planId: 85,
    sourceGeneration: 8,
    schemaVersion: 3,
    sourceCount: 1,
    status: 'recoveryRequired' as const,
    attentionStep: 0,
    recoveryAvailable: true,
    undoOfPlanId: 84,
    undoAvailable: false,
  };
  client.listLedger = vi
    .fn()
    .mockResolvedValueOnce([original])
    .mockResolvedValueOnce([{ ...original, undoAvailable: false }, interruptedUndo]);
  client.inspectUndo = async () => ({
    ledgerId: 6,
    originalPlanId: 84,
    sourceCount: 1,
    readiness: 'ready',
    blockReason: null,
    undoAvailable: true,
  });
  client.applyUndo = async () => {
    throw new Error('Undo stopped safely. Inspect the Rename Ledger before continuing.');
  };

  render(() => <App client={client} />);

  await user.click(await screen.findByRole('button', { name: 'Inspect plan 84 Undo' }));
  await user.click(await screen.findByRole('button', { name: 'Undo rename' }));

  expect(await screen.findByText('Undo of plan 84')).toBeInTheDocument();
  expect(screen.getByText('Recovery required')).toBeInTheDocument();
  expect(screen.queryByText('Undo checks passed')).not.toBeInTheDocument();
  expect(screen.getByRole('status')).toHaveTextContent('Undo could not run.');
});

test('loads sample sources and previews a prefix rule', async () => {
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  expect(screen.getByRole('heading', { name: 'No sources in this plan' })).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Load sample' }));

  expect((await screen.findAllByText('invoice.pdf')).length).toBeGreaterThan(0);
  const prefix = screen.getByRole('textbox', { name: 'Prefix' });
  await user.type(prefix, '2026-');

  expect(await screen.findByText('2026-invoice.pdf')).toBeInTheDocument();
  expect(
    screen.getByText((_, element) => element?.textContent === '3 changes')
  ).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Execution unavailable' })).toBeDisabled();
});

test('loads and persists Korean while localizing accessible workbench names', async () => {
  localeStorage.setItem(LOCALE_STORAGE_KEY, 'ko');
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  expect(document.documentElement.lang).toBe('ko');
  expect(screen.getByRole('heading', { name: '이름 변경 규칙' })).toBeInTheDocument();
  expect(screen.getByRole('combobox', { name: '언어' })).toHaveValue('ko');
  await user.click(screen.getByRole('button', { name: '샘플 불러오기' }));
  expect(await screen.findByRole('columnheader', { name: '원본' })).toBeInTheDocument();
  expect(screen.getByText('Windows에서 사용할 수 없는 문자')).toBeInTheDocument();
});

test('switches locale without reflecting an unknown backend error', async () => {
  const client = fakeClient();
  client.loadSample = async () => {
    throw new Error('/home/private/Documents/secret.txt');
  };
  const user = userEvent.setup();
  render(() => <App client={client} />);

  await user.selectOptions(screen.getByRole('combobox', { name: 'Language' }), 'ko');
  expect(localeStorage.getItem(LOCALE_STORAGE_KEY)).toBe('ko');
  await user.click(screen.getByRole('button', { name: '샘플 불러오기' }));

  expect(await screen.findByRole('status')).toHaveTextContent(
    '이름 변경 계획을 갱신할 수 없습니다.'
  );
  expect(screen.getByRole('status')).not.toHaveTextContent('/home/private');
});

test('adds, reorders, and disables rules without losing stable editing state', async () => {
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  await user.click(screen.getByRole('button', { name: 'Load sample' }));
  await user.type(screen.getByRole('textbox', { name: 'Prefix' }), 'draft-');
  await user.selectOptions(screen.getByRole('combobox', { name: 'New rule' }), 'literalReplace');
  await user.click(screen.getByRole('button', { name: 'Add rule' }));

  const replaceEditor = screen.getByRole('heading', { name: 'Replace text' }).closest('section');
  if (!replaceEditor) {
    throw new Error('Replace editor was not rendered.');
  }
  await user.type(within(replaceEditor).getByRole('textbox', { name: 'Find' }), 'draft');
  await user.type(within(replaceEditor).getByRole('textbox', { name: 'Replace with' }), 'final');
  expect(await screen.findByText('final-invoice.pdf')).toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: 'Move Replace text up' }));
  expect(await screen.findByText('draft-invoice.pdf')).toBeInTheDocument();

  const prefixEditor = screen.getByRole('heading', { name: 'Add prefix' }).closest('section');
  if (!prefixEditor) {
    throw new Error('Prefix editor was not rendered.');
  }
  await user.click(within(prefixEditor).getByRole('checkbox', { name: 'Enabled' }));
  expect(prefixEditor).toHaveAttribute('data-disabled', 'true');
  expect(within(prefixEditor).getByRole('checkbox', { name: 'Enabled' })).not.toBeChecked();
  expect(await screen.findByRole('button', { name: 'Unchanged 2' })).toBeInTheDocument();
  expect(screen.getAllByRole('cell', { name: /Unchanged/u })).toHaveLength(2);
});

test('edits sequence allocation independently from preview row order', async () => {
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  await user.click(screen.getByRole('button', { name: 'Load sample' }));
  await user.selectOptions(screen.getByRole('combobox', { name: 'New rule' }), 'sequence');
  await user.click(screen.getByRole('button', { name: 'Add rule' }));
  const sequenceEditor = screen.getByRole('heading', { name: 'Add sequence' }).closest('section');
  if (!sequenceEditor) {
    throw new Error('Sequence editor was not rendered.');
  }

  await user.selectOptions(
    within(sequenceEditor).getByRole('combobox', { name: 'Number by' }),
    'nameAscending'
  );
  await user.clear(within(sequenceEditor).getByRole('spinbutton', { name: 'Start' }));
  await user.type(within(sequenceEditor).getByRole('spinbutton', { name: 'Start' }), '5');
  await user.clear(within(sequenceEditor).getByRole('spinbutton', { name: 'Step' }));
  await user.type(within(sequenceEditor).getByRole('spinbutton', { name: 'Step' }), '5');
  await user.clear(within(sequenceEditor).getByRole('spinbutton', { name: 'Padding' }));
  await user.type(within(sequenceEditor).getByRole('spinbutton', { name: 'Padding' }), '2');

  expect(await screen.findByText('10-invoice.pdf')).toBeInTheDocument();
  expect(screen.getByText('15-notes.txt')).toBeInTheDocument();
  expect(screen.getByText('05-CON.txt')).toBeInTheDocument();
  expect(sequenceEditor).toHaveTextContent(
    'Number allocation is fixed before preview rows are rendered.'
  );
});

test('edits filename structure rules as one ordered pipeline', async () => {
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  await user.click(screen.getByRole('button', { name: 'Load sample' }));
  await user.type(screen.getByRole('textbox', { name: 'Prefix' }), '  Final   ');

  await user.selectOptions(screen.getByRole('combobox', { name: 'New rule' }), 'extension');
  await user.click(screen.getByRole('button', { name: 'Add rule' }));
  const extensionEditor = screen
    .getByRole('heading', { name: 'Change extension' })
    .closest('section');
  if (!extensionEditor) {
    throw new Error('Extension editor was not rendered.');
  }
  await user.selectOptions(
    within(extensionEditor).getByRole('combobox', { name: 'Operation' }),
    'replace'
  );
  const extension = within(extensionEditor).getByRole('textbox', {
    name: 'New extension (without dot)',
  });
  await user.clear(extension);
  await user.type(extension, 'md');

  await user.selectOptions(screen.getByRole('combobox', { name: 'New rule' }), 'whitespaceCleanup');
  await user.click(screen.getByRole('button', { name: 'Add rule' }));
  const whitespaceEditor = screen
    .getByRole('heading', { name: 'Clean whitespace' })
    .closest('section');
  if (!whitespaceEditor) {
    throw new Error('Whitespace editor was not rendered.');
  }
  const replacement = within(whitespaceEditor).getByRole('textbox', {
    name: 'Collapse runs to',
  });
  await user.clear(replacement);
  await user.type(replacement, '-');
  expect(await screen.findByText('Final-invoice.md')).toBeInTheDocument();

  await user.selectOptions(screen.getByRole('combobox', { name: 'New rule' }), 'case');
  await user.click(screen.getByRole('button', { name: 'Add rule' }));
  const caseEditor = screen.getByRole('heading', { name: 'Change case' }).closest('section');
  if (!caseEditor) {
    throw new Error('Case editor was not rendered.');
  }
  await user.selectOptions(
    within(caseEditor).getByRole('combobox', { name: 'Apply to' }),
    'extension'
  );
  await user.selectOptions(within(caseEditor).getByRole('combobox', { name: 'Case' }), 'uppercase');
  expect(await screen.findByText('Final-invoice.MD')).toBeInTheDocument();

  await user.selectOptions(
    screen.getByRole('combobox', { name: 'New rule' }),
    'unicodeNormalization'
  );
  await user.click(screen.getByRole('button', { name: 'Add rule' }));
  const normalizationEditor = screen
    .getByRole('heading', { name: 'Normalize Unicode' })
    .closest('section');
  if (!normalizationEditor) {
    throw new Error('Unicode normalization editor was not rendered.');
  }
  expect(
    within(normalizationEditor).getByRole('combobox', { name: 'Normalization form' })
  ).toHaveValue('nfc');
  expect(normalizationEditor).toHaveTextContent(
    'Normalization changes names only while this rule is enabled.'
  );
});

test('edits range and Unicode character-class rules in the rendered pipeline', async () => {
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  await user.click(screen.getByRole('button', { name: 'Load sample' }));
  await user.selectOptions(screen.getByRole('combobox', { name: 'New rule' }), 'range');
  await user.click(screen.getByRole('button', { name: 'Add rule' }));
  const rangeEditor = screen
    .getByRole('heading', { name: 'Select character range' })
    .closest('section');
  if (!rangeEditor) {
    throw new Error('Range editor was not rendered.');
  }
  const rangeLength = within(rangeEditor).getByRole('spinbutton', { name: 'Range length' });
  await user.clear(rangeLength);
  await user.type(rangeLength, '3');
  expect(await screen.findByText('inv.pdf')).toBeInTheDocument();

  await user.selectOptions(screen.getByRole('combobox', { name: 'New rule' }), 'characterClass');
  await user.click(screen.getByRole('button', { name: 'Add rule' }));
  const classEditor = screen
    .getByRole('heading', { name: 'Filter character class' })
    .closest('section');
  if (!classEditor) {
    throw new Error('Character-class editor was not rendered.');
  }
  await user.selectOptions(
    within(classEditor).getByRole('combobox', { name: 'Class action' }),
    'keep'
  );
  await user.selectOptions(
    within(classEditor).getByRole('combobox', { name: 'Unicode class' }),
    'letter'
  );
  expect(await screen.findByText('inv.pdf')).toBeInTheDocument();
  expect(classEditor).toHaveTextContent('Uses Unicode properties, not ASCII-only ranges.');
});

test('keeps an inline source override stable until it is reset', async () => {
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  await user.click(screen.getByRole('button', { name: 'Load sample' }));
  await user.click(screen.getByRole('button', { name: 'Edit override for invoice.pdf' }));
  const override = screen.getByRole('textbox', { name: 'Override name for invoice.pdf' });
  await user.clear(override);
  await user.type(override, 'manual.md');
  await user.click(screen.getByRole('button', { name: /^Save$/u }));
  expect(await screen.findByText('manual.md')).toBeInTheDocument();
  expect(screen.getByText('Override')).toBeInTheDocument();

  await user.type(screen.getByRole('textbox', { name: 'Prefix' }), 'shared-');
  expect(await screen.findByText('shared-notes.txt')).toBeInTheDocument();
  expect(screen.getByText('manual.md')).toBeInTheDocument();
  expect(screen.queryByText('shared-invoice.pdf')).not.toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: 'Reset override for invoice.pdf' }));
  expect(await screen.findByText('shared-invoice.pdf')).toBeInTheDocument();
  expect(screen.queryByText('manual.md')).not.toBeInTheDocument();
});

test('saves, applies, and deletes local presets without clearing source overrides', async () => {
  vi.useFakeTimers();
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  render(() => <App client={fakeClient()} />);

  await user.click(screen.getByRole('button', { name: 'Load sample' }));
  const prefix = screen.getByRole('textbox', { name: 'Prefix' });
  await user.type(prefix, 'saved-');
  await user.click(screen.getByRole('button', { name: 'Edit override for invoice.pdf' }));
  const override = screen.getByRole('textbox', { name: 'Override name for invoice.pdf' });
  await user.clear(override);
  await user.type(override, 'manual.md');
  await user.click(screen.getByRole('button', { name: /^Save$/u }));
  await vi.advanceTimersByTimeAsync(120);
  expect(screen.getByText('manual.md')).toBeInTheDocument();

  await user.type(screen.getByRole('textbox', { name: 'Preset name' }), 'Reports');
  await user.click(screen.getByRole('button', { name: 'Save preset' }));
  expect(screen.getByRole('status')).toHaveTextContent('Local rule preset saved.');

  await user.clear(prefix);
  await user.type(prefix, 'other-');
  await vi.advanceTimersByTimeAsync(120);
  expect(screen.getByText('other-notes.txt')).toBeInTheDocument();
  expect(screen.getByText('manual.md')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Apply' }));
  await vi.advanceTimersByTimeAsync(120);
  expect(screen.getByText('saved-notes.txt')).toBeInTheDocument();
  expect(screen.getByText('manual.md')).toBeInTheDocument();
  expect(screen.getByRole('status')).toHaveTextContent('Source overrides were preserved.');

  await user.click(screen.getByRole('button', { name: 'Delete preset Reports' }));
  expect(screen.getByText('No local presets saved.')).toBeInTheDocument();
  expect(screen.getByText('saved-notes.txt')).toBeInTheDocument();
  expect(screen.getByRole('status')).toHaveTextContent('current rules were not changed');
  expect(JSON.parse(presetStorage.getItem(PRESET_STORAGE_KEY) ?? '{}')).toMatchObject({
    schemaVersion: 2,
    presets: [],
  });
});

test('migrates schema-one presets before they can replace the active pipeline', async () => {
  presetStorage.setItem(
    PRESET_STORAGE_KEY,
    JSON.stringify({
      schemaVersion: 1,
      presets: [
        {
          name: 'Legacy',
          ruleSchemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
          rules: [{ kind: 'prefix', ruleId: 41, enabled: true, value: 'legacy-' }],
        },
      ],
    })
  );
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  expect(await screen.findByText('Legacy')).toBeInTheDocument();
  expect(screen.getByRole('status')).toHaveTextContent('updated to the current format');
  await user.click(screen.getByRole('button', { name: 'Apply' }));
  expect(screen.getByRole('status')).toHaveTextContent('Preset “Legacy” applied.');
  await user.click(screen.getByRole('button', { name: 'Load sample' }));
  expect(await screen.findByText('legacy-invoice.pdf')).toBeInTheDocument();
  expect(JSON.parse(presetStorage.getItem(PRESET_STORAGE_KEY) ?? '{}')).toMatchObject({
    schemaVersion: 2,
    nextPresetId: 2,
    presets: [{ presetId: 1, name: 'Legacy' }],
  });
});

test('does not mutate active rules when local preset data is malformed', async () => {
  presetStorage.setItem(
    PRESET_STORAGE_KEY,
    JSON.stringify({ schemaVersion: 2, nextPresetId: 1, presets: '/private/value' })
  );
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  expect(await screen.findByRole('status')).toHaveTextContent(
    'The saved preset data is invalid and was not loaded.'
  );
  expect(screen.getByRole('status')).not.toHaveTextContent('/private/value');
  await user.click(screen.getByRole('button', { name: 'Load sample' }));
  expect((await screen.findAllByText('invoice.pdf')).length).toBeGreaterThan(0);
});

test('associates an invalid regex error with its rule editor', async () => {
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  await user.click(screen.getByRole('button', { name: 'Load sample' }));
  await user.selectOptions(screen.getByRole('combobox', { name: 'New rule' }), 'regexReplace');
  await user.click(screen.getByRole('button', { name: 'Add rule' }));
  const regexEditor = screen
    .getByRole('heading', { name: 'Replace by pattern' })
    .closest('section');
  if (!regexEditor) {
    throw new Error('Regex editor was not rendered.');
  }
  await user.type(within(regexEditor).getByRole('textbox', { name: 'Rust regex' }), '(');

  expect(
    await within(regexEditor).findByText('Rule 2 uses an invalid regular expression.')
  ).toBeInTheDocument();
  expect(within(regexEditor).getByRole('textbox', { name: 'Rust regex' })).toHaveAttribute(
    'aria-invalid',
    'true'
  );
  expect(screen.getByRole('status')).not.toHaveTextContent('/home/');
});

test('reports blocked destinations without enabling execution', async () => {
  const user = userEvent.setup();
  render(() => <App client={fakeClient()} />);

  await user.click(screen.getByRole('button', { name: 'Load sample' }));
  const prefix = screen.getByRole('textbox', { name: 'Prefix' });
  await user.type(prefix, '?');

  expect(
    await screen.findByText((_, element) => element?.textContent === '3 blocked')
  ).toBeInTheDocument();
  expect(screen.getAllByRole('cell', { name: /Blocked/u })).toHaveLength(3);
  expect(screen.getByRole('status')).toHaveTextContent('3 names are blocked');
});

test('uses the native picker instead of browser samples in the desktop shell', async () => {
  const user = userEvent.setup();
  const client = fakeClient();
  const selectSources = vi.fn(client.selectSources);
  client.nativeSelectionAvailable = true;
  client.selectSources = selectSources;
  render(() => <App client={client} />);

  const addButtons = screen.getAllByRole('button', { name: 'Add files' });
  await user.click(addButtons.at(-1) as HTMLButtonElement);

  expect(selectSources).toHaveBeenCalledWith(emptyRequest());
  expect((await screen.findAllByText('invoice.pdf')).length).toBeGreaterThan(0);
});

test('refreshes the plan after Rust admits dropped sources', async () => {
  let notify: ((change: SourceChange) => void) | undefined;
  const client = fakeClient();
  client.nativeSelectionAvailable = true;
  client.watchSourceChanges = (onChange) => {
    notify = onChange;
    return () => undefined;
  };
  const previewRules = vi.fn(client.previewRules);
  client.previewRules = previewRules;
  render(() => <App client={client} />);

  notify?.({ revision: 1, error: null });

  expect(previewRules).toHaveBeenCalledWith(emptyRequest());
  expect((await screen.findAllByText('invoice.pdf')).length).toBeGreaterThan(0);
});

test('inspects and exports only the current opaque plan ID', async () => {
  const user = userEvent.setup();
  const client = fakeClient();
  client.nativeSelectionAvailable = true;
  const inspectPlan = vi.fn(client.inspectPlan);
  const exportPlan = vi.fn(async () => true);
  const exportPlanCsv = vi.fn(async () => true);
  client.inspectPlan = inspectPlan;
  client.exportPlan = exportPlan;
  client.exportPlanCsv = exportPlanCsv;
  render(() => <App client={client} />);

  await user.click(
    screen.getAllByRole('button', { name: 'Add files' }).at(-1) as HTMLButtonElement
  );
  const inspectButton = screen.getByRole('button', { name: 'Inspect JSON' });
  await user.click(inspectButton);

  expect(inspectPlan).toHaveBeenCalledWith(9);
  expect(screen.getByRole('dialog', { name: 'Plan 9' })).toHaveTextContent('"schemaVersion": 5');
  await user.click(screen.getByRole('button', { name: 'Export JSON…' }));
  expect(exportPlan).toHaveBeenCalledWith(9);
  expect(screen.getByRole('status')).toHaveTextContent('Plan JSON exported.');
  await user.click(screen.getByRole('button', { name: 'Export CSV…' }));
  expect(exportPlanCsv).toHaveBeenCalledWith(9);
  expect(screen.getByRole('status')).toHaveTextContent('Plan CSV exported.');
  await user.click(screen.getByRole('button', { name: 'Close' }));
  expect(inspectButton).toHaveFocus();
});
