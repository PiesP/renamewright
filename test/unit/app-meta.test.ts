import { describe, expect, it } from 'vitest';
import { APP_NAME, APP_TAGLINE } from '../../src/app-meta';

describe('application identity', () => {
  it('uses the approved public brand', () => {
    expect(APP_NAME).toBe('Renamewright');
    expect(APP_TAGLINE).toBe('Plan every rename.');
  });
});
