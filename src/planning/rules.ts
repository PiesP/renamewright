export const RULE_PIPELINE_SCHEMA_VERSION = 1;
export const MAX_RULES = 32;
export const MAX_RULE_TEXT_BYTES = 4_096;

interface RuleBase {
  ruleId: number;
  enabled: boolean;
}

export interface PrefixRule extends RuleBase {
  kind: 'prefix';
  value: string;
}

export interface SuffixRule extends RuleBase {
  kind: 'suffix';
  value: string;
}

export interface LiteralReplaceRule extends RuleBase {
  kind: 'literalReplace';
  search: string;
  replacement: string;
}

export interface RegexReplaceRule extends RuleBase {
  kind: 'regexReplace';
  pattern: string;
  replacement: string;
}

export type RuleRequest = PrefixRule | SuffixRule | LiteralReplaceRule | RegexReplaceRule;
export type RuleKind = RuleRequest['kind'];

export interface RulePipelineRequest {
  schemaVersion: number;
  rules: RuleRequest[];
}

export interface RuleTraceStep {
  ruleIndex: number;
  ruleId: number;
  before: string;
  after: string;
}

export class PlanningError extends Error {
  readonly code: string;
  readonly ruleId: number | undefined;

  constructor(code: string, message: string, ruleId?: number) {
    super(message);
    this.name = 'PlanningError';
    this.code = code;
    this.ruleId = ruleId;
  }
}

export function createRule(ruleId: number, kind: RuleKind): RuleRequest {
  switch (kind) {
    case 'prefix':
      return { kind, ruleId, enabled: true, value: '' };
    case 'suffix':
      return { kind, ruleId, enabled: true, value: '' };
    case 'literalReplace':
      return { kind, ruleId, enabled: true, search: '', replacement: '' };
    case 'regexReplace':
      return { kind, ruleId, enabled: true, pattern: '', replacement: '' };
  }
}

export function ruleLabel(kind: RuleKind): string {
  const labels: Record<RuleKind, string> = {
    prefix: 'Add prefix',
    suffix: 'Add suffix',
    literalReplace: 'Replace text',
    regexReplace: 'Replace by pattern',
  };
  return labels[kind];
}

export function planningError(cause: unknown): PlanningError {
  const record = typeof cause === 'object' && cause !== null ? cause : undefined;
  const code = record && 'code' in record ? (record as { code?: unknown }).code : undefined;
  const ruleId = record && 'ruleId' in record ? (record as { ruleId?: unknown }).ruleId : undefined;
  const normalizedCode = typeof code === 'string' ? code : 'planningFailed';
  const normalizedRuleId = typeof ruleId === 'number' ? ruleId : undefined;
  const subject = normalizedRuleId === undefined ? 'The rule pipeline' : `Rule ${normalizedRuleId}`;
  const messages: Record<string, string> = {
    unsupportedRuleSchema: 'This rule format is not supported by this version of Renamewright.',
    tooManyRules: `A rule pipeline can contain at most ${MAX_RULES} rules.`,
    invalidRuleId: 'A rule has an invalid editing identifier.',
    duplicateRuleId: `${subject} reuses another rule identifier.`,
    ruleTextTooLong: `${subject} contains text longer than ${MAX_RULE_TEXT_BYTES.toLocaleString()} bytes.`,
    emptyLiteralSearch: `${subject} needs text to search for.`,
    invalidRegex: `${subject} uses an invalid regular expression.`,
    pickerUnavailable: 'The native file picker is unavailable.',
    sourceAdmissionFailed: 'One or more selected sources are no longer available.',
    stateUnavailable: 'The planning worker is unavailable.',
    planningFailed: 'The rename plan could not be updated.',
  };
  return new PlanningError(
    normalizedCode,
    messages[normalizedCode] ?? 'The rename plan could not be updated.',
    normalizedRuleId
  );
}

export function applyBrowserRules(
  originalName: string,
  request: RulePipelineRequest
): { proposedName: string; trace: RuleTraceStep[] } {
  return compileBrowserRulePipeline(request)(originalName);
}

export function compileBrowserRulePipeline(
  request: RulePipelineRequest
): (originalName: string) => { proposedName: string; trace: RuleTraceStep[] } {
  validateRequest(request);
  const compiledRules: Array<{
    ruleId: number;
    apply: (input: string) => string;
  }> = [];
  for (const rule of request.rules) {
    if (!rule.enabled) {
      continue;
    }
    switch (rule.kind) {
      case 'prefix':
        compiledRules.push({ ruleId: rule.ruleId, apply: (input) => `${rule.value}${input}` });
        break;
      case 'suffix':
        compiledRules.push({ ruleId: rule.ruleId, apply: (input) => `${input}${rule.value}` });
        break;
      case 'literalReplace':
        if (rule.search.length === 0) {
          throw new PlanningError(
            'emptyLiteralSearch',
            `Rule ${rule.ruleId} needs text to search for.`,
            rule.ruleId
          );
        }
        compiledRules.push({
          ruleId: rule.ruleId,
          apply: (input) => input.replaceAll(rule.search, rule.replacement),
        });
        break;
      case 'regexReplace':
        compiledRules.push({ ruleId: rule.ruleId, apply: compileRegexReplace(rule) });
        break;
    }
  }
  return (originalName) => {
    let proposedName = originalName;
    const trace: RuleTraceStep[] = [];
    for (const [ruleIndex, rule] of compiledRules.entries()) {
      const before = proposedName;
      proposedName = rule.apply(proposedName);
      trace.push({ ruleIndex, ruleId: rule.ruleId, before, after: proposedName });
    }
    return { proposedName, trace };
  };
}

function validateRequest(request: RulePipelineRequest): void {
  if (request.schemaVersion !== RULE_PIPELINE_SCHEMA_VERSION) {
    throw new PlanningError(
      'unsupportedRuleSchema',
      'This rule format is not supported by this version of Renamewright.'
    );
  }
  if (request.rules.length > MAX_RULES) {
    throw new PlanningError(
      'tooManyRules',
      `A rule pipeline can contain at most ${MAX_RULES} rules.`
    );
  }
  const ids = new Set<number>();
  for (const rule of request.rules) {
    if (!Number.isSafeInteger(rule.ruleId) || rule.ruleId <= 0) {
      throw new PlanningError(
        'invalidRuleId',
        'A rule has an invalid editing identifier.',
        rule.ruleId
      );
    }
    if (ids.has(rule.ruleId)) {
      throw new PlanningError(
        'duplicateRuleId',
        `Rule ${rule.ruleId} reuses another rule identifier.`,
        rule.ruleId
      );
    }
    ids.add(rule.ruleId);
    if (
      ruleText(rule).some((text) => new TextEncoder().encode(text).length > MAX_RULE_TEXT_BYTES)
    ) {
      throw new PlanningError(
        'ruleTextTooLong',
        `Rule ${rule.ruleId} contains text longer than ${MAX_RULE_TEXT_BYTES.toLocaleString()} bytes.`,
        rule.ruleId
      );
    }
  }
}

function ruleText(rule: RuleRequest): string[] {
  switch (rule.kind) {
    case 'prefix':
    case 'suffix':
      return [rule.value];
    case 'literalReplace':
      return [rule.search, rule.replacement];
    case 'regexReplace':
      return [rule.pattern, rule.replacement];
  }
}

function compileRegexReplace(rule: RegexReplaceRule): (input: string) => string {
  if (/\(\?(?:[=!]|<[=!])|\\[1-9]/u.test(rule.pattern)) {
    throw new PlanningError(
      'invalidRegex',
      `Rule ${rule.ruleId} uses a regular-expression feature unsupported by Rust regex.`,
      rule.ruleId
    );
  }
  const browserPattern = rule.pattern.replace(/\(\?P<([A-Za-z_][A-Za-z0-9_]*)>/gu, '(?<$1>');
  let expression: RegExp;
  try {
    expression = new RegExp(browserPattern, 'gu');
  } catch {
    throw new PlanningError(
      'invalidRegex',
      `Rule ${rule.ruleId} uses an invalid regular expression.`,
      rule.ruleId
    );
  }
  return (input) =>
    input.replace(expression, (match, ...arguments_: unknown[]) => {
      const possibleGroups = arguments_.at(-1);
      const hasGroups = typeof possibleGroups === 'object' && possibleGroups !== null;
      const captureEnd = arguments_.length - (hasGroups ? 3 : 2);
      const captures = [match, ...arguments_.slice(0, captureEnd)].map((capture) =>
        typeof capture === 'string' ? capture : ''
      );
      const groups = hasGroups ? (possibleGroups as Record<string, string | undefined>) : {};
      return expandReplacement(rule.replacement, captures, groups);
    });
}

function expandReplacement(
  template: string,
  captures: string[],
  groups: Record<string, string | undefined>
): string {
  return template.replace(
    /\$(?:\$|\{([^}]+)\}|([0-9A-Za-z_]+))/gu,
    (token, braced: string | undefined, unbraced: string | undefined) => {
      if (token === '$$') {
        return '$';
      }
      const key = braced ?? unbraced ?? '';
      if (/^[0-9]+$/u.test(key)) {
        return captures[Number(key)] ?? '';
      }
      return groups[key] ?? '';
    }
  );
}
