import { expect, test } from 'vitest';
import { createPlanningClient } from '../../src/planning/client';
import {
  MAX_RULE_OUTPUT_BYTES,
  RULE_PIPELINE_SCHEMA_VERSION,
  type RulePipelineRequest,
} from '../../src/planning/rules';

test('keeps expanding browser previews and their inspection traces within the rule budget', async () => {
  const request: RulePipelineRequest = {
    schemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    overrides: [],
    rules: [43, 47].map((ruleId) => ({
      kind: 'regexReplace' as const,
      ruleId,
      enabled: true,
      pattern: '',
      replacement: 'x'.repeat(MAX_RULE_OUTPUT_BYTES / 64),
    })),
  };
  const client = createPlanningClient();

  const plan = await client.loadSample(request);

  expect(plan.rows).toHaveLength(3);
  for (const row of plan.rows) {
    expect(new TextEncoder().encode(row.proposedName).length).toBeLessThanOrEqual(
      MAX_RULE_OUTPUT_BYTES
    );
    expect(row.status).toBe('blocked');
    expect(row.diagnostics).toContain('nameTooLong');
  }

  const inspection = JSON.parse(await client.inspectPlan(plan.planId)) as {
    rows: Array<{ trace: Array<{ after: string }> }>;
  };
  for (const row of inspection.rows) {
    expect(row.trace).toHaveLength(1);
    expect(new TextEncoder().encode(row.trace[0]?.after ?? '').length).toBeLessThanOrEqual(
      MAX_RULE_OUTPUT_BYTES
    );
  }
});
