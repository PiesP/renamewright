import { expect, test } from 'vitest';
import {
  applyBrowserRules,
  compileBrowserRulePipeline,
  PlanningError,
  RULE_PIPELINE_SCHEMA_VERSION,
  type RulePipelineRequest,
} from '../../src/planning/rules';

test('applies enabled text rules in order and records stable rule IDs', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: [
      { kind: 'prefix', ruleId: 7, enabled: true, value: 'draft-' },
      {
        kind: 'literalReplace',
        ruleId: 11,
        enabled: true,
        search: 'draft',
        replacement: 'final',
      },
      { kind: 'suffix', ruleId: 19, enabled: false, value: '.ignored' },
      { kind: 'suffix', ruleId: 23, enabled: true, value: '.bak' },
    ],
  };

  const result = applyBrowserRules('report.txt', request);

  expect(result.proposedName).toBe('final-report.txt.bak');
  expect(result.trace.map(({ ruleIndex, ruleId }) => ({ ruleIndex, ruleId }))).toEqual([
    { ruleIndex: 0, ruleId: 7 },
    { ruleIndex: 1, ruleId: 11 },
    { ruleIndex: 2, ruleId: 23 },
  ]);
});

test('expands numbered and named Rust-style regex captures', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: [
      {
        kind: 'regexReplace',
        ruleId: 31,
        enabled: true,
        pattern: '^(?<stem>.*)\\.([^.]+)$',
        replacement: `\${stem}-copy.$2`,
      },
    ],
  };

  expect(applyBrowserRules('notes.txt', request).proposedName).toBe('notes-copy.txt');
});

test('parses unbraced replacement references with Rust longest-match semantics', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: [
      {
        kind: 'regexReplace',
        ruleId: 37,
        enabled: true,
        pattern: '^(notes)',
        replacement: `$1a-\${1}a`,
      },
    ],
  };

  expect(applyBrowserRules('notes.txt', request).proposedName).toBe('-notesa.txt');
});

test('rejects invalid and Rust-unsupported regex features with a path-free rule ID', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: [
      {
        kind: 'regexReplace',
        ruleId: 41,
        enabled: true,
        pattern: '(?=secret)',
        replacement: '',
      },
    ],
  };

  expect(() => applyBrowserRules('/home/private.txt', request)).toThrowError(PlanningError);
  try {
    applyBrowserRules('/home/private.txt', request);
  } catch (cause) {
    expect(cause).toMatchObject({ code: 'invalidRegex', ruleId: 41 });
    expect((cause as Error).message).not.toContain('/home/private.txt');
  }
});

test('allocates sequence values from the complete source set instead of render order', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: [
      {
        kind: 'sequence',
        ruleId: 53,
        enabled: true,
        scope: 'allSources',
        order: 'sourceOrder',
        start: 5,
        step: 5,
        padding: 3,
        placement: 'prefix',
        separator: '-',
      },
    ],
  };
  const sources = [
    { sourceId: 30, parentId: 1, originalName: 'third.txt' },
    { sourceId: 10, parentId: 1, originalName: 'first.txt' },
    { sourceId: 20, parentId: 1, originalName: 'second.txt' },
  ];
  const apply = compileBrowserRulePipeline(request, sources);

  expect(sources.map((source) => apply(source).proposedName)).toEqual([
    '015-third.txt',
    '005-first.txt',
    '010-second.txt',
  ]);
});

test('sorts sequence names and resets counters for each parent', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: [
      {
        kind: 'sequence',
        ruleId: 59,
        enabled: true,
        scope: 'perParent',
        order: 'nameAscending',
        start: 1,
        step: 1,
        padding: 2,
        placement: 'suffix',
        separator: '_',
      },
    ],
  };
  const sources = [
    { sourceId: 1, parentId: 7, originalName: 'zeta.txt' },
    { sourceId: 2, parentId: 8, originalName: 'beta.txt' },
    { sourceId: 3, parentId: 7, originalName: 'alpha.txt' },
    { sourceId: 4, parentId: 8, originalName: 'alpha.txt' },
  ];
  const apply = compileBrowserRulePipeline(request, sources);

  expect(sources.map((source) => apply(source).proposedName)).toEqual([
    'zeta.txt_02',
    'beta.txt_02',
    'alpha.txt_01',
    'alpha.txt_01',
  ]);
});

test('reports sequence validation and row-local overflow without source data', () => {
  const invalid: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: [
      {
        kind: 'sequence',
        ruleId: 61,
        enabled: false,
        scope: 'allSources',
        order: 'sourceOrder',
        start: 1,
        step: 0,
        padding: 21,
        placement: 'prefix',
        separator: '-',
      },
    ],
  };
  expect(() => applyBrowserRules('/home/private.txt', invalid)).toThrowError(PlanningError);
  try {
    applyBrowserRules('/home/private.txt', invalid);
  } catch (cause) {
    expect(cause).toMatchObject({ code: 'invalidSequenceStep', ruleId: 61 });
    expect((cause as Error).message).not.toContain('/home/private.txt');
  }

  const overflow: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: [
      {
        kind: 'sequence',
        ruleId: 67,
        enabled: true,
        scope: 'allSources',
        order: 'sourceOrder',
        start: Number.MAX_SAFE_INTEGER,
        step: Number.MAX_SAFE_INTEGER,
        padding: 1,
        placement: 'prefix',
        separator: '-',
      },
    ],
  };
  const sources = Array.from({ length: 2_049 }, (_, index) => ({
    sourceId: index + 1,
    parentId: 1,
    originalName: `source-${index + 1}.txt`,
  }));
  const first = sources.at(0);
  const overflowed = sources.at(-1);
  if (!first || !overflowed) {
    throw new Error('Sequence overflow fixtures are incomplete.');
  }
  const apply = compileBrowserRulePipeline(overflow, sources);
  expect(apply(first).diagnostic).toBeUndefined();
  expect(apply(overflowed)).toMatchObject({
    proposedName: 'source-2049.txt',
    diagnostic: 'sequenceOverflow',
  });
});
