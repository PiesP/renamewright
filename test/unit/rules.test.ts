import { expect, test } from 'vitest';
import {
  applyBrowserRules,
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
