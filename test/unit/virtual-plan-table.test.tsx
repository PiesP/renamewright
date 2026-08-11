import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, test } from 'vitest';
import type { PlanRow } from '../../src/planning/client';
import { VirtualPlanTable } from '../../src/planning/VirtualPlanTable';

afterEach(cleanup);

function makeRows(count: number): PlanRow[] {
  return Array.from({ length: count }, (_, index) => ({
    sourceId: index + 1,
    originalName: `source-${index}.txt`,
    proposedName: `renamed-source-${index}.txt`,
    status: index === count - 1 ? 'blocked' : 'changed',
    diagnostics: index === count - 1 ? ['duplicateDestination'] : [],
  }));
}

test('windows a large plan and filters without rendering every row', async () => {
  const user = userEvent.setup();
  render(() => <VirtualPlanTable rows={makeRows(10_000)} />);

  expect(screen.getByText('Showing 10000 of 10000')).toBeInTheDocument();
  expect(screen.getAllByRole('row').length).toBeLessThan(30);
  expect(screen.queryByText('source-9999.txt')).not.toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: 'Blocked 1' }));

  expect(screen.getByText('Showing 1 of 10000')).toBeInTheDocument();
  expect(screen.getByText('source-9999.txt')).toBeInTheDocument();
  expect(screen.getByText('Duplicate destination')).toBeInTheDocument();
});

test('updates the rendered window while scrolling', () => {
  render(() => <VirtualPlanTable rows={makeRows(100)} />);
  const viewport = screen.getByRole('region', { name: 'Scrollable rename plan' });

  Object.defineProperty(viewport, 'scrollTop', { configurable: true, value: 8_000 });
  fireEvent.scroll(viewport);

  expect(screen.getByText('source-90.txt')).toBeInTheDocument();
  expect(screen.queryByText('source-0.txt')).not.toBeInTheDocument();
});
