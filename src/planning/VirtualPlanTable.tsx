import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { type Locale, type MessageKey, message } from '../i18n/catalog';
import type { PlanRow, RowStatus } from './client';
import type { SourceOverride } from './rules';

type PlanFilter = 'all' | RowStatus;
type DiagnosticFilter = 'all' | string;

interface VirtualPlanTableProps {
  rows: PlanRow[];
  overrides: SourceOverride[];
  locale?: Locale;
  onOverride: (sourceId: number, value: string | undefined) => void;
}

const DEFAULT_ROW_HEIGHT = 92;
const DEFAULT_VIEWPORT_HEIGHT = 560;
const OVERSCAN_ROWS = 5;

const diagnosticLabelKeys: Record<string, MessageKey> = {
  unchanged: 'diagnosticUnchanged',
  emptyName: 'diagnosticEmptyName',
  illegalCharacter: 'diagnosticIllegalCharacter',
  trailingDotOrSpace: 'diagnosticTrailingDotOrSpace',
  reservedName: 'diagnosticReservedName',
  nameTooLong: 'diagnosticNameTooLong',
  duplicateDestination: 'diagnosticDuplicateDestination',
  unsupportedEncoding: 'diagnosticUnsupportedEncoding',
  occupiedDestination: 'diagnosticOccupiedDestination',
  staleSource: 'diagnosticStaleSource',
  parentUnavailable: 'diagnosticParentUnavailable',
  sequenceOverflow: 'diagnosticSequenceOverflow',
};

const filterLabelKeys: Record<PlanFilter, MessageKey> = {
  all: 'filterAll',
  changed: 'filterChanged',
  blocked: 'filterBlocked',
  unchanged: 'filterUnchanged',
};

const filters: PlanFilter[] = ['all', 'changed', 'blocked', 'unchanged'];

function statusLabelKey(status: RowStatus): MessageKey {
  if (status === 'blocked') {
    return 'statusBlocked';
  }
  if (status === 'changed') {
    return 'statusChanged';
  }
  return 'statusUnchanged';
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
  const text = (key: MessageKey, values?: Readonly<Record<string, string | number>>) =>
    message(props.locale ?? 'en', key, values);
  const [filter, setFilter] = createSignal<PlanFilter>('all');
  const [diagnosticFilter, setDiagnosticFilter] = createSignal<DiagnosticFilter>('all');
  const [sourceQuery, setSourceQuery] = createSignal('');
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
  const diagnosticOptions = createMemo(() => {
    const countsByCode = new Map<string, number>();
    for (const row of props.rows) {
      for (const code of new Set(row.diagnostics)) {
        countsByCode.set(code, (countsByCode.get(code) ?? 0) + 1);
      }
    }
    const knownOrder = new Map(
      Object.keys(diagnosticLabelKeys).map((code, index) => [code, index])
    );
    return [...countsByCode]
      .map(([code, count]) => ({ code, count }))
      .sort((left, right) => {
        const leftOrder = knownOrder.get(left.code) ?? Number.MAX_SAFE_INTEGER;
        const rightOrder = knownOrder.get(right.code) ?? Number.MAX_SAFE_INTEGER;
        return leftOrder - rightOrder || left.code.localeCompare(right.code);
      });
  });
  const filteredRows = createMemo(() => {
    const activeFilter = filter();
    const activeDiagnostic = diagnosticFilter();
    const query = sourceQuery()
      .trim()
      .toLocaleLowerCase(props.locale ?? 'en');
    return props.rows.filter(
      (row) =>
        (activeFilter === 'all' || row.status === activeFilter) &&
        (activeDiagnostic === 'all' || row.diagnostics.includes(activeDiagnostic)) &&
        (query.length === 0 ||
          row.originalName.toLocaleLowerCase(props.locale ?? 'en').includes(query) ||
          row.proposedName.toLocaleLowerCase(props.locale ?? 'en').includes(query))
    );
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

  createEffect(() => {
    const activeDiagnostic = diagnosticFilter();
    if (
      activeDiagnostic !== 'all' &&
      !diagnosticOptions().some((option) => option.code === activeDiagnostic)
    ) {
      setDiagnosticFilter('all');
    }
  });

  const resetViewport = () => {
    setScrollTop(0);
    if (viewport) {
      viewport.scrollTop = 0;
    }
  };

  const chooseFilter = (nextFilter: PlanFilter) => {
    setFilter(nextFilter);
    resetViewport();
  };

  const chooseDiagnostic = (nextFilter: DiagnosticFilter) => {
    setDiagnosticFilter(nextFilter);
    resetViewport();
  };

  const updateSourceQuery = (value: string) => {
    setSourceQuery(value);
    resetViewport();
  };

  const clearFilters = () => {
    setFilter('all');
    setDiagnosticFilter('all');
    setSourceQuery('');
    resetViewport();
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
    <section class="plan-results" aria-label={text('planRows')}>
      <div class="preview-toolbar">
        <fieldset class="filter-group">
          <legend class="visually-hidden">{text('filterPlanRows')}</legend>
          <For each={filters}>
            {(candidate) => (
              <button
                class="filter-button"
                type="button"
                aria-pressed={filter() === candidate}
                onClick={() => chooseFilter(candidate)}
              >
                {text(filterLabelKeys[candidate])} <span>{counts()[candidate]}</span>
              </button>
            )}
          </For>
        </fieldset>
        <div class="diagnostic-filter-controls">
          <label>
            <span>{text('filterDiagnostic')}</span>
            <select
              value={diagnosticFilter()}
              onChange={(event) => chooseDiagnostic(event.currentTarget.value)}
            >
              <option value="all">{text('filterAllDiagnostics')}</option>
              <For each={diagnosticOptions()}>
                {(option) => {
                  const key = diagnosticLabelKeys[option.code];
                  const label = key ? text(key) : option.code;
                  return <option value={option.code}>{`${label} (${option.count})`}</option>;
                }}
              </For>
            </select>
          </label>
          <label>
            <span>{text('filterAffectedSource')}</span>
            <input
              type="search"
              value={sourceQuery()}
              placeholder={text('filterAffectedSourcePlaceholder')}
              onInput={(event) => updateSourceQuery(event.currentTarget.value)}
            />
          </label>
          <button
            class="button button-secondary button-compact"
            type="button"
            disabled={
              filter() === 'all' && diagnosticFilter() === 'all' && sourceQuery().length === 0
            }
            onClick={clearFilters}
          >
            {text('clearPlanFilters')}
          </button>
        </div>
        <span class="visible-count" aria-live="polite">
          {text('showingRows', {
            visible: filteredRows().length,
            total: props.rows.length,
          })}
        </span>
      </div>

      <div class="virtual-table">
        <Show
          when={filteredRows().length > 0}
          fallback={
            <div class="filter-empty-state">
              <strong>{text('noFilteredRows')}</strong>
              <p>{text('noFilteredRowsDescription')}</p>
              <button
                class="button button-secondary button-compact"
                type="button"
                onClick={clearFilters}
              >
                {text('clearPlanFilters')}
              </button>
            </div>
          }
        >
          <section
            class="virtual-viewport"
            aria-label={text('scrollablePlan')}
            ref={viewport}
            onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
          >
            <table aria-rowcount={filteredRows().length + 1}>
              <thead class="virtual-header">
                <tr>
                  <th scope="col">{text('columnSource')}</th>
                  <th scope="col">{text('columnProposed')}</th>
                  <th scope="col">{text('columnStatus')}</th>
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
                      <td class="virtual-cell" data-label={text('columnSource')}>
                        <span class="file-name">{row.originalName}</span>
                      </td>
                      <td class="virtual-cell" data-label={text('columnProposed')}>
                        <Show
                          when={editingSourceId() === row.sourceId}
                          fallback={
                            <div class="proposed-name-content">
                              <span class="file-name proposed-name">{row.proposedName}</span>
                              <div class="override-actions">
                                <Show when={row.overrideApplied}>
                                  <span class="override-badge">{text('overrideBadge')}</span>
                                </Show>
                                <button
                                  class="table-action"
                                  type="button"
                                  aria-label={text('editOverride', { name: row.originalName })}
                                  onClick={() => beginOverride(row)}
                                >
                                  {text('edit')}
                                </button>
                                <Show when={row.overrideApplied}>
                                  <button
                                    class="table-action table-action-reset"
                                    type="button"
                                    aria-label={text('resetOverride', { name: row.originalName })}
                                    onClick={() => props.onOverride(row.sourceId, undefined)}
                                  >
                                    {text('reset')}
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
                              {text('overrideName', { name: row.originalName })}
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
                                {text('save')}
                              </button>
                              <button class="table-action" type="button" onClick={cancelOverride}>
                                {text('cancel')}
                              </button>
                            </div>
                          </form>
                        </Show>
                      </td>
                      <td
                        class="virtual-cell virtual-status-cell"
                        data-label={text('columnStatus')}
                      >
                        <span class={`status status-${row.status}`}>
                          <span aria-hidden="true">{statusMark(row.status)}</span>
                          {text(statusLabelKey(row.status))}
                        </span>
                        <Show when={row.diagnostics.length > 0}>
                          <span class="diagnostic">
                            {row.diagnostics
                              .map((code) => {
                                const key = diagnosticLabelKeys[code];
                                return key ? text(key) : code;
                              })
                              .join(', ')}
                          </span>
                        </Show>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </section>
        </Show>
      </div>
    </section>
  );
}
