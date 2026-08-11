import { createSignal, For, Show } from 'solid-js';
import { APP_NAME } from './app-meta';
import { createPlanningClient, type Plan, type PlanningClient } from './planning/client';

interface AppProps {
  client?: PlanningClient;
}

const diagnosticLabels: Record<string, string> = {
  unchanged: 'No change',
  emptyName: 'Empty name',
  illegalCharacter: 'Illegal Windows character',
  trailingDotOrSpace: 'Trailing dot or space',
  reservedName: 'Reserved Windows name',
  nameTooLong: 'Name exceeds 255 characters',
  duplicateDestination: 'Duplicate destination',
  unsupportedEncoding: 'Unsupported name encoding',
};

export function App(props: AppProps) {
  const planningClient = props.client ?? createPlanningClient();
  const [prefix, setPrefix] = createSignal('');
  const [plan, setPlan] = createSignal<Plan>();
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  let requestSequence = 0;

  const setResult = (result: Plan | null) => {
    if (result) {
      setPlan(result);
    }
  };

  const run = async (operation: () => Promise<Plan | null>) => {
    const sequence = ++requestSequence;
    setBusy(true);
    setError('');
    try {
      const result = await operation();
      if (sequence === requestSequence) {
        setResult(result);
      }
    } catch (cause) {
      if (sequence === requestSequence) {
        setError(cause instanceof Error ? cause.message : 'The rename plan could not be updated.');
      }
    } finally {
      if (sequence === requestSequence) {
        setBusy(false);
      }
    }
  };

  const updatePrefix = (value: string) => {
    setPrefix(value);
    if (plan()) {
      void run(() => planningClient.previewPrefix(value));
    }
  };

  const statusMessage = () => {
    const current = plan();
    if (error()) {
      return error();
    }
    if (busy()) {
      return 'Updating the rename plan…';
    }
    if (!current) {
      return 'No sources are loaded.';
    }
    if (current.blockedCount > 0) {
      return `${current.blockedCount} names are blocked. Edit the prefix before continuing.`;
    }
    return `${current.changedCount} names are ready for review.`;
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
          <span class="read-only-badge">Read-only milestone</span>
          <button
            class="button button-secondary"
            type="button"
            disabled={!planningClient.nativeSelectionAvailable || busy()}
            aria-describedby="native-selection-note"
            onClick={() => void run(() => planningClient.selectSources(prefix()))}
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
          Native paths stay inside the Rust process.
        </Show>
      </p>

      <div class="workbench-grid">
        <aside class="rule-rail" aria-labelledby="rule-heading">
          <div class="rail-heading">
            <h1 id="rule-heading">Rename rules</h1>
            <span>1 active</span>
          </div>
          <section class="rule-editor" aria-labelledby="prefix-heading">
            <div class="rule-title">
              <span class="rule-order">01</span>
              <h2 id="prefix-heading">Add prefix</h2>
            </div>
            <label for="prefix">Prefix</label>
            <div class="input-shell" data-invalid={plan()?.blockedCount ? 'true' : 'false'}>
              <input
                id="prefix"
                type="text"
                value={prefix()}
                placeholder="2026-"
                aria-invalid={plan()?.blockedCount ? 'true' : 'false'}
                aria-describedby="prefix-help"
                onInput={(event) => updatePrefix(event.currentTarget.value)}
              />
              <span aria-hidden="true">{plan()?.blockedCount ? '!' : ''}</span>
            </div>
            <p
              id="prefix-help"
              class={plan()?.blockedCount ? 'field-help field-help-error' : 'field-help'}
            >
              <Show
                when={plan()?.blockedCount}
                fallback="Added before every source name. The preview updates immediately."
              >
                Some destinations are invalid on Windows. Remove reserved characters or names.
              </Show>
            </p>
          </section>
          <div class="scope-note">
            <strong>Current scope</strong>
            <p>Prefix rules and deterministic Windows name checks. No file operation can run.</p>
          </div>
        </aside>

        <section class="preview-pane" aria-labelledby="preview-heading">
          <div class="preview-heading">
            <div>
              <h2 id="preview-heading">Proposed names</h2>
              <p>Original names remain untouched while you inspect this plan.</p>
            </div>
            <span class="generation">Generation {plan()?.generation ?? 0}</span>
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
                  Load the local sample to test a prefix, or open the desktop app to select files.
                </p>
                <button
                  class="button button-primary"
                  type="button"
                  disabled={busy()}
                  onClick={() => void run(() => planningClient.loadSample(prefix()))}
                >
                  Load sample
                </button>
              </div>
            }
          >
            <div class="table-wrap">
              <table>
                <thead>
                  <tr>
                    <th scope="col">Source</th>
                    <th scope="col">Proposed</th>
                    <th scope="col">Status</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={plan()?.rows}>
                    {(row) => (
                      <tr data-status={row.status}>
                        <td data-label="Source">
                          <span class="file-name">{row.originalName}</span>
                        </td>
                        <td data-label="Proposed">
                          <span class="file-name proposed-name">{row.proposedName}</span>
                        </td>
                        <td data-label="Status">
                          <span class={`status status-${row.status}`}>
                            <span aria-hidden="true">
                              {row.status === 'blocked'
                                ? '×'
                                : row.status === 'changed'
                                  ? '→'
                                  : '—'}
                            </span>
                            {row.status === 'blocked'
                              ? 'Blocked'
                              : row.status === 'changed'
                                ? 'Changed'
                                : 'Unchanged'}
                          </span>
                          <Show when={row.diagnostics.length > 0}>
                            <span class="diagnostic">
                              {row.diagnostics
                                .map((code) => diagnosticLabels[code] ?? code)
                                .join(', ')}
                            </span>
                          </Show>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
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
          <span>File execution arrives after recovery design and Windows testing.</span>
          <button class="button button-locked" type="button" disabled>
            Execution unavailable
          </button>
        </div>
      </footer>

      <p
        class={error() ? 'live-status live-status-error' : 'live-status'}
        role="status"
        aria-live="polite"
      >
        {statusMessage()}
      </p>
    </main>
  );
}
