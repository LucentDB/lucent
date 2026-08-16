import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/svelte';
import Notebook from './Notebook.svelte';
import { notebooks } from '../../stores/notebooks.svelte.ts';

vi.mock('../../ipc/notebook', () => ({
  notebookAttach: vi.fn(async () => 'session-key-1'),
  notebookDetach: vi.fn(async () => undefined),
  notebookClearOutputs: vi.fn(async () => undefined),
}));

const SPEC = {
  filePath: null,
  connectionId: 'profile-1',
  database: 'postgres',
};

afterEach(async () => {
  cleanup();
  for (const id of ['tab-a', 'tab-b']) {
    if (notebooks.has(id)) await notebooks.release(id);
  }
});

describe('Notebook', () => {
  it('mounts without throwing and registers its tab in the registry', () => {
    // Regression: the model lookup used to live in a `$derived`, and
    // `notebooks.ensure()` mutates the registry's reactive map — a state
    // mutation during `$derived` evaluation, which Svelte forbids
    // (state_unsafe_mutation). ensure() must be called from the component
    // initializer / an effect, never from a derived.
    expect(() => render(Notebook, { tabId: 'tab-a', ...SPEC })).not.toThrow();
    expect(notebooks.get('tab-a')).toBeDefined();
    expect(screen.getByText('Run All')).toBeTruthy();
  });

  it('focuses the notebook surface in command mode', async () => {
    const { container } = render(Notebook, { tabId: 'tab-a', ...SPEC });
    await Promise.resolve();
    expect(document.activeElement).toBe(container.querySelector('.notebook'));
  });

  it('re-points to the new tab model when tabId changes', async () => {
    const { rerender } = render(Notebook, { tabId: 'tab-a', ...SPEC });
    const first = notebooks.get('tab-a');

    await rerender({ tabId: 'tab-b', ...SPEC });
    expect(notebooks.get('tab-b')).toBeDefined();
    expect(screen.getByText('Run All')).toBeTruthy();

    // Switch back: the component must re-point at tab-a's existing model.
    await rerender({ tabId: 'tab-a', ...SPEC });
    expect(notebooks.get('tab-a')).toBe(first);
  });
});
