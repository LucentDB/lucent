import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/svelte';
import ThinkingCard from './ThinkingCard.svelte';

afterEach(cleanup);

describe('ThinkingCard', () => {
  it('renders "Thinking…" when durationMs is undefined and expands body by default', () => {
    render(ThinkingCard, { content: 'Analyzing schema...' });
    expect(screen.getByText('Thinking…')).toBeTruthy();
    expect(screen.getByText('Analyzing schema...')).toBeTruthy();
    const button = screen.getByRole('button');
    expect(button.getAttribute('aria-expanded')).toBe('true');
  });

  it('renders duration when durationMs is set and collapses body by default', () => {
    render(ThinkingCard, {
      content: 'Query plan considered.',
      durationMs: 2400,
    });
    expect(screen.getByText('Thought for 2s')).toBeTruthy();
    expect(screen.queryByText('Query plan considered.')).toBeNull();
    const button = screen.getByRole('button');
    expect(button.getAttribute('aria-expanded')).toBe('false');
  });

  it('toggles expansion state and aria-expanded on click', async () => {
    render(ThinkingCard, {
      content: 'Deep thought process.',
      durationMs: 1500,
    });
    const button = screen.getByRole('button');
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByText('Deep thought process.')).toBeNull();

    await fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByText('Deep thought process.')).toBeTruthy();

    await fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('false');
  });
});
