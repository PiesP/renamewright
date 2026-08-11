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

export interface PlanningClient {
  nativeSelectionAvailable: boolean;
  loadSample(prefix: string): Promise<Plan>;
  selectSources(prefix: string): Promise<Plan | null>;
  previewPrefix(prefix: string): Promise<Plan>;
  watchSourceChanges(onChange: (change: SourceChange) => void): () => void;
}

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

const sampleNames = ['Quarterly review.pdf', 'team-photo 01.jpg', 'project-notes.txt'];
const illegalWindowsCharacters = '<>:"/\\|?*';

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
    generation: 1,
    rows,
    changedCount,
    blockedCount,
    canApply: changedCount > 0 && blockedCount === 0,
  };
}

export function createPlanningClient(): PlanningClient {
  const nativeSelectionAvailable = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  return {
    nativeSelectionAvailable,
    loadSample: async (prefix) => browserPlan(prefix),
    selectSources: async (prefix) => invoke<Plan | null>('select_sources', { prefix }),
    previewPrefix: async (prefix) =>
      nativeSelectionAvailable ? invoke<Plan>('preview_prefix', { prefix }) : browserPlan(prefix),
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
