import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/svelte';
import ChatMessage from './ChatMessage.svelte';
import type { ChatMessage as T } from '../../stores/chat.svelte.ts';
import chatMessageSource from './ChatMessage.svelte?raw';

if (!HTMLElement.prototype.animate) {
  HTMLElement.prototype.animate = (() => ({
    finished: Promise.resolve(),
    cancel: () => {},
    play: () => {},
    pause: () => {},
  })) as unknown as typeof HTMLElement.prototype.animate;
}

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

describe('ChatMessage — turn anchoring', () => {
  it('carries its message id so the panel can pin the sent message', () => {
    const { container } = render(ChatMessage, {
      message: assistantMsg({ id: 'msg-42', role: 'user' }),
    });
    const root = container.querySelector('[data-message-id="msg-42"]');
    expect(root).toBeTruthy();
    expect(root?.classList.contains('message')).toBe(true);
  });
});

describe('ChatMessage — reduced motion (F-I8)', () => {
  it('disables the message-in animation under prefers-reduced-motion', () => {
    expect(chatMessageSource).toMatch(
      /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.message\s*\{[^}]*animation:\s*none/,
    );
  });
});

describe('ChatMessage — work sessions (thoughts and tool calls)', () => {
  it('renders thinking session and displays thinking content when expanded', async () => {
    render(ChatMessage, {
      message: assistantMsg({
        content: 'Answer after thinking',
        session: {
          segments: [
            {
              type: 'thinking',
              content: 'Considering database schema and foreign keys...',
              streaming: false,
              startedAt: 1000,
              durationMs: 2500,
            },
          ],
          startedAt: 1000,
          durationMs: 2500,
          active: false,
          expanded: true,
        },
      }),
    });

    const thoughtButtons = screen.getAllByRole('button', { name: /Thought for 3s/i });
    expect(thoughtButtons.length).toBe(2);

    // Clicking the ThinkingCard header expands its thoughts
    await fireEvent.click(thoughtButtons[1]);
    expect(screen.getByText(/Considering database schema and foreign keys.../i)).toBeTruthy();
    expect(screen.getByText('Answer after thinking')).toBeTruthy();
  });

  it('renders tool calls with summary and tool status in work session', async () => {
    render(ChatMessage, {
      message: assistantMsg({
        content: 'Found 3 users in the database',
        session: {
          segments: [
            {
              type: 'tool_call',
              call: {
                id: 'call-1',
                name: 'run_readonly_query',
                args: { sql: 'SELECT * FROM users LIMIT 3' },
                summary: '3 rows',
                status: 'completed',
              },
            },
          ],
          startedAt: 1000,
          durationMs: 1200,
          active: false,
          expanded: true,
        },
      }),
    });

    expect(screen.getByText(/Worked for 1s/i)).toBeTruthy();
    expect(screen.getByText(/run readonly query/i)).toBeTruthy();
    expect(screen.getByText('3 rows')).toBeTruthy();
    expect(screen.getByText('Found 3 users in the database')).toBeTruthy();
  });

  it('toggles expansion when the session header is clicked', async () => {
    render(ChatMessage, {
      message: assistantMsg({
        content: 'Final response',
        session: {
          segments: [
            {
              type: 'thinking',
              content: 'Secret thoughts...',
              streaming: false,
              startedAt: 1000,
            },
          ],
          startedAt: 1000,
          durationMs: 1000,
          active: false,
          expanded: false,
        },
      }),
    });

    expect(screen.getByText(/Thought for 1s/i)).toBeTruthy();
    expect(screen.queryByText('Secret thoughts...')).toBeNull();

    const headerBtn = screen.getByRole('button', { name: /Thought for 1s/i });
    await fireEvent.click(headerBtn);
    expect(await screen.findByText('Secret thoughts...')).toBeTruthy();
  });
});
