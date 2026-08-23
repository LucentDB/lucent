import { describe, it, expect } from 'vitest';
import { argsMissing } from './tool-args.ts';

describe('argsMissing', () => {
  it('treats null and undefined as absent', () => {
    expect(argsMissing(null)).toBe(true);
    expect(argsMissing(undefined)).toBe(true);
  });

  // The whole reason this predicate exists: an ACP agent announces a call with
  // `raw_input: {}` before the real arguments are known, and `{}` carries no
  // more information than `null` did.
  it('treats an empty object as absent', () => {
    expect(argsMissing({})).toBe(true);
  });

  it('treats an empty array and a blank string as absent', () => {
    expect(argsMissing([])).toBe(true);
    expect(argsMissing('')).toBe(true);
    expect(argsMissing('   ')).toBe(true);
  });

  it('treats real arguments as present', () => {
    expect(argsMissing({ sql: 'select 1' })).toBe(false);
    expect(argsMissing(['a'])).toBe(false);
    expect(argsMissing('select 1')).toBe(false);
  });

  // `0` and `false` are values the agent chose to send, not the absence of one.
  it('treats falsy scalars as present', () => {
    expect(argsMissing(0)).toBe(false);
    expect(argsMissing(false)).toBe(false);
  });
});
