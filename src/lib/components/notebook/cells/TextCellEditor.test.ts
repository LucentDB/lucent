import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import TextCellEditor from './TextCellEditor.svelte';

afterEach(cleanup);

describe('TextCellEditor focus', () => {
  it('focuses the textarea when edit mode is entered', async () => {
    const { container, rerender } = render(TextCellEditor, {
      props: { source: 'notes', editing: false },
    });

    await rerender({ source: 'notes', editing: true });
    await Promise.resolve();

    expect(document.activeElement).toBe(container.querySelector('textarea'));
  });
});
