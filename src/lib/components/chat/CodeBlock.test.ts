import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/svelte';
import CodeBlock from './CodeBlock.svelte';

afterEach(cleanup);

describe('CodeBlock', () => {
  beforeEach(() => {
    Object.assign(navigator, {
      clipboard: {
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });
  });

  it('renders code and language header correctly', () => {
    render(CodeBlock, { code: 'SELECT * FROM users;', lang: 'sql' });

    expect(screen.getByText('sql')).toBeTruthy();
    expect(screen.getByText('SELECT * FROM users;')).toBeTruthy();
  });

  it('has accessible copy button with type="button" and initial aria-label', () => {
    render(CodeBlock, { code: 'console.log("hello")', lang: 'js' });

    const button = screen.getByRole('button', {
      name: 'Copy code to clipboard',
    });
    expect(button.getAttribute('type')).toBe('button');
    expect(button.textContent).toBe('Copy');
  });

  it('updates aria-label and label text when copy is clicked', async () => {
    render(CodeBlock, { code: 'const x = 42;', lang: 'ts' });

    const button = screen.getByRole('button', {
      name: 'Copy code to clipboard',
    });
    await fireEvent.click(button);

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('const x = 42;');
    expect(
      screen.getByRole('button', { name: 'Code copied to clipboard' }),
    ).toBeTruthy();
    expect(button.textContent).toBe('Copied!');
  });
});
