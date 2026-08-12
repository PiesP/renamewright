import { performance } from 'node:perf_hooks';
import {
  compileBrowserRulePipeline,
  createBrowserTraceBudget,
  MAX_PLAN_TRACE_BYTES,
  MAX_RULES,
  RULE_PIPELINE_SCHEMA_VERSION,
} from '../src/planning/rules.ts';

const SOURCE_COUNT = 10_000;
const REPRESENTATIVE_PLAN_BUDGET_MS = 3_000;
const EXPANDING_PLAN_BUDGET_MS = 8_000;
const FILTER_BUDGET_MS = 250;
const RETAINED_HEAP_BUDGET_BYTES = 192 * 1024 * 1024;

if (typeof globalThis.gc !== 'function') {
  throw new Error('Run the browser performance budget with node --expose-gc.');
}

const sources = Array.from({ length: SOURCE_COUNT }, (_, index) => ({
  sourceId: index + 1,
  parentId: 1,
  originalName: `Quarterly review ${index.toString().padStart(5, '0')}.txt`,
}));

globalThis.gc();
const baselineHeapBytes = process.memoryUsage().heapUsed;
const representative = measurePipeline(
  [
    { kind: 'prefix', ruleId: 1, enabled: true, value: '2026-' },
    { kind: 'suffix', ruleId: 2, enabled: true, value: '-final' },
    {
      kind: 'literalReplace',
      ruleId: 3,
      enabled: true,
      search: 'review',
      replacement: 'archive',
    },
    {
      kind: 'regexReplace',
      ruleId: 4,
      enabled: true,
      pattern: 'Quarterly',
      replacement: 'Q',
    },
  ],
  false
);
enforceDuration(
  'representative browser planning',
  representative.elapsedMs,
  REPRESENTATIVE_PLAN_BUDGET_MS
);
if (representative.results[0]?.proposedName !== '2026-Q archive 00000.txt-final') {
  throw new Error('The representative browser pipeline changed fixture semantics.');
}
if (representative.traceBudget.retainedBytes > MAX_PLAN_TRACE_BYTES) {
  throw new Error('The representative browser pipeline exceeded the trace budget.');
}

const rows = representative.results.map((result, index) => ({
  sourceId: index + 1,
  status: index % 10 === 0 ? 'blocked' : 'changed',
  proposedName: result.proposedName,
}));
const filterStarted = performance.now();
const blockedRows = rows.filter((row) => row.status === 'blocked');
const filterElapsedMs = performance.now() - filterStarted;
enforceDuration('10,000-row filtering', filterElapsedMs, FILTER_BUDGET_MS);
if (blockedRows.length !== 1_000) {
  throw new Error('The browser filter fixture produced an unexpected result.');
}

const expanding = measurePipeline(
  Array.from({ length: MAX_RULES }, (_, index) => ({
    kind: 'prefix',
    ruleId: index + 1,
    enabled: true,
    value: 'x'.repeat(120),
  })),
  true
);
enforceDuration('expanding browser planning', expanding.elapsedMs, EXPANDING_PLAN_BUDGET_MS);
if (
  expanding.results.length !== SOURCE_COUNT ||
  !expanding.results.every((result) => result.proposedName.startsWith('x'.repeat(120 * MAX_RULES)))
) {
  throw new Error('The expanding browser pipeline changed fixture semantics.');
}
if (
  expanding.traceBudget.retainedBytes > MAX_PLAN_TRACE_BYTES ||
  !expanding.results.some((result) => result.traceTruncated)
) {
  throw new Error('The expanding browser pipeline did not enforce the trace budget.');
}

globalThis.gc();
const retainedHeapBytes = process.memoryUsage().heapUsed - baselineHeapBytes;
if (retainedHeapBytes > RETAINED_HEAP_BUDGET_BYTES) {
  throw new Error(
    `Browser retained heap ${retainedHeapBytes} exceeded ${RETAINED_HEAP_BUDGET_BYTES} bytes.`
  );
}

console.log(
  JSON.stringify({
    runtime: 'browser',
    sourceCount: SOURCE_COUNT,
    representativePlanMs: Math.round(representative.elapsedMs),
    expandingPlanMs: Math.round(expanding.elapsedMs),
    filterMs: Math.round(filterElapsedMs),
    retainedHeapBytes,
    retainedHeapBudgetBytes: RETAINED_HEAP_BUDGET_BYTES,
    retainedTraceBytes: expanding.traceBudget.retainedBytes,
    traceTruncatedRows: expanding.results.filter((result) => result.traceTruncated).length,
  })
);

function measurePipeline(rules, sampleHeap) {
  const request = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules,
  };
  const apply = compileBrowserRulePipeline(request, sources);
  const traceBudget = createBrowserTraceBudget();
  const results = [];
  let peakHeapBytes = process.memoryUsage().heapUsed;
  const started = performance.now();
  for (const [index, source] of sources.entries()) {
    results.push(apply(source, traceBudget));
    if (sampleHeap && index % 250 === 0) {
      peakHeapBytes = Math.max(peakHeapBytes, process.memoryUsage().heapUsed);
    }
  }
  const elapsedMs = performance.now() - started;
  if (sampleHeap && peakHeapBytes - baselineHeapBytes > RETAINED_HEAP_BUDGET_BYTES) {
    throw new Error(
      `Browser sampled heap ${peakHeapBytes - baselineHeapBytes} exceeded ${RETAINED_HEAP_BUDGET_BYTES} bytes.`
    );
  }
  return { elapsedMs, results, traceBudget };
}

function enforceDuration(label, elapsedMs, budgetMs) {
  if (elapsedMs > budgetMs) {
    throw new Error(`${label} took ${Math.round(elapsedMs)}ms, exceeding ${budgetMs}ms.`);
  }
}
