import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test, vi } from 'vitest';
import type { PlanRow } from '../../src/planning/client';
import { VirtualPlanTable } from '../../src/planning/VirtualPlanTable';

afterEach(cleanup);

function makeRows(count: number): PlanRow[] {
  return Array.from({ length: count }, (_, index) => ({
    sourceId: index + 1,
    originalName: `source-${index}.txt`,
    proposedName: `renamed-source-${index}.txt`,
    overrideApplied: false,
    status: index === count - 1 ? 'blocked' : 'changed',
    diagnostics:
      index === count - 1
        ? ['duplicateDestination', 'occupiedDestination', 'staleSource', 'parentUnavailable']
        : [],
  }));
}

test('windows a large plan and filters without rendering every row', async () => {
  const user = userEvent.setup();
  render(() => <VirtualPlanTable rows={makeRows(10_000)} overrides={[]} onOverride={() => {}} />);

  expect(screen.getByText('Showing 10000 of 10000')).toBeInTheDocument();
  expect(screen.getAllByRole('row').length).toBeLessThan(30);
  expect(screen.queryByText('source-9999.txt')).not.toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: 'Blocked 1' }));

  expect(screen.getByText('Showing 1 of 10000')).toBeInTheDocument();
  expect(screen.getByText('source-9999.txt')).toBeInTheDocument();
  expect(screen.getByText(/Duplicate destination/u)).toBeInTheDocument();
  expect(screen.getByText(/Destination already exists/u)).toBeInTheDocument();
  expect(screen.getByText(/Source changed since admission/u)).toBeInTheDocument();
  expect(screen.getByText(/Source directory could not be validated/u)).toBeInTheDocument();
});

test('updates the rendered window while scrolling', () => {
  render(() => <VirtualPlanTable rows={makeRows(100)} overrides={[]} onOverride={() => {}} />);
  const viewport = screen.getByRole('region', { name: 'Scrollable rename plan' });

  Object.defineProperty(viewport, 'scrollTop', { configurable: true, value: 8_000 });
  fireEvent.scroll(viewport);

  expect(screen.getByText('source-90.txt')).toBeInTheDocument();
  expect(screen.queryByText('source-0.txt')).not.toBeInTheDocument();
});

test('edits, cancels, saves, and resets a source override', async () => {
  const user = userEvent.setup();
  const onOverride = vi.fn();
  const rows = makeRows(1);
  const row = rows[0];
  if (!row) {
    throw new Error('Override fixture is incomplete.');
  }
  rows[0] = { ...row, proposedName: 'manual.md', overrideApplied: true };
  render(() => (
    <VirtualPlanTable
      rows={rows}
      overrides={[{ sourceId: 1, value: 'manual.md' }]}
      onOverride={onOverride}
    />
  ));

  expect(screen.getByText('Override')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Edit override for source-0.txt' }));
  const input = screen.getByRole('textbox', { name: 'Override name for source-0.txt' });
  expect(input).toHaveValue('manual.md');
  await user.clear(input);
  await user.type(input, 'reviewed.txt');
  await user.click(screen.getByRole('button', { name: 'Cancel' }));
  expect(onOverride).not.toHaveBeenCalled();

  await user.click(screen.getByRole('button', { name: 'Edit override for source-0.txt' }));
  const reopened = screen.getByRole('textbox', { name: 'Override name for source-0.txt' });
  await user.clear(reopened);
  await user.type(reopened, 'reviewed.txt');
  await user.click(screen.getByRole('button', { name: 'Save' }));
  expect(onOverride).toHaveBeenCalledWith(1, 'reviewed.txt');

  await user.click(screen.getByRole('button', { name: 'Reset override for source-0.txt' }));
  expect(onOverride).toHaveBeenCalledWith(1, undefined);
});
