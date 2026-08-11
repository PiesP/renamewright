import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { APP_NAME } from './app-meta';
import { createPlanningClient, type Plan, type PlanningClient } from './planning/client';
import { VirtualPlanTable } from './planning/VirtualPlanTable';

interface AppProps {
  client?: PlanningClient;
}

export function App(props: AppProps) {
  const planningClient = props.client ?? createPlanningClient();
  const [prefix, setPrefix] = createSignal('');
  const [plan, setPlan] = createSignal<Plan>();
  const [planDocument, setPlanDocument] = createSignal<string>();
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [notice, setNotice] = createSignal('');
  let requestSequence = 0;
  let planInspector: HTMLDialogElement | undefined;
  let planInspectorOpener: HTMLButtonElement | undefined;
  let previewTimer: number | undefined;

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
    setNotice('');
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
      if (previewTimer !== undefined) {
        window.clearTimeout(previewTimer);
      }
      previewTimer = window.setTimeout(() => {
        previewTimer = undefined;
        void run(() => planningClient.previewPrefix(value));
      }, 120);
    }
  };

  const loadInitialSources = () =>
    planningClient.nativeSelectionAvailable
      ? planningClient.selectSources(prefix())
      : planningClient.loadSample(prefix());

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

  onMount(() => {
    const stopWatching = planningClient.watchSourceChanges((change) => {
      if (change.error) {
        setError(change.error);
        return;
      }
      void run(() => planningClient.previewPrefix(prefix()));
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
          <span class="drop-hint">
            {planningClient.nativeSelectionAvailable ? 'Drop files anywhere' : 'Desktop drop ready'}
          </span>
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
          Native picker and drop paths stay inside the Rust process.
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
                fallback="Added before every source name. The preview updates as you type."
              >
                One or more destinations are blocked. Review the row diagnostics before continuing.
              </Show>
            </p>
          </section>
          <div class="scope-note">
            <strong>Current scope</strong>
            <p>
              Prefix rules with deterministic name, occupancy, and stale-source checks. No file
              operation can run.
            </p>
          </div>
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
                  Load the local sample to test a prefix, or select and drop files in the desktop
                  app.
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
          <span>File execution arrives after recovery design and Windows testing.</span>
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
