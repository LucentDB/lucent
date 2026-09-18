import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/svelte';
import ChatMessage from './ChatMessage.svelte';
import type { ChatMessage as T } from '../../stores/chat.svelte.ts';
import chatMessageSource from './ChatMessage.svelte?raw';

afterEach(cleanup);

function assistantMsg(overrides: Partial<T> = {}): T {
  return {
    id: 'm1',
    role: 'assistant',
    content: 'hello',
    createdAt: Date.now(),
    ...overrides,
  };
}

describe('ChatMessage memory pill (F-C2)', () => {
  it('renders the singular label and aria when exactly one rule applied', () => {
    render(ChatMessage, {
      message: assistantMsg({ rulesApplied: 1 }),
      onOpenMemoryDrawer: vi.fn(),
    });
    const pill = screen.getByRole('button', {
      name: '1 rule applied. Open Memory Drawer',
    });
    // Svelte/Prettier may reflow the markup, so collapse whitespace before
    // asserting on the visible label.
    const text = (pill.textContent ?? '').replace(/\s+/g, ' ').trim();
    expect(text).toMatch(/1 rule applied/);
    expect(text).not.toMatch(/1 rules applied/);
  });

  it('renders the plural label and aria when several rules applied', () => {
    render(ChatMessage, {
      message: assistantMsg({ rulesApplied: 3 }),
      onOpenMemoryDrawer: vi.fn(),
    });
    const pill = screen.getByRole('button', {
      name: '3 rules applied. Open Memory Drawer',
    });
    const text = (pill.textContent ?? '').replace(/\s+/g, ' ').trim();
    expect(text).toMatch(/3 rules applied/);
  });

  it('clicking memory pill opens attribution popover with rules', async () => {
    render(ChatMessage, {
      message: assistantMsg({
        rulesApplied: 2,
        appliedRuleIds: ['r1', 'r2'],
      }),
    });
    const pill = screen.getByRole('button', { name: /2 rules applied/i });
    await fireEvent.click(pill);
    expect(await screen.findByText(/applied rules/i)).toBeTruthy();
  });

  it('hides the pill when rulesApplied is 0', () => {
    render(ChatMessage, {
      message: assistantMsg({ rulesApplied: 0 }),
      onOpenMemoryDrawer: vi.fn(),
    });
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('hides the pill when rulesApplied is undefined', () => {
    render(ChatMessage, {
      message: assistantMsg(),
      onOpenMemoryDrawer: vi.fn(),
    });
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('renders the pill without a drawer handler and opens the attribution popover', async () => {
    render(ChatMessage, { message: assistantMsg({ rulesApplied: 2 }) });
    const pill = screen.getByRole('button', { name: /2 rules applied/i });
    await fireEvent.click(pill);
    expect(await screen.findByText(/applied rules/i)).toBeTruthy();
  });

  it('does not render the pill for a user message', () => {
    render(ChatMessage, {
      message: assistantMsg({ role: 'user', rulesApplied: 2 }),
      onOpenMemoryDrawer: vi.fn(),
    });
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('opens the Memory Drawer from inside the attribution popover', async () => {
    const onOpen = vi.fn();
    render(ChatMessage, {
      message: assistantMsg({ rulesApplied: 2 }),
      onOpenMemoryDrawer: onOpen,
    });
    await fireEvent.click(screen.getByRole('button'));
    expect(await screen.findByText(/applied rules/i)).toBeTruthy();
    await fireEvent.click(
      screen.getByRole('button', { name: 'Open Memory Drawer' }),
    );
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it('traps focus in the attribution popover and returns it to the pill on Escape', async () => {
    render(ChatMessage, {
      message: assistantMsg({ rulesApplied: 2, appliedRuleIds: ['r1'] }),
    });
    const pill = screen.getByRole('button', { name: /2 rules applied/i });
    await fireEvent.click(pill);

    const dialog = screen.getByRole('dialog', { name: 'Applied rules' });
    expect(dialog.contains(document.activeElement)).toBe(true);

    // Tab from the last focusable wraps to the first (W3C APG modal trap).
    const focusables = dialog.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
    );
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    last.focus();
    await fireEvent.keyDown(window, { key: 'Tab' });
    expect(document.activeElement).toBe(first);

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(pill);
  });

  it('still shows token usage with no pill when zero rules applied', () => {
    render(ChatMessage, {
      message: assistantMsg({
        rulesApplied: 0,
        usage: {
          promptTokens: 90,
          completionTokens: 10,
          cachedPromptTokens: 0,
        },
      }),
      onOpenMemoryDrawer: vi.fn(),
    });
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.getByText('~100 tokens')).toBeTruthy();
  });
});

describe('ChatMessage — reduced motion (F-I8)', () => {
  it('disables the message-in animation under prefers-reduced-motion', () => {
    expect(chatMessageSource).toMatch(
      /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.message\s*\{[^}]*animation:\s*none/,
    );
  });
});
