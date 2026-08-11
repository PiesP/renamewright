import { invoke } from '@tauri-apps/api/core';

export type RowStatus = 'changed' | 'unchanged' | 'blocked';

export interface PlanRow {
  sourceId: number;
  originalName: string;
  proposedName: string;
  status: RowStatus;
  diagnostics: string[];
}

export interface Plan {
  planId: number;
  generation: number;
  rows: PlanRow[];
  changedCount: number;
  blockedCount: number;
  canApply: boolean;
}

export interface SourceChange {
  revision: number;
  error: string | null;
}

export type LedgerStatus =
  | 'completed'
  | 'rolledBack'
  | 'forwardPending'
  | 'completionPending'
  | 'rollbackPending'
  | 'rollbackCompletionPending'
  | 'reconciliationRequired'
  | 'recoveryRequired'
  | 'legacyInspectionRequired'
  | 'torn'
  | 'damaged'
  | 'unsupportedVersion'
  | 'tooLarge'
  | 'unreadable';

export interface LedgerEntry {
  ledgerId: number;
  planId: number | null;
  sourceGeneration: number | null;
  schemaVersion: number | null;
  sourceCount: number;
  status: LedgerStatus;
  attentionStep: number | null;
  recoveryAvailable: boolean;
}

export interface PlanningClient {
  nativeSelectionAvailable: boolean;
  loadSample(prefix: string): Promise<Plan>;
  selectSources(prefix: string): Promise<Plan | null>;
  previewPrefix(prefix: string): Promise<Plan>;
  inspectPlan(planId: number): Promise<string>;
  exportPlan(planId: number): Promise<boolean>;
  listLedger(): Promise<LedgerEntry[]>;
  watchSourceChanges(onChange: (change: SourceChange) => void): () => void;
}

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

const sampleNames = ['Quarterly review.pdf', 'team-photo 01.jpg', 'project-notes.txt'];
const illegalWindowsCharacters = '<>:"/\\|?*';

let nextBrowserPlanId = 1;

function browserPlan(prefix: string): Plan {
  const rows = sampleNames.map((originalName, index): PlanRow => {
    const proposedName = `${prefix}${originalName}`;
    const illegal = [...proposedName].some((character) => {
      const codePoint = character.codePointAt(0) ?? 0;
      return codePoint < 32 || illegalWindowsCharacters.includes(character);
    });
    const trailing = /[. ]$/u.test(proposedName);
    const reserved = /^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?$/iu.test(proposedName);
    const blocked = illegal || trailing || reserved || proposedName.length > 255;
    const diagnostics = [
      ...(illegal ? ['illegalCharacter'] : []),
      ...(trailing ? ['trailingDotOrSpace'] : []),
      ...(reserved ? ['reservedName'] : []),
      ...(proposedName.length > 255 ? ['nameTooLong'] : []),
    ];

    return {
      sourceId: index + 1,
      originalName,
      proposedName,
      status: blocked ? 'blocked' : proposedName === originalName ? 'unchanged' : 'changed',
      diagnostics,
    };
  });
  const changedCount = rows.filter((row) => row.status === 'changed').length;
  const blockedCount = rows.filter((row) => row.status === 'blocked').length;

  return {
    planId: nextBrowserPlanId++,
    generation: 1,
    rows,
    changedCount,
    blockedCount,
    canApply: changedCount > 0 && blockedCount === 0,
  };
}

export function createPlanningClient(): PlanningClient {
  const nativeSelectionAvailable = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
  let latestBrowserPlan: { prefix: string; plan: Plan } | undefined;
  const createBrowserPlan = (prefix: string) => {
    const plan = browserPlan(prefix);
    latestBrowserPlan = { prefix, plan };
    return plan;
  };

  const inspectBrowserPlan = (planId: number) => {
    if (!latestBrowserPlan || latestBrowserPlan.plan.planId !== planId) {
      throw new Error('The requested plan is no longer current.');
    }
    const { plan, prefix } = latestBrowserPlan;
    return JSON.stringify(
      {
        schemaVersion: 1,
        protocolVersion: 1,
        product: 'Renamewright',
        planId: plan.planId,
        sourceGeneration: plan.generation,
        rules: [{ kind: 'prefix', value: prefix }],
        summary: {
          sourceCount: plan.rows.length,
          changedCount: plan.changedCount,
          blockedCount: plan.blockedCount,
          canApply: plan.canApply,
        },
        rows: plan.rows.map((row) => ({
          sourceId: row.sourceId,
          originalDisplay: row.originalName,
          proposedDisplay: row.proposedName,
          status: row.status,
          diagnostics: row.diagnostics,
          trace: [{ ruleIndex: 0, before: row.originalName, after: row.proposedName }],
        })),
      },
      null,
      2
    );
  };

  return {
    nativeSelectionAvailable,
    loadSample: async (prefix) => createBrowserPlan(prefix),
    selectSources: async (prefix) => invoke<Plan | null>('select_sources', { prefix }),
    previewPrefix: async (prefix) =>
      nativeSelectionAvailable
        ? invoke<Plan>('preview_prefix', { prefix })
        : createBrowserPlan(prefix),
    inspectPlan: async (planId) =>
      nativeSelectionAvailable
        ? invoke<string>('inspect_plan', { planId })
        : inspectBrowserPlan(planId),
    exportPlan: async (planId) =>
      nativeSelectionAvailable ? invoke<boolean>('export_plan', { planId }) : false,
    listLedger: async () => (nativeSelectionAvailable ? invoke<LedgerEntry[]>('list_ledger') : []),
    watchSourceChanges: (onChange) => {
      if (!nativeSelectionAvailable) {
        return () => undefined;
      }

      let revision = 0;
      let inFlight = false;
      const poll = async () => {
        if (inFlight) {
          return;
        }
        inFlight = true;
        try {
          const change = await invoke<SourceChange | null>('poll_source_changes', {
            since: revision,
          });
          if (change) {
            revision = change.revision;
            onChange(change);
          }
        } catch (cause) {
          onChange({
            revision,
            error: cause instanceof Error ? cause.message : 'Dropped sources could not be checked.',
          });
        } finally {
          inFlight = false;
        }
      };
      const interval = window.setInterval(() => void poll(), 400);
      return () => window.clearInterval(interval);
    },
  };
}
