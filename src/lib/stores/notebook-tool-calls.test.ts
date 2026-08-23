import { describe, it, expect } from 'vitest';
import { mergeToolResult } from './notebook-tool-calls.ts';

const call = (over: Record<string, unknown> = {}) => ({
  id: 'call_1',
  name: 'Bash',
  args: {},
  ...over,
});

describe('mergeToolResult', () => {
  it('backfills arguments over the empty object the call announced', () => {
    const merged = mergeToolResult([call()], {
      id: 'call_1',
      tool: 'run_readonly_query',
      summary: '10 rows',
      input: { sql: 'select 1' },
      output: undefined,
    });
    expect(merged[0].args).toEqual({ sql: 'select 1' });
  });

  it('backfills arguments over a null the call announced', () => {
    const merged = mergeToolResult([call({ args: null })], {
      id: 'call_1',
      tool: 'run_readonly_query',
      summary: '10 rows',
      input: { sql: 'select 1' },
    });
    expect(merged[0].args).toEqual({ sql: 'select 1' });
  });

  it('never overwrites arguments the call really reported', () => {
    const merged = mergeToolResult([call({ args: { sql: 'announced' } })], {
      id: 'call_1',
      tool: 'run_readonly_query',
      summary: '10 rows',
      input: { sql: 'from the bridge' },
    });
    expect(merged[0].args).toEqual({ sql: 'announced' });
  });

  it('leaves blank arguments blank when the result reports none either', () => {
    const merged = mergeToolResult([call()], {
      id: 'call_1',
      tool: 'run_readonly_query',
      summary: '10 rows',
    });
    expect(merged[0].args).toEqual({});
  });

  it('replaces a free-text title with the real tool id', () => {
    const merged = mergeToolResult([call({ name: 'psql -c "select 1"' })], {
      id: 'call_1',
      tool: 'run_readonly_query',
      summary: '10 rows',
    });
    expect(merged[0].name).toBe('run_readonly_query');
  });

  it('keeps a name that is already a tool id', () => {
    const merged = mergeToolResult([call({ name: 'get_objects_info' })], {
      id: 'call_1',
      tool: 'run_readonly_query',
      summary: 'ok',
    });
    expect(merged[0].name).toBe('get_objects_info');
  });

  it('records the summary and output on the matching call only', () => {
    const merged = mergeToolResult([call(), call({ id: 'call_2' })], {
      id: 'call_1',
      tool: 'run_readonly_query',
      summary: '10 rows',
      output: { type: 'text', data: 'ok' },
    });
    expect(merged[0].summary).toBe('10 rows');
    expect(merged[0].output).toEqual({ type: 'text', data: 'ok' });
    expect(merged[1].summary).toBeUndefined();
  });

  it('returns a new array and never mutates the calls it was given', () => {
    const original = call();
    const list = [original];
    const merged = mergeToolResult(list, {
      id: 'call_1',
      tool: 'run_readonly_query',
      summary: '10 rows',
      input: { sql: 'select 1' },
    });
    expect(merged).not.toBe(list);
    expect(merged[0]).not.toBe(original);
    expect(original.args).toEqual({});
  });

  it('leaves the list untouched when no call matches', () => {
    const merged = mergeToolResult([call()], {
      id: 'nope',
      tool: 'run_readonly_query',
      summary: '10 rows',
      input: { sql: 'select 1' },
    });
    expect(merged[0].args).toEqual({});
    expect(merged[0].summary).toBeUndefined();
  });
});
