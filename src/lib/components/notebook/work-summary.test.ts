import { describe, it, expect } from 'vitest';
import { workSummary } from './work-summary.ts';

// Built through helpers rather than inline literals: a fresh object literal
// passed straight to workSummary trips TypeScript's excess-property check on
// the deliberately minimal ToolCallLike.
const tool = (name: string, id = 'a') => ({ id, name, args: {} });
const q = (id = 'a') => tool('run_readonly_query', id);
const thought = (durationMs?: number) => ({ thinking: 'hmm', durationMs });

describe('workSummary', () => {
  it('names what a query tool did rather than counting calls', () => {
    expect(workSummary([q()], [])).toBe('Ran 1 query');
  });

  it('pluralises repeated calls to the same tool', () => {
    expect(workSummary([q('a'), q('b'), q('c')], [])).toBe('Ran 3 queries');
  });

  it('names each of Lucent’s own tools', () => {
    expect(workSummary([tool('get_objects_info')], [])).toBe(
      'Inspected schema',
    );
    expect(workSummary([tool('search_objects')], [])).toBe('Searched schema');
    expect(workSummary([tool('preview_dml')], [])).toBe('Previewed 1 change');
  });

  it('lists distinct tools in the order they were first called', () => {
    expect(workSummary([tool('search_objects'), q('b'), q('c')], [])).toBe(
      'Searched schema · ran 2 queries',
    );
  });

  // An ACP agent's tool "name" is free text — often the whole shell command.
  // It gets shown as-is rather than dressed up as a verb phrase.
  it('falls back to the agent’s own title for tools it does not know', () => {
    expect(workSummary([tool('Bash')], [])).toBe('Bash');
    expect(workSummary([tool('Bash', 'a'), tool('Bash', 'b')], [])).toBe(
      'Bash ×2',
    );
  });

  it('prettifies an unknown snake_case tool id', () => {
    expect(workSummary([tool('list_indexes')], [])).toBe('List indexes');
  });

  it('adds the total time spent thinking', () => {
    expect(workSummary([q()], [thought(20_000), thought(5_000)])).toBe(
      'Ran 1 query · thought for 25s',
    );
  });

  it('reports thinking on its own when no tool was called', () => {
    expect(workSummary([], [thought(25_000)])).toBe('Thought for 25s');
  });

  // A segment still streaming has no duration yet; it must not read as 0s.
  it('ignores segments that have not closed yet', () => {
    expect(workSummary([], [thought(4_000), thought(undefined)])).toBe(
      'Thought for 4s',
    );
    expect(workSummary([], [thought(undefined)])).toBe('');
  });

  it('rounds sub-second thinking up to a second, as the cards do', () => {
    expect(workSummary([], [thought(120)])).toBe('Thought for 1s');
  });

  it('is empty when the model did nothing worth reporting', () => {
    expect(workSummary([], [])).toBe('');
  });

  it('ignores messages that are not thinking segments', () => {
    expect(workSummary([q()], [{ text: 'hello' }])).toBe('Ran 1 query');
  });
});
