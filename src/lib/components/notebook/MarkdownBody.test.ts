import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, cleanup, fireEvent } from '@testing-library/svelte';
import MarkdownBody from './MarkdownBody.svelte';
import { toggleTaskAtIndex } from '../chat/markdown.ts';

afterEach(cleanup);

describe('MarkdownBody', () => {
  it('renders GFM task lists as checkboxes', () => {
    const { container } = render(MarkdownBody, {
      props: { source: '- [ ] unchecked\n- [x] checked' },
    });
    const boxes = container.querySelectorAll('input[type="checkbox"]');
    expect(boxes.length).toBe(2);
    expect((boxes[0] as HTMLInputElement).checked).toBe(false);
    expect((boxes[1] as HTMLInputElement).checked).toBe(true);
  });

  it('renders GFM tables', () => {
    const { container } = render(MarkdownBody, {
      props: { source: '| a | b |\n|---|---|\n| 1 | 2 |' },
    });
    expect(container.querySelector('table')).toBeTruthy();
    expect(container.querySelectorAll('th').length).toBe(2);
  });

  it('renders strikethrough', () => {
    const { container } = render(MarkdownBody, {
      props: { source: '~~gone~~' },
    });
    expect(container.querySelector('del')).toBeTruthy();
  });

  it('strips script tags', () => {
    const { container } = render(MarkdownBody, {
      props: { source: 'hi <script>alert(1)</script>' },
    });
    expect(container.querySelector('script')).toBeNull();
  });

  it('emits a toggle with the task index when a checkbox is clicked', async () => {
    const onToggleTask = vi.fn();
    const { container } = render(MarkdownBody, {
      props: { source: '- [ ] first\n- [ ] second', onToggleTask },
    });
    const boxes = container.querySelectorAll('input[type="checkbox"]');
    await fireEvent.click(boxes[1]);
    expect(onToggleTask).toHaveBeenCalledWith(1, true);
  });
});

describe('toggleTaskAtIndex', () => {
  it('checks the requested task and leaves others alone', () => {
    const src = '- [ ] one\n- [ ] two';
    expect(toggleTaskAtIndex(src, 1, true)).toBe('- [ ] one\n- [x] two');
  });

  it('unchecks a checked task', () => {
    const src = '- [x] one\n- [x] two';
    expect(toggleTaskAtIndex(src, 0, false)).toBe('- [ ] one\n- [x] two');
  });

  it('handles ordered lists and indentation', () => {
    const src = '1. [ ] a\n   - [ ] b';
    expect(toggleTaskAtIndex(src, 1, true)).toBe('1. [ ] a\n   - [x] b');
  });

  it('returns the source unchanged for an out-of-range index', () => {
    const src = '- [ ] only';
    expect(toggleTaskAtIndex(src, 5, true)).toBe(src);
  });

  it('ignores bracket pairs that are not task markers', () => {
    const src = 'see [x] in text\n- [ ] real task';
    expect(toggleTaskAtIndex(src, 0, true)).toBe(
      'see [x] in text\n- [x] real task',
    );
  });
});
