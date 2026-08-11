import { cleanup, render, screen } from '@solidjs/testing-library';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import { App } from '../../src/App';
import type {
  Plan,
  PlanningClient,
  RecoveryCommandResult,
  SourceChange,
} from '../../src/planning/client';

const sources = ['invoice.pdf', 'CON.txt', 'notes.txt'];

afterEach(cleanup);

function makePlan(prefix: string): Plan {
  const rows = sources.map((originalName, index) => {
    const proposedName = `${prefix}${originalName}`;
    const blocked = proposedName.includes('?') || proposedName.toUpperCase() === 'CON.TXT';
    return {
      sourceId: index + 1,
      originalName,
      proposedName,
      status: blocked ? ('blocked' as const) : ('changed' as const),
      diagnostics: blocked ? ['illegalCharacter'] : [],
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
    loadSample: async (prefix) => makePlan(prefix),
    selectSources: async (prefix) => makePlan(prefix),
    previewPrefix: async (prefix) => makePlan(prefix),
    inspectPlan: async (planId) =>
      JSON.stringify({ schemaVersion: 1, planId, rows: makePlan('').rows }, null, 2),
    exportPlan: async () => false,
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
  expect(screen.getByRole('status')).toHaveTextContent('Undo stopped safely');
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
  expect(prefix).toHaveAttribute('aria-invalid', 'true');
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

  expect(selectSources).toHaveBeenCalledWith('');
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
  const previewPrefix = vi.fn(client.previewPrefix);
  client.previewPrefix = previewPrefix;
  render(() => <App client={client} />);

  notify?.({ revision: 1, error: null });

  expect(previewPrefix).toHaveBeenCalledWith('');
  expect((await screen.findAllByText('invoice.pdf')).length).toBeGreaterThan(0);
});

test('inspects and exports only the current opaque plan ID', async () => {
  const user = userEvent.setup();
  const client = fakeClient();
  client.nativeSelectionAvailable = true;
  const inspectPlan = vi.fn(client.inspectPlan);
  const exportPlan = vi.fn(async () => true);
  client.inspectPlan = inspectPlan;
  client.exportPlan = exportPlan;
  render(() => <App client={client} />);

  await user.click(
    screen.getAllByRole('button', { name: 'Add files' }).at(-1) as HTMLButtonElement
  );
  const inspectButton = screen.getByRole('button', { name: 'Inspect JSON' });
  await user.click(inspectButton);

  expect(inspectPlan).toHaveBeenCalledWith(9);
  expect(screen.getByRole('dialog', { name: 'Plan 9' })).toHaveTextContent('"schemaVersion": 1');
  await user.click(screen.getByRole('button', { name: 'Export JSON…' }));
  expect(exportPlan).toHaveBeenCalledWith(9);
  expect(screen.getByRole('status')).toHaveTextContent('Plan JSON exported.');
  await user.click(screen.getByRole('button', { name: 'Close' }));
  expect(inspectButton).toHaveFocus();
});
