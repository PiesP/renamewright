import {
  compileBrowserRulePipeline,
  MAX_RULES,
  RULE_PIPELINE_SCHEMA_VERSION,
  type RuleRequest,
} from './rules';

export const PRESET_DOCUMENT_SCHEMA_VERSION = 2;
export const PRESET_STORAGE_KEY = 'renamewright.rule-presets';
export const MAX_PRESETS = 32;
export const MAX_PRESET_NAME_BYTES = 256;
export const MAX_PRESET_DOCUMENT_BYTES = 512 * 1_024;

export interface RulePreset {
  presetId: number;
  name: string;
  ruleSchemaVersion: number;
  rules: RuleRequest[];
}

export interface PresetDocument {
  schemaVersion: typeof PRESET_DOCUMENT_SCHEMA_VERSION;
  nextPresetId: number;
  presets: RulePreset[];
}

export interface PresetReadResult {
  document: PresetDocument;
  migrated: boolean;
}

export interface PresetStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export type PresetErrorCode =
  | 'storageUnavailable'
  | 'unsupportedPresetSchema'
  | 'invalidPresetDocument'
  | 'presetDocumentTooLarge'
  | 'tooManyPresets'
  | 'invalidPresetName'
  | 'duplicatePresetName';

export class PresetError extends Error {
  readonly code: PresetErrorCode;

  constructor(code: PresetErrorCode, message: string) {
    super(message);
    this.name = 'PresetError';
    this.code = code;
  }
}

export function emptyPresetDocument(): PresetDocument {
  return { schemaVersion: PRESET_DOCUMENT_SCHEMA_VERSION, nextPresetId: 1, presets: [] };
}

export function readPresetDocument(storage: PresetStorage): PresetReadResult {
  let serialized: string | null;
  try {
    serialized = storage.getItem(PRESET_STORAGE_KEY);
  } catch {
    throw presetError('storageUnavailable');
  }
  if (serialized === null) {
    return { document: emptyPresetDocument(), migrated: false };
  }
  ensureDocumentSize(serialized);

  let value: unknown;
  try {
    value = JSON.parse(serialized) as unknown;
  } catch {
    throw presetError('invalidPresetDocument');
  }
  const record = objectRecord(value);
  if (!record || !Number.isSafeInteger(record.schemaVersion)) {
    throw presetError('invalidPresetDocument');
  }
  if (record.schemaVersion === PRESET_DOCUMENT_SCHEMA_VERSION) {
    return { document: parseVersionTwo(record), migrated: false };
  }
  if (record.schemaVersion === 1) {
    const document = migrateVersionOne(record);
    writePresetDocument(storage, document);
    return { document, migrated: true };
  }
  throw presetError('unsupportedPresetSchema');
}

export function writePresetDocument(storage: PresetStorage, document: PresetDocument): void {
  const normalized = parseVersionTwo(document as unknown as Record<string, unknown>);
  const serialized = JSON.stringify(normalized);
  ensureDocumentSize(serialized);
  try {
    storage.setItem(PRESET_STORAGE_KEY, serialized);
  } catch {
    throw presetError('storageUnavailable');
  }
}

export function addPreset(
  document: PresetDocument,
  name: string,
  rules: readonly RuleRequest[]
): PresetDocument {
  const current = parseVersionTwo(document as unknown as Record<string, unknown>);
  if (current.presets.length >= MAX_PRESETS) {
    throw presetError('tooManyPresets');
  }
  const normalizedName = normalizePresetName(name);
  if (current.presets.some((preset) => preset.name === normalizedName)) {
    throw presetError('duplicatePresetName');
  }
  const normalizedRules = parseRules(rules);
  const preset: RulePreset = {
    presetId: current.nextPresetId,
    name: normalizedName,
    ruleSchemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
    rules: normalizedRules,
  };
  return {
    ...current,
    nextPresetId: current.nextPresetId + 1,
    presets: [...current.presets, preset],
  };
}

export function deletePreset(document: PresetDocument, presetId: number): PresetDocument {
  const current = parseVersionTwo(document as unknown as Record<string, unknown>);
  return { ...current, presets: current.presets.filter((preset) => preset.presetId !== presetId) };
}

function parseVersionTwo(record: Record<string, unknown>): PresetDocument {
  if (
    record.schemaVersion !== PRESET_DOCUMENT_SCHEMA_VERSION ||
    !positiveSafeInteger(record.nextPresetId) ||
    !Array.isArray(record.presets) ||
    record.presets.length > MAX_PRESETS
  ) {
    throw presetError(
      Array.isArray(record.presets) && record.presets.length > MAX_PRESETS
        ? 'tooManyPresets'
        : 'invalidPresetDocument'
    );
  }
  const ids = new Set<number>();
  const names = new Set<string>();
  const presets = record.presets.map((value) => {
    const preset = objectRecord(value);
    if (
      !preset ||
      !positiveSafeInteger(preset.presetId) ||
      typeof preset.name !== 'string' ||
      preset.name !== preset.name.trim() ||
      preset.ruleSchemaVersion !== RULE_PIPELINE_SCHEMA_VERSION
    ) {
      throw presetError('invalidPresetDocument');
    }
    const name = normalizePresetName(preset.name);
    if (ids.has(preset.presetId) || names.has(name)) {
      throw presetError('invalidPresetDocument');
    }
    ids.add(preset.presetId);
    names.add(name);
    return {
      presetId: preset.presetId,
      name,
      ruleSchemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
      rules: parseRules(preset.rules),
    };
  });
  const maximumId = presets.reduce((maximum, preset) => Math.max(maximum, preset.presetId), 0);
  if (record.nextPresetId <= maximumId) {
    throw presetError('invalidPresetDocument');
  }
  return {
    schemaVersion: PRESET_DOCUMENT_SCHEMA_VERSION,
    nextPresetId: record.nextPresetId,
    presets,
  };
}

function migrateVersionOne(record: Record<string, unknown>): PresetDocument {
  if (!Array.isArray(record.presets) || record.presets.length > MAX_PRESETS) {
    throw presetError(
      Array.isArray(record.presets) && record.presets.length > MAX_PRESETS
        ? 'tooManyPresets'
        : 'invalidPresetDocument'
    );
  }
  const names = new Set<string>();
  const presets = record.presets.map((value, index): RulePreset => {
    const preset = objectRecord(value);
    if (
      !preset ||
      typeof preset.name !== 'string' ||
      preset.name !== preset.name.trim() ||
      preset.ruleSchemaVersion !== RULE_PIPELINE_SCHEMA_VERSION
    ) {
      throw presetError('invalidPresetDocument');
    }
    const name = normalizePresetName(preset.name);
    if (names.has(name)) {
      throw presetError('invalidPresetDocument');
    }
    names.add(name);
    return {
      presetId: index + 1,
      name,
      ruleSchemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
      rules: parseRules(preset.rules),
    };
  });
  return {
    schemaVersion: PRESET_DOCUMENT_SCHEMA_VERSION,
    nextPresetId: presets.length + 1,
    presets,
  };
}

function parseRules(value: unknown): RuleRequest[] {
  if (!Array.isArray(value) || value.length > MAX_RULES) {
    throw presetError('invalidPresetDocument');
  }
  const rules = value.map(parseRule);
  try {
    compileBrowserRulePipeline(
      { schemaVersion: RULE_PIPELINE_SCHEMA_VERSION, rules, overrides: [] },
      []
    );
  } catch {
    throw presetError('invalidPresetDocument');
  }
  return rules;
}

function parseRule(value: unknown): RuleRequest {
  const rule = objectRecord(value);
  if (
    !rule ||
    typeof rule.kind !== 'string' ||
    !positiveSafeInteger(rule.ruleId) ||
    typeof rule.enabled !== 'boolean'
  ) {
    throw presetError('invalidPresetDocument');
  }
  const base = { ruleId: rule.ruleId, enabled: rule.enabled };
  switch (rule.kind) {
    case 'prefix':
    case 'suffix':
      return { kind: rule.kind, ...base, value: requiredString(rule.value) };
    case 'literalReplace':
      return {
        kind: rule.kind,
        ...base,
        search: requiredString(rule.search),
        replacement: requiredString(rule.replacement),
      };
    case 'regexReplace':
      return {
        kind: rule.kind,
        ...base,
        pattern: requiredString(rule.pattern),
        replacement: requiredString(rule.replacement),
      };
    case 'sequence':
      return {
        kind: rule.kind,
        ...base,
        scope: enumValue(rule.scope, ['allSources', 'perParent']),
        order: enumValue(rule.order, ['sourceOrder', 'nameAscending']),
        start: requiredNumber(rule.start),
        step: requiredNumber(rule.step),
        padding: requiredNumber(rule.padding),
        placement: enumValue(rule.placement, ['prefix', 'suffix']),
        separator: requiredString(rule.separator),
      };
    case 'extension':
      return {
        kind: rule.kind,
        ...base,
        operation: enumValue(rule.operation, ['remove', 'replace']),
        value: requiredString(rule.value),
      };
    case 'case':
      return {
        kind: rule.kind,
        ...base,
        target: filenamePart(rule.target),
        mode: enumValue(rule.mode, ['lowercase', 'uppercase']),
      };
    case 'whitespaceCleanup':
      return {
        kind: rule.kind,
        ...base,
        target: filenamePart(rule.target),
        replacement: requiredString(rule.replacement),
      };
    case 'unicodeNormalization':
      return {
        kind: rule.kind,
        ...base,
        target: filenamePart(rule.target),
        form: enumValue(rule.form, ['nfc', 'nfd', 'nfkc', 'nfkd']),
      };
    case 'range':
      return {
        kind: rule.kind,
        ...base,
        target: filenamePart(rule.target),
        operation: enumValue(rule.operation, ['keep', 'remove']),
        origin: enumValue(rule.origin, ['start', 'end']),
        offset: requiredNumber(rule.offset),
        length: rule.length === null ? null : requiredNumber(rule.length),
      };
    case 'characterClass':
      return {
        kind: rule.kind,
        ...base,
        target: filenamePart(rule.target),
        operation: enumValue(rule.operation, ['keep', 'remove']),
        class: enumValue(rule.class, [
          'decimalNumber',
          'letter',
          'whitespace',
          'punctuation',
          'symbol',
        ]),
      };
    default:
      throw presetError('invalidPresetDocument');
  }
}

function normalizePresetName(name: string): string {
  const normalized = name.trim();
  const byteLength = new TextEncoder().encode(normalized).length;
  if (normalized.length === 0 || byteLength > MAX_PRESET_NAME_BYTES) {
    throw presetError('invalidPresetName');
  }
  return normalized;
}

function ensureDocumentSize(serialized: string): void {
  if (new TextEncoder().encode(serialized).length > MAX_PRESET_DOCUMENT_BYTES) {
    throw presetError('presetDocumentTooLarge');
  }
}

function objectRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function positiveSafeInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
}

function requiredString(value: unknown): string {
  if (typeof value !== 'string') {
    throw presetError('invalidPresetDocument');
  }
  return value;
}

function requiredNumber(value: unknown): number {
  if (typeof value !== 'number') {
    throw presetError('invalidPresetDocument');
  }
  return value;
}

function enumValue<const Value extends string>(value: unknown, values: readonly Value[]): Value {
  if (typeof value !== 'string' || !values.includes(value as Value)) {
    throw presetError('invalidPresetDocument');
  }
  return value as Value;
}

function filenamePart(value: unknown): 'wholeName' | 'stem' | 'extension' {
  return enumValue(value, ['wholeName', 'stem', 'extension']);
}

function presetError(code: PresetErrorCode): PresetError {
  const messages: Record<PresetErrorCode, string> = {
    storageUnavailable: 'Local preset storage is unavailable.',
    unsupportedPresetSchema: 'These presets were created by an unsupported version.',
    invalidPresetDocument: 'Stored presets are invalid and were not loaded.',
    presetDocumentTooLarge: 'Stored presets exceed the local size limit.',
    tooManyPresets: `At most ${MAX_PRESETS} presets can be stored.`,
    invalidPresetName: `Preset names must contain 1 to ${MAX_PRESET_NAME_BYTES} UTF-8 bytes.`,
    duplicatePresetName: 'A preset with that name already exists.',
  };
  return new PresetError(code, messages[code]);
}
