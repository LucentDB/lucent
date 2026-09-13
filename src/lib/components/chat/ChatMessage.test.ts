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
    expect(pill.textContent).toMatch(/1 rule applied/);
    expect(pill.textContent).not.toMatch(/1 rules applied/);
  });

  it('renders the plural label and aria when several rules applied', () => {
    render(ChatMessage, {
      message: assistantMsg({ rulesApplied: 3 }),
      onOpenMemoryDrawer: vi.fn(),
    });
    const pill = screen.getByRole('button', {
      name: '3 rules applied. Open Memory Drawer',
    });
    expect(pill.textContent).toMatch(/3 rules applied/);
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

  it('hides the pill when no drawer handler is provided', () => {
    render(ChatMessage, { message: assistantMsg({ rulesApplied: 2 }) });
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('does not render the pill for a user message', () => {
    render(ChatMessage, {
      message: assistantMsg({ role: 'user', rulesApplied: 2 }),
      onOpenMemoryDrawer: vi.fn(),
    });
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('opens the Memory Drawer when the pill is clicked', async () => {
    const onOpen = vi.fn();
    render(ChatMessage, {
      message: assistantMsg({ rulesApplied: 2 }),
      onOpenMemoryDrawer: onOpen,
    });
    await fireEvent.click(screen.getByRole('button'));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it('still shows token usage with no pill when zero rules applied', () => {
    render(ChatMessage, {
      message: assistantMsg({
        rulesApplied: 0,
        usage: { promptTokens: 90, completionTokens: 10, cachedPromptTokens: 0 },
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
