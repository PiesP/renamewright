import { expect, test } from 'vitest';
import {
  addPreset,
  deletePreset,
  emptyPresetDocument,
  MAX_PRESET_DOCUMENT_BYTES,
  MAX_PRESETS,
  PRESET_DOCUMENT_SCHEMA_VERSION,
  PRESET_STORAGE_KEY,
  PresetError,
  type PresetStorage,
  readPresetDocument,
  writePresetDocument,
} from '../../src/planning/presets';
import { RULE_PIPELINE_SCHEMA_VERSION, type RuleRequest } from '../../src/planning/rules';

class MemoryStorage implements PresetStorage {
  value: string | null = null;
  writes = 0;

  getItem(key: string): string | null {
    expect(key).toBe(PRESET_STORAGE_KEY);
    return this.value;
  }

  setItem(key: string, value: string): void {
    expect(key).toBe(PRESET_STORAGE_KEY);
    this.value = value;
    this.writes += 1;
  }
}

const prefixRules = (value = 'Final-'): RuleRequest[] => [
  { kind: 'prefix', ruleId: 7, enabled: true, value },
];

function expectPresetError(operation: () => unknown, code: string): void {
  try {
    operation();
    throw new Error(`Expected preset error ${code}.`);
  } catch (cause) {
    expect(cause).toMatchObject({ code });
  }
}

test('stores bounded rule-only presets with stable local IDs', () => {
  const storage = new MemoryStorage();
  const first = addPreset(emptyPresetDocument(), '  Reports  ', prefixRules());
  const second = addPreset(first, 'Images', [
    {
      kind: 'characterClass',
      ruleId: 11,
      enabled: true,
      target: 'stem',
      operation: 'remove',
      class: 'whitespace',
    },
  ]);

  writePresetDocument(storage, second);
  const loaded = readPresetDocument(storage);

  expect(loaded.migrated).toBe(false);
  expect(loaded.document).toEqual({
    schemaVersion: PRESET_DOCUMENT_SCHEMA_VERSION,
    nextPresetId: 3,
    presets: [
      {
        presetId: 1,
        name: 'Reports',
        ruleSchemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
        rules: prefixRules(),
      },
      expect.objectContaining({ presetId: 2, name: 'Images' }),
    ],
  });
  expect(JSON.stringify(loaded.document)).not.toContain('sourceId');
  expect(deletePreset(loaded.document, 1).presets.map(({ presetId }) => presetId)).toEqual([2]);
});

test('migrates schema one deterministically and persists schema two once', () => {
  const storage = new MemoryStorage();
  storage.value = JSON.stringify({
    schemaVersion: 1,
    presets: [
      { name: 'First', ruleSchemaVersion: RULE_PIPELINE_SCHEMA_VERSION, rules: prefixRules('A-') },
      { name: 'Second', ruleSchemaVersion: RULE_PIPELINE_SCHEMA_VERSION, rules: prefixRules('B-') },
    ],
  });

  const migrated = readPresetDocument(storage);
  const reloaded = readPresetDocument(storage);

  expect(migrated.migrated).toBe(true);
  expect(migrated.document.presets.map(({ presetId }) => presetId)).toEqual([1, 2]);
  expect(migrated.document.nextPresetId).toBe(3);
  expect(reloaded).toEqual({ document: migrated.document, migrated: false });
  expect(storage.writes).toBe(1);
});

test('rejects malformed, unsupported, or invalid-rule documents without rewriting them', () => {
  const fixtures: Array<[unknown, string]> = [
    [{ schemaVersion: 99, presets: [] }, 'unsupportedPresetSchema'],
    [{ schemaVersion: 2, nextPresetId: 1, presets: 'private-value' }, 'invalidPresetDocument'],
    [
      {
        schemaVersion: 2,
        nextPresetId: 2,
        presets: [
          {
            presetId: 1,
            name: 'Invalid regex',
            ruleSchemaVersion: RULE_PIPELINE_SCHEMA_VERSION,
            rules: [
              { kind: 'regexReplace', ruleId: 1, enabled: true, pattern: '(', replacement: '' },
            ],
          },
        ],
      },
      'invalidPresetDocument',
    ],
  ];

  for (const [value, code] of fixtures) {
    const storage = new MemoryStorage();
    storage.value = JSON.stringify(value);
    expect(() => readPresetDocument(storage)).toThrowError(PresetError);
    try {
      readPresetDocument(storage);
    } catch (cause) {
      expect(cause).toMatchObject({ code });
      expect((cause as Error).message).not.toContain('private-value');
    }
    expect(storage.writes).toBe(0);
  }
});

test('enforces name, count, serialized-size, and storage availability limits', () => {
  const first = addPreset(emptyPresetDocument(), 'Unique', prefixRules());
  expectPresetError(() => addPreset(first, 'Unique', prefixRules()), 'duplicatePresetName');
  expectPresetError(() => addPreset(first, ' ', prefixRules()), 'invalidPresetName');

  let full = emptyPresetDocument();
  for (let index = 0; index < MAX_PRESETS; index += 1) {
    full = addPreset(full, `Preset ${index}`, prefixRules());
  }
  expectPresetError(() => addPreset(full, 'Overflow', prefixRules()), 'tooManyPresets');

  const oversized = new MemoryStorage();
  oversized.value = ' '.repeat(MAX_PRESET_DOCUMENT_BYTES + 1);
  expectPresetError(() => readPresetDocument(oversized), 'presetDocumentTooLarge');

  const unavailable: PresetStorage = {
    getItem: () => {
      throw new Error('private storage detail');
    },
    setItem: () => {
      throw new Error('private storage detail');
    },
  };
  expectPresetError(() => readPresetDocument(unavailable), 'storageUnavailable');
  expectPresetError(
    () => writePresetDocument(unavailable, emptyPresetDocument()),
    'storageUnavailable'
  );
});
