import { describe, it, expect } from 'vitest';
import { fieldInputType, needsFilePicker } from './ConnectionForm.svelte';

describe('driver field rendering', () => {
  it('maps field kinds onto input types', () => {
    expect(fieldInputType({ kind: 'text' })).toBe('text');
    expect(fieldInputType({ kind: 'number' })).toBe('number');
    expect(fieldInputType({ kind: 'password' })).toBe('password');
    // A path is a text input plus a Browse button, not a native file input —
    // Tauri's dialog returns a real filesystem path, which <input type="file">
    // deliberately hides.
    expect(fieldInputType({ kind: 'path' })).toBe('text');
  });

  it('only path fields get a browse button', () => {
    expect(needsFilePicker({ kind: 'path' })).toBe(true);
    expect(needsFilePicker({ kind: 'text' })).toBe(false);
    expect(needsFilePicker({ kind: 'select' })).toBe(false);
  });
});
