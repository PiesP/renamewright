import { expect, test } from 'vitest';
import {
  applyBrowserRules,
  compileBrowserRulePipeline,
  MAX_RULE_OUTPUT_BYTES,
  PlanningError,
  RULE_PIPELINE_SCHEMA_VERSION,
  type RulePipelineRequest,
} from '../../src/planning/rules';

test('applies enabled text rules in order and records stable rule IDs', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
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
    overrides: [],
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
    overrides: [],
    rules: [
      {
        kind: 'regexReplace',
        ruleId: 37,
        enabled: true,
        pattern: '^(notes)',
        replacement: `$1a-\${1}a-$$-$missing-$-\${}`,
      },
    ],
  };

  expect(applyBrowserRules('notes.txt', request).proposedName).toBe('-notesa-$--$-.txt');
});

test('stops expanding replacements before an oversized name enters the browser trace', () => {
  for (const rule of [
    {
      kind: 'regexReplace' as const,
      ruleId: 39,
      enabled: true,
      pattern: '',
      replacement: 'x'.repeat(MAX_RULE_OUTPUT_BYTES),
    },
    {
      kind: 'literalReplace' as const,
      ruleId: 40,
      enabled: true,
      search: 'a',
      replacement: 'x'.repeat(MAX_RULE_OUTPUT_BYTES / 4),
    },
  ]) {
    const request: RulePipelineRequest = {
      schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
      overrides: [],
      rules: [rule],
    };

    expect(applyBrowserRules('aaaaaaaa', request)).toEqual({
      proposedName: 'aaaaaaaa',
      trace: [],
      overrideApplied: false,
      diagnostic: 'nameTooLong',
    });
  }

  const chained: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules: [39, 40].map((ruleId) => ({
      kind: 'regexReplace' as const,
      ruleId,
      enabled: true,
      pattern: '',
      replacement: 'x'.repeat(MAX_RULE_OUTPUT_BYTES / 4),
    })),
  };
  const result = applyBrowserRules('a', chained);
  expect(new TextEncoder().encode(result.proposedName)).toHaveLength(MAX_RULE_OUTPUT_BYTES / 2 + 1);
  expect(result.trace).toHaveLength(1);
  expect(result.diagnostic).toBe('nameTooLong');
});

test('rejects invalid and Rust-unsupported regex features with a path-free rule ID', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
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
    overrides: [],
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
    overrides: [],
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
    overrides: [],
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
    overrides: [],
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

test('recomputes the extension boundary through ordered structure rules', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules: [
      { kind: 'extension', ruleId: 71, enabled: true, operation: 'remove', value: 'txt' },
      {
        kind: 'case',
        ruleId: 73,
        enabled: true,
        target: 'extension',
        mode: 'uppercase',
      },
      {
        kind: 'extension',
        ruleId: 79,
        enabled: true,
        operation: 'replace',
        value: 'backup.zip',
      },
    ],
  };

  const result = applyBrowserRules('archive.tar.gz', request);

  expect(result.proposedName).toBe('archive.backup.zip');
  expect(result.trace.map(({ before, after }) => ({ before, after }))).toEqual([
    { before: 'archive.tar.gz', after: 'archive.tar' },
    { before: 'archive.tar', after: 'archive.TAR' },
    { before: 'archive.TAR', after: 'archive.backup.zip' },
  ]);

  const pathLikeUnits: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules: [
      { kind: 'prefix', ruleId: 80, enabled: true, value: 'invalid/' },
      {
        kind: 'extension',
        ruleId: 81,
        enabled: true,
        operation: 'replace',
        value: 'md',
      },
    ],
  };
  expect(applyBrowserRules('report.txt', pathLikeUnits).proposedName).toBe('invalid/report.md');
});

test('handles hidden files, trailing dots, case, and Unicode whitespace by selected part', () => {
  const hidden: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules: [
      {
        kind: 'extension',
        ruleId: 83,
        enabled: true,
        operation: 'replace',
        value: 'txt',
      },
    ],
  };
  expect(applyBrowserRules('.env', hidden).proposedName).toBe('.env.txt');
  expect(applyBrowserRules('report.', hidden).proposedName).toBe('report.txt');

  const cleanup: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules: [
      {
        kind: 'whitespaceCleanup',
        ruleId: 89,
        enabled: true,
        target: 'stem',
        replacement: '-',
      },
      {
        kind: 'case',
        ruleId: 97,
        enabled: true,
        target: 'extension',
        mode: 'lowercase',
      },
    ],
  };
  expect(applyBrowserRules('\u2003초안\t 보고서 \n.TXT', cleanup).proposedName).toBe(
    '초안-보고서.txt'
  );
});

test('normalizes Unicode only when an explicit normalization rule is enabled', () => {
  const decomposed = 're\u0301sume\u0301.txt';
  const disabled: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules: [
      {
        kind: 'unicodeNormalization',
        ruleId: 101,
        enabled: false,
        target: 'stem',
        form: 'nfc',
      },
    ],
  };
  expect(applyBrowserRules(decomposed, disabled).proposedName).toBe(decomposed);

  const forms = [
    { form: 'nfc' as const, input: decomposed, output: 'résumé.txt' },
    { form: 'nfd' as const, input: 'é.txt', output: 'e\u0301.txt' },
    { form: 'nfkc' as const, input: 'Ｆｉｌｅ.txt', output: 'File.txt' },
    { form: 'nfkd' as const, input: 'ﬁle.txt', output: 'file.txt' },
  ];
  for (const { form, input, output } of forms) {
    const request: RulePipelineRequest = {
      schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
      overrides: [],
      rules: [
        {
          kind: 'unicodeNormalization',
          ruleId: 103,
          enabled: true,
          target: 'stem',
          form,
        },
      ],
    };
    expect(applyBrowserRules(input, request).proposedName).toBe(output);
  }
});

test('selects ranges by Unicode scalar value from either edge', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules: [
      {
        kind: 'range',
        ruleId: 105,
        enabled: true,
        target: 'stem',
        operation: 'keep',
        origin: 'start',
        offset: 1,
        length: 3,
      },
    ],
  };

  expect(applyBrowserRules('A😀e\u0301Z.txt', request).proposedName).toBe('😀e\u0301.txt');

  request.rules = [
    {
      kind: 'range',
      ruleId: 106,
      enabled: true,
      target: 'stem',
      operation: 'remove',
      origin: 'end',
      offset: 1,
      length: 2,
    },
  ];
  expect(applyBrowserRules('A😀e\u0301Z.txt', request).proposedName).toBe('A😀Z.txt');

  request.rules = [
    {
      kind: 'range',
      ruleId: 107,
      enabled: true,
      target: 'stem',
      operation: 'keep',
      origin: 'start',
      offset: 99,
      length: null,
    },
  ];
  expect(applyBrowserRules('short.txt', request).proposedName).toBe('.txt');
});

test('filters filename parts with Unicode character properties', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules: [
      {
        kind: 'characterClass',
        ruleId: 109,
        enabled: true,
        target: 'stem',
        operation: 'remove',
        class: 'decimalNumber',
      },
    ],
  };

  expect(applyBrowserRules('Ａ١2한!-★.txt', request).proposedName).toBe('Ａ한!-★.txt');

  request.rules = [
    {
      kind: 'characterClass',
      ruleId: 110,
      enabled: true,
      target: 'stem',
      operation: 'keep',
      class: 'letter',
    },
  ];
  expect(applyBrowserRules('Ａ١2한!-★.txt', request).proposedName).toBe('Ａ한.txt');
});

test('applies stable source overrides after shared rules and validates them pathlessly', () => {
  const sources = [
    { sourceId: 2, parentId: 1, originalName: 'second.txt' },
    { sourceId: 1, parentId: 1, originalName: 'first.txt' },
  ];
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [{ sourceId: 1, value: 'manual.md' }],
    rules: [{ kind: 'prefix', ruleId: 113, enabled: true, value: 'shared-' }],
  };
  const apply = compileBrowserRulePipeline(request, sources);

  expect(apply(sources[0] as (typeof sources)[number])).toMatchObject({
    proposedName: 'shared-second.txt',
    overrideApplied: false,
  });
  expect(apply(sources[1] as (typeof sources)[number])).toMatchObject({
    proposedName: 'manual.md',
    overrideApplied: true,
    trace: [{ ruleId: 113, before: 'first.txt', after: 'shared-first.txt' }],
  });

  const invalidRequests: Array<[RulePipelineRequest, string]> = [
    [
      { ...request, overrides: [{ sourceId: 9, value: '/private/unknown.txt' }] },
      'unknownOverrideSource',
    ],
    [
      {
        ...request,
        overrides: [
          { sourceId: 1, value: 'first-private-value' },
          { sourceId: 1, value: 'second-private-value' },
        ],
      },
      'duplicateOverrideSourceId',
    ],
    [{ ...request, overrides: [{ sourceId: 1, value: 'x'.repeat(4_097) }] }, 'overrideTextTooLong'],
  ];
  for (const [invalid, code] of invalidRequests) {
    try {
      compileBrowserRulePipeline(invalid, sources);
      throw new Error(`Expected ${code}.`);
    } catch (cause) {
      expect(cause).toMatchObject({ code });
      expect((cause as Error).message).not.toContain('private');
    }
  }
});

test('rejects invalid extension replacement without reflecting its value', () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules: [
      {
        kind: 'extension',
        ruleId: 107,
        enabled: false,
        operation: 'replace',
        value: '.sensitive-extension',
      },
    ],
  };

  try {
    applyBrowserRules('/home/private.txt', request);
    throw new Error('Expected invalid extension replacement.');
  } catch (cause) {
    expect(cause).toMatchObject({ code: 'invalidExtensionReplacement', ruleId: 107 });
    expect((cause as Error).message).not.toContain('sensitive-extension');
    expect((cause as Error).message).not.toContain('/home/private.txt');
  }
});
