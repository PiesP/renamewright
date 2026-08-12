import { describe, expect, test } from 'vitest';
import {
  LOCALE_STORAGE_KEY,
  type LocaleStorage,
  localizedError,
  message,
  persistLocale,
  resolveLocale,
} from '../../src/i18n/catalog';
import { PlanningError } from '../../src/planning/rules';

function storage(initial?: string): LocaleStorage & { value: string | null } {
  return {
    value: initial ?? null,
    getItem(key) {
      expect(key).toBe(LOCALE_STORAGE_KEY);
      return this.value;
    },
    setItem(key, value) {
      expect(key).toBe(LOCALE_STORAGE_KEY);
      this.value = value;
    },
  };
}

describe('bilingual message catalog', () => {
  test('resolves a stored locale before the environment preference', () => {
    expect(resolveLocale(storage('en'), ['ko-KR'])).toBe('en');
    expect(resolveLocale(storage('ko'), ['en-US'])).toBe('ko');
  });

  test('uses Korean environment preferences and otherwise falls back to English', () => {
    expect(resolveLocale(storage(), ['ja-JP', 'ko-KR'])).toBe('ko');
    expect(resolveLocale(storage(), ['ja-JP'])).toBe('en');
    expect(resolveLocale(undefined, [])).toBe('en');
  });

  test('persists only the locale code and fails closed when storage is unavailable', () => {
    const available = storage();
    expect(persistLocale(available, 'ko')).toBe(true);
    expect(available.value).toBe('ko');
    expect(
      persistLocale(
        {
          getItem: () => null,
          setItem: () => {
            throw new Error('private storage detail');
          },
        },
        'en'
      )
    ).toBe(false);
  });

  test('interpolates both catalogs and localizes typed error codes', () => {
    expect(message('en', 'activeRules', { count: 3 })).toBe('3 active');
    expect(message('ko', 'activeRules', { count: 3 })).toBe('3개 활성');
    const error = new PlanningError('invalidRegex', 'internal English detail', 7);
    expect(localizedError('en', error, 'errorPlanningFailed')).toBe(
      'Rule 7 uses an invalid regular expression.'
    );
    expect(localizedError('ko', error, 'errorPlanningFailed')).toBe(
      '규칙 7의 정규식이 올바르지 않습니다.'
    );
  });

  test('does not reflect unknown error details into the interface', () => {
    expect(
      localizedError('ko', new Error('/home/private/native-path.txt'), 'errorPlanOpenFailed')
    ).toBe('계획 문서를 열 수 없습니다.');
  });
});
