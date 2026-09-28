// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/svelte';
import CodeBlock from './CodeBlock.svelte';

afterEach(cleanup);

describe('CodeBlock', () => {
  it('renders copy button with type="button" and accessible aria-label', () => {
    render(CodeBlock, { code: 'SELECT 1;' });
    const btn = screen.getByRole('button', {
      name: 'Copy code block to clipboard',
    });
    expect(btn).toBeTruthy();
    expect(btn.getAttribute('type')).toBe('button');
  });
});
