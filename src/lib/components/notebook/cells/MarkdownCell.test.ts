import { describe, it, expect, afterEach, vi } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/svelte';
import MarkdownCell from './MarkdownCell.svelte';

afterEach(cleanup);

describe('MarkdownCell', () => {
  it('renders markdown in preview mode', () => {
    render(MarkdownCell, { props: { source: '# Hello', status: 'ok' } });
    expect(screen.getByText('Hello')).toBeTruthy();
  });

  it('toggles to edit mode on click', async () => {
    const { container } = render(MarkdownCell, {
      props: { source: '# Hello', status: 'ok' },
    });
    await fireEvent.click(container.querySelector('.display')!);
    expect(container.querySelector('textarea')).toBeTruthy();
  });

  it('starts in edit mode when empty and status is pending', () => {
    const { container } = render(MarkdownCell, {
      props: { source: '', status: 'pending' },
    });
    expect(container.querySelector('textarea')).toBeTruthy();
  });

  it('starts in preview mode when non-empty and status is pending', () => {
    const { container } = render(MarkdownCell, {
      props: { source: '# Hello', status: 'pending' },
    });
    expect(container.querySelector('.display')).toBeTruthy();
    expect(container.querySelector('textarea')).toBeNull();
  });

  it('applies bold formatting from toolbar', async () => {
    const onSourceChange = vi.fn();
    const { container } = render(MarkdownCell, {
      props: { source: 'hello', status: 'ok', onSourceChange },
    });
    // Click to enter edit mode
    await fireEvent.click(container.querySelector('.display')!);
    const textarea = container.querySelector('textarea');
    expect(textarea).toBeTruthy();
    textarea!.selectionStart = 0;
    textarea!.selectionEnd = 5;
    await fireEvent.click(container.querySelector('[aria-label="Bold"]')!);
    expect(onSourceChange).toHaveBeenCalledWith('**hello**');
  });

  it('toggles a task checkbox back into the source', async () => {
    const onSourceChange = vi.fn();
    const { container } = render(MarkdownCell, {
      props: { source: '- [ ] todo', status: 'ok', onSourceChange },
    });
    await fireEvent.click(container.querySelector('input[type="checkbox"]')!);
    expect(onSourceChange).toHaveBeenCalledWith('- [x] todo');
  });

  it('renders empty state placeholder', () => {
    render(MarkdownCell, { props: { source: '', status: 'ok' } });
    expect(
      screen.getByText('Empty markdown cell — click to edit'),
    ).toBeTruthy();
  });
});
