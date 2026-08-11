import { cleanup, render, screen } from '@solidjs/testing-library';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import { App } from '../../src/App';
import type { Plan, PlanningClient, SourceChange } from '../../src/planning/client';

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
    watchSourceChanges: () => () => undefined,
  };
}

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
