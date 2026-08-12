import { createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import type { PlanRow, RowStatus } from './client';
import type { SourceOverride } from './rules';

type PlanFilter = 'all' | RowStatus;

interface VirtualPlanTableProps {
  rows: PlanRow[];
  overrides: SourceOverride[];
  onOverride: (sourceId: number, value: string | undefined) => void;
}

const DEFAULT_ROW_HEIGHT = 92;
const DEFAULT_VIEWPORT_HEIGHT = 560;
const OVERSCAN_ROWS = 5;

const diagnosticLabels: Record<string, string> = {
  unchanged: 'No change',
  emptyName: 'Empty name',
  illegalCharacter: 'Illegal Windows character',
  trailingDotOrSpace: 'Trailing dot or space',
  reservedName: 'Reserved Windows name',
  nameTooLong: 'Name exceeds 255 characters',
  duplicateDestination: 'Duplicate destination',
  unsupportedEncoding: 'Unsupported name encoding',
  occupiedDestination: 'Destination already exists',
  staleSource: 'Source changed since admission',
  parentUnavailable: 'Source directory could not be validated',
};

const filterLabels: Record<PlanFilter, string> = {
  all: 'All',
  changed: 'Changed',
  blocked: 'Blocked',
  unchanged: 'Unchanged',
};

const filters: PlanFilter[] = ['all', 'changed', 'blocked', 'unchanged'];

function statusLabel(status: RowStatus) {
  if (status === 'blocked') {
    return 'Blocked';
  }
  if (status === 'changed') {
    return 'Changed';
  }
  return 'Unchanged';
}

function statusMark(status: RowStatus) {
  if (status === 'blocked') {
    return '×';
  }
  if (status === 'changed') {
    return '→';
  }
  return '—';
}

export function VirtualPlanTable(props: VirtualPlanTableProps) {
  const [filter, setFilter] = createSignal<PlanFilter>('all');
  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewportHeight, setViewportHeight] = createSignal(DEFAULT_VIEWPORT_HEIGHT);
  const [rowHeight, setRowHeight] = createSignal(DEFAULT_ROW_HEIGHT);
  const [editingSourceId, setEditingSourceId] = createSignal<number>();
  const [overrideDraft, setOverrideDraft] = createSignal('');
  let viewport: HTMLDivElement | undefined;

  const counts = createMemo(() => ({
    all: props.rows.length,
    changed: props.rows.filter((row) => row.status === 'changed').length,
    blocked: props.rows.filter((row) => row.status === 'blocked').length,
    unchanged: props.rows.filter((row) => row.status === 'unchanged').length,
  }));
  const filteredRows = createMemo(() => {
    const activeFilter = filter();
    return activeFilter === 'all'
      ? props.rows
      : props.rows.filter((row) => row.status === activeFilter);
  });
  const windowStart = createMemo(() =>
    Math.max(Math.floor(scrollTop() / rowHeight()) - OVERSCAN_ROWS, 0)
  );
  const windowEnd = createMemo(() =>
    Math.min(
      Math.ceil((scrollTop() + viewportHeight()) / rowHeight()) + OVERSCAN_ROWS,
      filteredRows().length
    )
  );
  const visibleRows = createMemo(() =>
    filteredRows()
      .slice(windowStart(), windowEnd())
      .map((row, offset) => ({ row, index: windowStart() + offset }))
  );

  const updateMetrics = () => {
    if (!viewport) {
      return;
    }
    setViewportHeight(viewport.clientHeight || DEFAULT_VIEWPORT_HEIGHT);
    const configuredHeight = Number.parseFloat(
      getComputedStyle(viewport).getPropertyValue('--virtual-row-height')
    );
    setRowHeight(configuredHeight || DEFAULT_ROW_HEIGHT);
  };

  onMount(() => {
    updateMetrics();
    if (typeof ResizeObserver === 'undefined' || !viewport) {
      return;
    }
    const observer = new ResizeObserver(updateMetrics);
    observer.observe(viewport);
    onCleanup(() => observer.disconnect());
  });

  const chooseFilter = (nextFilter: PlanFilter) => {
    setFilter(nextFilter);
    setScrollTop(0);
    if (viewport) {
      viewport.scrollTop = 0;
    }
  };

  const existingOverride = (sourceId: number) =>
    props.overrides.find((nameOverride) => nameOverride.sourceId === sourceId)?.value;

  const beginOverride = (row: PlanRow) => {
    setEditingSourceId(row.sourceId);
    setOverrideDraft(existingOverride(row.sourceId) ?? row.proposedName);
  };

  const cancelOverride = () => {
    setEditingSourceId(undefined);
    setOverrideDraft('');
  };

  const saveOverride = (sourceId: number) => {
    props.onOverride(sourceId, overrideDraft());
    cancelOverride();
  };

  return (
    <section class="plan-results" aria-label="Rename plan rows">
      <div class="preview-toolbar">
        <fieldset class="filter-group">
          <legend class="visually-hidden">Filter plan rows</legend>
          <For each={filters}>
            {(candidate) => (
              <button
                class="filter-button"
                type="button"
                aria-pressed={filter() === candidate}
                onClick={() => chooseFilter(candidate)}
              >
                {filterLabels[candidate]} <span>{counts()[candidate]}</span>
              </button>
            )}
          </For>
        </fieldset>
        <span class="visible-count" aria-live="polite">
          Showing {filteredRows().length} of {props.rows.length}
        </span>
      </div>

      <div class="virtual-table">
        <section
          class="virtual-viewport"
          aria-label="Scrollable rename plan"
          ref={viewport}
          onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        >
          <table aria-rowcount={filteredRows().length + 1}>
            <thead class="virtual-header">
              <tr>
                <th scope="col">Source</th>
                <th scope="col">Proposed</th>
                <th scope="col">Status</th>
              </tr>
            </thead>
            <tbody
              class="virtual-canvas"
              style={{ height: `${filteredRows().length * rowHeight()}px` }}
            >
              <For each={visibleRows()}>
                {({ row, index }) => (
                  <tr
                    class="virtual-row"
                    aria-rowindex={index + 2}
                    data-status={row.status}
                    style={{
                      height: `${rowHeight()}px`,
                      transform: `translateY(${index * rowHeight()}px)`,
                    }}
                  >
                    <td class="virtual-cell" data-label="Source">
                      <span class="file-name">{row.originalName}</span>
                    </td>
                    <td class="virtual-cell" data-label="Proposed">
                      <Show
                        when={editingSourceId() === row.sourceId}
                        fallback={
                          <div class="proposed-name-content">
                            <span class="file-name proposed-name">{row.proposedName}</span>
                            <div class="override-actions">
                              <Show when={row.overrideApplied}>
                                <span class="override-badge">Override</span>
                              </Show>
                              <button
                                class="table-action"
                                type="button"
                                aria-label={`Edit override for ${row.originalName}`}
                                onClick={() => beginOverride(row)}
                              >
                                Edit
                              </button>
                              <Show when={row.overrideApplied}>
                                <button
                                  class="table-action table-action-reset"
                                  type="button"
                                  aria-label={`Reset override for ${row.originalName}`}
                                  onClick={() => props.onOverride(row.sourceId, undefined)}
                                >
                                  Reset
                                </button>
                              </Show>
                            </div>
                          </div>
                        }
                      >
                        <form
                          class="override-editor"
                          onSubmit={(event) => {
                            event.preventDefault();
                            saveOverride(row.sourceId);
                          }}
                        >
                          <label class="visually-hidden" for={`override-${row.sourceId}`}>
                            Override name for {row.originalName}
                          </label>
                          <input
                            id={`override-${row.sourceId}`}
                            type="text"
                            autofocus
                            value={overrideDraft()}
                            onInput={(event) => setOverrideDraft(event.currentTarget.value)}
                          />
                          <div class="override-actions">
                            <button class="table-action" type="submit">
                              Save
                            </button>
                            <button class="table-action" type="button" onClick={cancelOverride}>
                              Cancel
                            </button>
                          </div>
                        </form>
                      </Show>
                    </td>
                    <td class="virtual-cell virtual-status-cell" data-label="Status">
                      <span class={`status status-${row.status}`}>
                        <span aria-hidden="true">{statusMark(row.status)}</span>
                        {statusLabel(row.status)}
                      </span>
                      <Show when={row.diagnostics.length > 0}>
                        <span class="diagnostic">
                          {row.diagnostics.map((code) => diagnosticLabels[code] ?? code).join(', ')}
                        </span>
                      </Show>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </section>
      </div>
    </section>
  );
}
