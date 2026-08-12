export const RULE_PIPELINE_SCHEMA_VERSION = 4;
export const MAX_RULES = 32;
export const MAX_RULE_TEXT_BYTES = 4_096;
export const MAX_RULE_OUTPUT_BYTES = 4_096;
export const MAX_SEQUENCE_PADDING = 20;
export const MAX_OVERRIDES = 100_000;
export const MAX_OVERRIDE_TEXT_BYTES = 4_096;
export const MAX_RANGE_INTEGER = 4_294_967_295;
const MAX_U64 = 18_446_744_073_709_551_615n;
const RULE_OUTPUT_TOO_LONG = Symbol('ruleOutputTooLong');
const textEncoder = new TextEncoder();

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

export type SequenceScope = 'allSources' | 'perParent';
export type SequenceOrder = 'sourceOrder' | 'nameAscending';
export type SequencePlacement = 'prefix' | 'suffix';

export interface SequenceRule extends RuleBase {
  kind: 'sequence';
  scope: SequenceScope;
  order: SequenceOrder;
  start: number;
  step: number;
  padding: number;
  placement: SequencePlacement;
  separator: string;
}

export type FilenamePart = 'wholeName' | 'stem' | 'extension';

export interface ExtensionRule extends RuleBase {
  kind: 'extension';
  operation: 'remove' | 'replace';
  value: string;
}

export interface CaseRule extends RuleBase {
  kind: 'case';
  target: FilenamePart;
  mode: 'lowercase' | 'uppercase';
}

export interface WhitespaceCleanupRule extends RuleBase {
  kind: 'whitespaceCleanup';
  target: FilenamePart;
  replacement: string;
}

export interface UnicodeNormalizationRule extends RuleBase {
  kind: 'unicodeNormalization';
  target: FilenamePart;
  form: 'nfc' | 'nfd' | 'nfkc' | 'nfkd';
}

export interface RangeRule extends RuleBase {
  kind: 'range';
  target: FilenamePart;
  operation: 'keep' | 'remove';
  origin: 'start' | 'end';
  offset: number;
  length: number | null;
}

export interface CharacterClassRule extends RuleBase {
  kind: 'characterClass';
  target: FilenamePart;
  operation: 'keep' | 'remove';
  class: 'decimalNumber' | 'letter' | 'whitespace' | 'punctuation' | 'symbol';
}

export interface SourceOverride {
  sourceId: number;
  value: string;
}

export type RuleRequest =
  | PrefixRule
  | SuffixRule
  | LiteralReplaceRule
  | RegexReplaceRule
  | SequenceRule
  | ExtensionRule
  | CaseRule
  | WhitespaceCleanupRule
  | UnicodeNormalizationRule
  | RangeRule
  | CharacterClassRule;
export type RuleKind = RuleRequest['kind'];

export interface RulePipelineRequest {
  schemaVersion: number;
  rules: RuleRequest[];
  overrides: SourceOverride[];
}

export interface RuleTraceStep {
  ruleIndex: number;
  ruleId: number;
  before: string;
  after: string;
}

export interface BrowserRuleSource {
  sourceId: number;
  parentId: number;
  originalName: string;
}

export interface BrowserRuleResult {
  proposedName: string;
  trace: RuleTraceStep[];
  overrideApplied: boolean;
  diagnostic?: 'nameTooLong' | 'sequenceOverflow';
}

export class PlanningError extends Error {
  readonly code: string;
  readonly ruleId: number | undefined;
  readonly sourceId: number | undefined;

  constructor(code: string, message: string, ruleId?: number, sourceId?: number) {
    super(message);
    this.name = 'PlanningError';
    this.code = code;
    this.ruleId = ruleId;
    this.sourceId = sourceId;
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
    case 'sequence':
      return {
        kind,
        ruleId,
        enabled: true,
        scope: 'allSources',
        order: 'sourceOrder',
        start: 1,
        step: 1,
        padding: 3,
        placement: 'prefix',
        separator: '-',
      };
    case 'extension':
      return { kind, ruleId, enabled: true, operation: 'remove', value: 'txt' };
    case 'case':
      return { kind, ruleId, enabled: true, target: 'wholeName', mode: 'lowercase' };
    case 'whitespaceCleanup':
      return { kind, ruleId, enabled: true, target: 'wholeName', replacement: ' ' };
    case 'unicodeNormalization':
      return { kind, ruleId, enabled: true, target: 'wholeName', form: 'nfc' };
    case 'range':
      return {
        kind,
        ruleId,
        enabled: true,
        target: 'stem',
        operation: 'keep',
        origin: 'start',
        offset: 0,
        length: 1,
      };
    case 'characterClass':
      return {
        kind,
        ruleId,
        enabled: true,
        target: 'stem',
        operation: 'remove',
        class: 'decimalNumber',
      };
  }
}

export function ruleLabel(kind: RuleKind): string {
  const labels: Record<RuleKind, string> = {
    prefix: 'Add prefix',
    suffix: 'Add suffix',
    literalReplace: 'Replace text',
    regexReplace: 'Replace by pattern',
    sequence: 'Add sequence',
    extension: 'Change extension',
    case: 'Change case',
    whitespaceCleanup: 'Clean whitespace',
    unicodeNormalization: 'Normalize Unicode',
    range: 'Select character range',
    characterClass: 'Filter character class',
  };
  return labels[kind];
}

export function planningError(cause: unknown): PlanningError {
  const record = typeof cause === 'object' && cause !== null ? cause : undefined;
  const code = record && 'code' in record ? (record as { code?: unknown }).code : undefined;
  const ruleId = record && 'ruleId' in record ? (record as { ruleId?: unknown }).ruleId : undefined;
  const sourceId =
    record && 'sourceId' in record ? (record as { sourceId?: unknown }).sourceId : undefined;
  const normalizedCode = typeof code === 'string' ? code : 'planningFailed';
  const normalizedRuleId = typeof ruleId === 'number' ? ruleId : undefined;
  const normalizedSourceId = typeof sourceId === 'number' ? sourceId : undefined;
  const subject = normalizedRuleId === undefined ? 'The rule pipeline' : `Rule ${normalizedRuleId}`;
  const messages: Record<string, string> = {
    unsupportedRuleSchema: 'This rule format is not supported by this version of Renamewright.',
    tooManyRules: `A rule pipeline can contain at most ${MAX_RULES} rules.`,
    invalidRuleId: 'A rule has an invalid editing identifier.',
    duplicateRuleId: `${subject} reuses another rule identifier.`,
    ruleTextTooLong: `${subject} contains text longer than ${MAX_RULE_TEXT_BYTES.toLocaleString()} bytes.`,
    emptyLiteralSearch: `${subject} needs text to search for.`,
    invalidRegex: `${subject} uses an invalid regular expression.`,
    invalidSequenceStart: `${subject} needs a non-negative safe whole-number start.`,
    invalidSequenceStep: `${subject} needs a positive whole-number step.`,
    invalidSequencePadding: `${subject} needs padding from 1 through ${MAX_SEQUENCE_PADDING}.`,
    invalidExtensionReplacement: `${subject} needs an extension without a leading dot.`,
    invalidRangeOffset: `${subject} needs a non-negative range offset.`,
    invalidRangeLength: `${subject} needs a positive range length or an open-ended range.`,
    tooManyOverrides: `A plan can contain at most ${MAX_OVERRIDES.toLocaleString()} filename overrides.`,
    invalidOverrideSourceId: 'A filename override has an invalid source identifier.',
    duplicateOverrideSourceId: 'A source has more than one filename override.',
    overrideTextTooLong: `A filename override is longer than ${MAX_OVERRIDE_TEXT_BYTES.toLocaleString()} bytes.`,
    unknownOverrideSource: 'A filename override no longer belongs to the current source set.',
    pickerUnavailable: 'The native file picker is unavailable.',
    sourceAdmissionFailed: 'One or more selected sources are no longer available.',
    stateUnavailable: 'The planning worker is unavailable.',
    planningFailed: 'The rename plan could not be updated.',
  };
  return new PlanningError(
    normalizedCode,
    messages[normalizedCode] ?? 'The rename plan could not be updated.',
    normalizedRuleId,
    normalizedSourceId
  );
}

export function applyBrowserRules(
  originalName: string,
  request: RulePipelineRequest
): BrowserRuleResult {
  const source = { sourceId: 1, parentId: 1, originalName };
  return compileBrowserRulePipeline(request, [source])(source);
}

export function compileBrowserRulePipeline(
  request: RulePipelineRequest,
  sources: readonly BrowserRuleSource[]
): (source: BrowserRuleSource) => BrowserRuleResult {
  validateRequest(request);
  const sourceIds = new Set(sources.map((source) => source.sourceId));
  const overrides = new Map<number, string>();
  for (const nameOverride of request.overrides) {
    if (!sourceIds.has(nameOverride.sourceId)) {
      throw new PlanningError(
        'unknownOverrideSource',
        'A filename override no longer belongs to the current source set.',
        undefined,
        nameOverride.sourceId
      );
    }
    overrides.set(nameOverride.sourceId, nameOverride.value);
  }
  const compiledRules: Array<{
    ruleId: number;
    apply: (input: string, source: BrowserRuleSource) => string | undefined;
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
          apply: (input) => replaceLiteralBounded(input, rule.search, rule.replacement),
        });
        break;
      case 'regexReplace':
        compiledRules.push({ ruleId: rule.ruleId, apply: compileRegexReplace(rule) });
        break;
      case 'sequence': {
        const values = allocateSequence(sources, rule);
        compiledRules.push({
          ruleId: rule.ruleId,
          apply: (input, source) => {
            const value = values.get(source.sourceId);
            if (value === undefined) {
              return undefined;
            }
            const number = String(value).padStart(rule.padding, '0');
            return rule.placement === 'prefix'
              ? `${number}${rule.separator}${input}`
              : `${input}${rule.separator}${number}`;
          },
        });
        break;
      }
      case 'extension':
        compiledRules.push({
          ruleId: rule.ruleId,
          apply: (input) => applyExtension(input, rule.operation, rule.value),
        });
        break;
      case 'case':
        compiledRules.push({
          ruleId: rule.ruleId,
          apply: (input) =>
            transformFilenamePart(input, rule.target, (text) =>
              rule.mode === 'lowercase' ? text.toLowerCase() : text.toUpperCase()
            ),
        });
        break;
      case 'whitespaceCleanup':
        compiledRules.push({
          ruleId: rule.ruleId,
          apply: (input) =>
            transformFilenamePart(input, rule.target, (text) =>
              cleanupWhitespace(text, rule.replacement)
            ),
        });
        break;
      case 'unicodeNormalization':
        compiledRules.push({
          ruleId: rule.ruleId,
          apply: (input) =>
            transformFilenamePart(input, rule.target, (text) =>
              text.normalize(rule.form.toUpperCase() as 'NFC' | 'NFD' | 'NFKC' | 'NFKD')
            ),
        });
        break;
      case 'range':
        compiledRules.push({
          ruleId: rule.ruleId,
          apply: (input) =>
            transformFilenamePart(input, rule.target, (text) =>
              applyRange(text, rule.operation, rule.origin, rule.offset, rule.length)
            ),
        });
        break;
      case 'characterClass': {
        const matcher = characterClassMatcher(rule.class);
        compiledRules.push({
          ruleId: rule.ruleId,
          apply: (input) =>
            transformFilenamePart(input, rule.target, (text) =>
              [...text]
                .filter((character) =>
                  rule.operation === 'keep' ? matcher.test(character) : !matcher.test(character)
                )
                .join('')
            ),
        });
        break;
      }
    }
  }
  return (source) => {
    let proposedName = source.originalName;
    const trace: RuleTraceStep[] = [];
    if (utf8Length(proposedName) > MAX_RULE_OUTPUT_BYTES) {
      return { proposedName, trace, overrideApplied: false, diagnostic: 'nameTooLong' };
    }
    for (const [ruleIndex, rule] of compiledRules.entries()) {
      const before = proposedName;
      let after: string | undefined;
      try {
        after = rule.apply(proposedName, source);
      } catch (cause) {
        if (cause === RULE_OUTPUT_TOO_LONG) {
          return { proposedName, trace, overrideApplied: false, diagnostic: 'nameTooLong' };
        }
        throw cause;
      }
      if (after === undefined) {
        return { proposedName, trace, overrideApplied: false, diagnostic: 'sequenceOverflow' };
      }
      if (utf8Length(after) > MAX_RULE_OUTPUT_BYTES) {
        return { proposedName, trace, overrideApplied: false, diagnostic: 'nameTooLong' };
      }
      proposedName = after;
      trace.push({ ruleIndex, ruleId: rule.ruleId, before, after: proposedName });
    }
    const nameOverride = overrides.get(source.sourceId);
    return nameOverride === undefined
      ? { proposedName, trace, overrideApplied: false }
      : { proposedName: nameOverride, trace, overrideApplied: true };
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
  if (request.overrides.length > MAX_OVERRIDES) {
    throw new PlanningError(
      'tooManyOverrides',
      `A plan can contain at most ${MAX_OVERRIDES.toLocaleString()} filename overrides.`
    );
  }
  const overrideIds = new Set<number>();
  for (const nameOverride of request.overrides) {
    if (!Number.isSafeInteger(nameOverride.sourceId) || nameOverride.sourceId <= 0) {
      throw new PlanningError(
        'invalidOverrideSourceId',
        'A filename override has an invalid source identifier.',
        undefined,
        nameOverride.sourceId
      );
    }
    if (overrideIds.has(nameOverride.sourceId)) {
      throw new PlanningError(
        'duplicateOverrideSourceId',
        'A source has more than one filename override.',
        undefined,
        nameOverride.sourceId
      );
    }
    overrideIds.add(nameOverride.sourceId);
    if (utf8Length(nameOverride.value) > MAX_OVERRIDE_TEXT_BYTES) {
      throw new PlanningError(
        'overrideTextTooLong',
        `A filename override is longer than ${MAX_OVERRIDE_TEXT_BYTES.toLocaleString()} bytes.`,
        undefined,
        nameOverride.sourceId
      );
    }
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
    if (ruleText(rule).some((text) => utf8Length(text) > MAX_RULE_TEXT_BYTES)) {
      throw new PlanningError(
        'ruleTextTooLong',
        `Rule ${rule.ruleId} contains text longer than ${MAX_RULE_TEXT_BYTES.toLocaleString()} bytes.`,
        rule.ruleId
      );
    }
    if (rule.kind === 'range') {
      if (!Number.isInteger(rule.offset) || rule.offset < 0 || rule.offset > MAX_RANGE_INTEGER) {
        throw new PlanningError(
          'invalidRangeOffset',
          `Rule ${rule.ruleId} needs a non-negative range offset.`,
          rule.ruleId
        );
      }
      if (
        rule.length !== null &&
        (!Number.isInteger(rule.length) || rule.length < 1 || rule.length > MAX_RANGE_INTEGER)
      ) {
        throw new PlanningError(
          'invalidRangeLength',
          `Rule ${rule.ruleId} needs a positive range length or an open-ended range.`,
          rule.ruleId
        );
      }
    }
    if (rule.kind === 'sequence') {
      if (!Number.isSafeInteger(rule.start) || rule.start < 0) {
        throw new PlanningError(
          'invalidSequenceStart',
          `Rule ${rule.ruleId} needs a non-negative whole-number start.`,
          rule.ruleId
        );
      }
      if (!Number.isSafeInteger(rule.step) || rule.step <= 0) {
        throw new PlanningError(
          'invalidSequenceStep',
          `Rule ${rule.ruleId} needs a positive whole-number step.`,
          rule.ruleId
        );
      }
      if (
        !Number.isSafeInteger(rule.padding) ||
        rule.padding < 1 ||
        rule.padding > MAX_SEQUENCE_PADDING
      ) {
        throw new PlanningError(
          'invalidSequencePadding',
          `Rule ${rule.ruleId} needs padding from 1 through ${MAX_SEQUENCE_PADDING}.`,
          rule.ruleId
        );
      }
    }
    if (
      rule.kind === 'extension' &&
      rule.operation === 'replace' &&
      (rule.value.length === 0 || rule.value.startsWith('.'))
    ) {
      throw new PlanningError(
        'invalidExtensionReplacement',
        `Rule ${rule.ruleId} needs an extension without a leading dot.`,
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
    case 'sequence':
      return [rule.separator];
    case 'extension':
      return [rule.value];
    case 'whitespaceCleanup':
      return [rule.replacement];
    case 'case':
    case 'unicodeNormalization':
    case 'range':
    case 'characterClass':
      return [];
  }
}

function extensionBoundary(name: string): number | undefined {
  const index = name.lastIndexOf('.');
  return index > 0 ? index : undefined;
}

function applyExtension(
  name: string,
  operation: ExtensionRule['operation'],
  value: string
): string {
  const boundary = extensionBoundary(name);
  if (operation === 'remove') {
    return boundary === undefined ? name : name.slice(0, boundary);
  }
  return `${boundary === undefined ? name : name.slice(0, boundary)}.${value}`;
}

function transformFilenamePart(
  name: string,
  target: FilenamePart,
  transform: (text: string) => string
): string {
  if (target === 'wholeName') {
    return transform(name);
  }
  const boundary = extensionBoundary(name);
  if (target === 'stem') {
    return boundary === undefined
      ? transform(name)
      : `${transform(name.slice(0, boundary))}${name.slice(boundary)}`;
  }
  return boundary === undefined
    ? name
    : `${name.slice(0, boundary + 1)}${transform(name.slice(boundary + 1))}`;
}

function cleanupWhitespace(text: string, replacement: string): string {
  return text
    .replace(/^\p{White_Space}+|\p{White_Space}+$/gu, '')
    .replace(/\p{White_Space}+/gu, replacement);
}

function applyRange(
  text: string,
  operation: RangeRule['operation'],
  origin: RangeRule['origin'],
  offset: number,
  length: number | null
): string {
  const characters = [...text];
  let start: number;
  let end: number;
  if (origin === 'start') {
    start = Math.min(offset, characters.length);
    end = length === null ? characters.length : Math.min(start + length, characters.length);
  } else {
    end = Math.max(characters.length - offset, 0);
    start = length === null ? 0 : Math.max(end - length, 0);
  }
  return operation === 'keep'
    ? characters.slice(start, end).join('')
    : [...characters.slice(0, start), ...characters.slice(end)].join('');
}

function characterClassMatcher(characterClass: CharacterClassRule['class']): RegExp {
  const patterns: Record<CharacterClassRule['class'], RegExp> = {
    decimalNumber: /^\p{Decimal_Number}$/u,
    letter: /^\p{Letter}$/u,
    whitespace: /^\p{White_Space}$/u,
    punctuation: /^\p{Punctuation}$/u,
    symbol: /^\p{Symbol}$/u,
  };
  return patterns[characterClass];
}

function allocateSequence(
  sources: readonly BrowserRuleSource[],
  rule: SequenceRule
): Map<number, bigint | undefined> {
  const ordered = [...sources].sort((left, right) => {
    if (rule.order === 'sourceOrder') {
      return left.sourceId - right.sourceId;
    }
    if (left.originalName !== right.originalName) {
      return compareUnicodeScalars(left.originalName, right.originalName);
    }
    return left.sourceId - right.sourceId;
  });
  let globalOrdinal = 0;
  const parentOrdinals = new Map<number, number>();
  return new Map(
    ordered.map((source) => {
      const ordinal =
        rule.scope === 'allSources' ? globalOrdinal++ : (parentOrdinals.get(source.parentId) ?? 0);
      if (rule.scope === 'perParent') {
        parentOrdinals.set(source.parentId, ordinal + 1);
      }
      const value = BigInt(rule.start) + BigInt(rule.step) * BigInt(ordinal);
      return [source.sourceId, value <= MAX_U64 ? value : undefined] as const;
    })
  );
}

function compareUnicodeScalars(left: string, right: string): number {
  const leftScalars = [...left].map((value) => value.codePointAt(0) ?? 0);
  const rightScalars = [...right].map((value) => value.codePointAt(0) ?? 0);
  const sharedLength = Math.min(leftScalars.length, rightScalars.length);
  for (let index = 0; index < sharedLength; index += 1) {
    const difference = (leftScalars[index] ?? 0) - (rightScalars[index] ?? 0);
    if (difference !== 0) {
      return difference;
    }
  }
  return leftScalars.length - rightScalars.length;
}

function utf8Length(value: string): number {
  return textEncoder.encode(value).length;
}

function consumeOutputBytes(byteLength: { value: number }, chunk: string): void {
  const nextLength = byteLength.value + utf8Length(chunk);
  if (!Number.isSafeInteger(nextLength) || nextLength > MAX_RULE_OUTPUT_BYTES) {
    throw RULE_OUTPUT_TOO_LONG;
  }
  byteLength.value = nextLength;
}

function appendBoundedChunk(chunks: string[], byteLength: { value: number }, chunk: string): void {
  consumeOutputBytes(byteLength, chunk);
  chunks.push(chunk);
}

function replaceLiteralBounded(input: string, search: string, replacement: string): string {
  const chunks: string[] = [];
  const byteLength = { value: 0 };
  let consumed = 0;
  let offset = input.indexOf(search);
  while (offset !== -1) {
    appendBoundedChunk(chunks, byteLength, input.slice(consumed, offset));
    appendBoundedChunk(chunks, byteLength, replacement);
    consumed = offset + search.length;
    offset = input.indexOf(search, consumed);
  }
  appendBoundedChunk(chunks, byteLength, input.slice(consumed));
  return chunks.join('');
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
  return (input) => {
    const outputLength = { value: 0 };
    let consumed = 0;
    const result = input.replace(expression, (match, ...arguments_: unknown[]) => {
      const possibleGroups = arguments_.at(-1);
      const hasGroups = typeof possibleGroups === 'object' && possibleGroups !== null;
      const captureEnd = arguments_.length - (hasGroups ? 3 : 2);
      const offset = arguments_[captureEnd];
      if (typeof offset !== 'number') {
        throw new PlanningError(
          'invalidRegex',
          `Rule ${rule.ruleId} could not apply its regular expression.`,
          rule.ruleId
        );
      }
      const captures = [match, ...arguments_.slice(0, captureEnd)].map((capture) =>
        typeof capture === 'string' ? capture : ''
      );
      const groups = hasGroups ? (possibleGroups as Record<string, string | undefined>) : {};
      const replacement = expandReplacement(rule.replacement, captures, groups);
      consumeOutputBytes(outputLength, input.slice(consumed, offset));
      consumeOutputBytes(outputLength, replacement);
      consumed = offset + match.length;
      return replacement;
    });
    consumeOutputBytes(outputLength, input.slice(consumed));
    return result;
  };
}

function expandReplacement(
  template: string,
  captures: string[],
  groups: Record<string, string | undefined>
): string {
  const chunks: string[] = [];
  const byteLength = { value: 0 };
  let consumed = 0;
  for (const match of template.matchAll(/\$(?:\$|\{([^}]+)\}|([0-9A-Za-z_]+))/gu)) {
    const token = match[0];
    const offset = match.index;
    appendBoundedChunk(chunks, byteLength, template.slice(consumed, offset));
    if (token === '$$') {
      appendBoundedChunk(chunks, byteLength, '$');
    } else {
      const key = match[1] ?? match[2] ?? '';
      appendBoundedChunk(
        chunks,
        byteLength,
        /^[0-9]+$/u.test(key) ? (captures[Number(key)] ?? '') : (groups[key] ?? '')
      );
    }
    consumed = offset + token.length;
  }
  appendBoundedChunk(chunks, byteLength, template.slice(consumed));
  return chunks.join('');
}
