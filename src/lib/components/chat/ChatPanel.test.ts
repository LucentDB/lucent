import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, fireEvent, cleanup, waitFor, screen } from '@testing-library/svelte';

if (!HTMLElement.prototype.animate) {
  HTMLElement.prototype.animate = (() => ({
    finished: Promise.resolve(),
    cancel: () => {},
    play: () => {},
    pause: () => {},
  })) as unknown as typeof HTMLElement.prototype.animate;
}

// ChatPanel pulls in the chat store and ChatLanding, both of which reach for
// the Tauri IPC boundary. Stub it so this stays a pure interaction test.
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => []),
  Channel: class {},
}));

vi.mock('../../ipc/ai.ts', () => ({
  listMemories: vi.fn(async () => []),
  saveMemoryManual: vi.fn(async () => {}),
  deleteMemory: vi.fn(async () => true),
  toggleMemoryStatus: vi.fn(async () => {}),
  resolveDrift: vi.fn(async () => {}),
  exportMemoriesMarkdown: vi.fn(async () => '# LUCENT.md'),
  importMemoriesMarkdown: vi.fn(async () => 0),
  listGoldenQueries: vi.fn(async () => []),
  deleteGoldenQuery: vi.fn(async () => true),
  runConsolidation: vi.fn(async () => ({
    archived_memory_count: 0,
    pruned_session_json_count: 0,
  })),
  listChatConversations: vi.fn(async () => []),
  loadChatConversation: vi.fn(async () => []),
}));

import { QueryClient } from '@tanstack/svelte-query';
import ChatPanelHarness from './ChatPanelHarness.svelte';
import { chat, createConversation } from '../../stores/chat.svelte.ts';
import { listChatConversations, loadChatConversation } from '../../ipc/ai.ts';

afterEach(cleanup);

beforeEach(() => {
  chat.conversations = [];
  chat.activeConversationId = null;
  chat.isStreaming = false;
});

function setup(props: { onCloseConv?: (id: string) => void } = {}) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  return render(ChatPanelHarness, {
    client,
    onSend: vi.fn(),
    onRunDml: vi.fn(),
    onCancelDml: vi.fn(),
    ...props,
  });
}

describe('ChatPanel — inert app background behind the Memory Drawer (F-C3)', () => {
  it('inerts the whole app background (not just the panel) while open', async () => {
    const { container, getByTitle, getByPlaceholderText } = setup();
    const trigger = getByTitle('AI Memory & Rules (Cmd+Shift+M)');
    // The chat panel (and everything else the app renders) lives inside the
    // test container, which stands in for `#app`.
    expect(container.hasAttribute('inert')).toBe(false);

    trigger.focus();
    await fireEvent.click(trigger);

    await waitFor(() => expect(container.hasAttribute('inert')).toBe(true));

    // The portaled dialog itself must never be inert.
    const portaled = document.querySelector(
      '[data-memory-drawer-portal]',
    ) as HTMLElement;
    expect(portaled).toBeTruthy();
    expect(portaled.hasAttribute('inert')).toBe(false);

    const search = getByPlaceholderText('Search rules...');
    await waitFor(() => expect(document.activeElement).toBe(search));

    await fireEvent.keyDown(window, { key: 'Escape' });

    await waitFor(() => expect(container.hasAttribute('inert')).toBe(false));
    // Focus lands back on the trigger only once the background is un-inerted.
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });
});

describe('ChatPanel — conversation close control (F-I3)', () => {
  it('keeps the close control in the tab order and closes on Enter', async () => {
    const onCloseConv = vi.fn();
    const conv = createConversation('conn-1');
    chat.conversations = [conv];
    chat.activeConversationId = conv.id;

    const { getAllByRole } = setup({ onCloseConv });
    const close = getAllByRole('button').find((b) =>
      b.getAttribute('aria-label')?.startsWith('Close'),
    ) as HTMLElement | undefined;

    expect(close).toBeTruthy();
    expect(close!.getAttribute('tabindex')).not.toBe('-1');

    await fireEvent.keyDown(close!, { key: 'Enter' });
    expect(onCloseConv).toHaveBeenCalledWith(conv.id);
  });
});

describe('ChatPanel — conversation tab context menu', () => {
  function seedConversations(count: number) {
    const convs = Array.from({ length: count }, (_, i) => {
      const conv = createConversation('conn-1');
      conv.messages = [
        {
          id: `msg-${i}`,
          role: 'user' as const,
          content: `Question ${i}`,
          createdAt: Date.now(),
        },
      ];
      return conv;
    });
    chat.conversations = convs;
    chat.activeConversationId = convs[0].id;
    return convs;
  }

  it('offers the same close actions as the DB tab strip', async () => {
    seedConversations(2);
    const { getByTitle, getByText } = setup();

    await fireEvent.contextMenu(getByTitle('Question 0'));

    expect(getByText('Close Tab')).toBeTruthy();
    expect(getByText('Close Others')).toBeTruthy();
    expect(getByText('Close to the Right')).toBeTruthy();
    expect(getByText('Close to the Left')).toBeTruthy();
    expect(getByText('Close All')).toBeTruthy();
  });

  it('closes only the conversations to the right of the target', async () => {
    const convs = seedConversations(3);
    const { getByTitle, getByText } = setup();

    await fireEvent.contextMenu(getByTitle('Question 1'));
    await fireEvent.click(getByText('Close to the Right'));

    expect(chat.conversations.map((c) => c.id)).toEqual([
      convs[0].id,
      convs[1].id,
    ]);
  });

  it('closes every conversation from Close All', async () => {
    seedConversations(3);
    const { getByTitle, getByText } = setup();

    await fireEvent.contextMenu(getByTitle('Question 2'));
    await fireEvent.click(getByText('Close All'));

    expect(chat.conversations).toEqual([]);
    expect(chat.activeConversationId).toBeNull();
  });

  it('routes Close Tab through the onCloseConv callback', async () => {
    const onCloseConv = vi.fn();
    const convs = seedConversations(2);
    const { getByTitle, getByText } = setup({ onCloseConv });

    await fireEvent.contextMenu(getByTitle('Question 0'));
    await fireEvent.click(getByText('Close Tab'));

    expect(onCloseConv).toHaveBeenCalledWith(convs[0].id);
  });
});

describe('ChatPanel — action buttons (Memory, Previous chats, New chat, Close)', () => {
  it('renders all four action buttons in the panel header', () => {
    const { getByTitle } = setup();

    const memoryBtn = getByTitle('AI Memory & Rules (Cmd+Shift+M)');
    const prevChatsBtn = getByTitle('Previous chats');
    const newChatBtn = getByTitle('New conversation');
    const closeBtn = getByTitle('Close AI panel');

    expect(memoryBtn).toBeTruthy();
    expect(prevChatsBtn).toBeTruthy();
    expect(newChatBtn).toBeTruthy();
    expect(closeBtn).toBeTruthy();

    // Verify memory button contains brain icon paths
    const memorySvg = memoryBtn.querySelector('svg');
    expect(memorySvg).toBeTruthy();
    // Brain SVG contains path with d="M12 5..."
    expect(memorySvg!.innerHTML).toContain('M12 5');
  });

  it('opens the Previous Chats drawer when clicking the previous chats button', async () => {
    const { getByTitle } = setup();
    const prevChatsBtn = getByTitle('Previous chats');

    await fireEvent.click(prevChatsBtn);

    await waitFor(() => {
      const portal = document.querySelector('[data-previous-chats-portal]');
      expect(portal).toBeTruthy();
      expect(document.querySelector('[aria-label="Previous Chats"]')).toBeTruthy();
    });
  });
});

describe('ChatPanel — restoring and rendering complete previous conversation history', () => {
  it('opens a previous conversation from drawer and renders user message, thoughts, tool calls, and assistant response', async () => {
    const mockList = listChatConversations as unknown as ReturnType<typeof vi.fn>;
    const mockLoad = loadChatConversation as unknown as ReturnType<typeof vi.fn>;

    mockList.mockResolvedValue([
      {
        id: 'conv-hist-1',
        connection_id: 'conn-1',
        title: 'Find Active Customers',
        archived: false,
        created_at: 1000,
        updated_at: 1000,
      },
    ]);

    mockLoad.mockResolvedValue([
      {
        id: 'msg-u1',
        conversation_id: 'conv-hist-1',
        role: 'user',
        content: 'Which customers are active?',
        session_json: null,
        created_at: 1000,
      },
      {
        id: 'msg-a1',
        conversation_id: 'conv-hist-1',
        role: 'assistant',
        content: 'Found 24 active customers.',
        session_json: JSON.stringify({
          segments: [
            {
              type: 'thinking',
              content: 'Inspecting customers table and filtering on active=true',
              streaming: false,
              startedAt: 1000,
              durationMs: 2500,
            },
            {
              type: 'tool_call',
              call: {
                id: 'call_1',
                name: 'run_readonly_query',
                args: { sql: 'SELECT * FROM customers WHERE active = true' },
                summary: '24 rows',
                status: 'completed',
              },
            },
          ],
          startedAt: 1000,
          durationMs: 2500,
          active: false,
        }),
        created_at: 1002,
      },
    ]);

    const { container, getByTitle } = setup();

    const prevChatsBtn = getByTitle('Previous chats');
    await fireEvent.click(prevChatsBtn);

    const convItem = await screen.findByText('Find Active Customers');
    expect(convItem).toBeTruthy();
    await fireEvent.click(convItem);

    // Wait for drawer to close and background to un-inert
    await waitFor(() => expect(container.hasAttribute('inert')).toBe(false));

    const userMsg = container.querySelector('.message.user .text');
    expect(userMsg?.textContent).toContain('Which customers are active');
    expect(await screen.findByText(/Found 24 active customers/i)).toBeTruthy();

    // Verify WorkSession header is rendered
    const sessionHeader = await screen.findByText(/Worked for 3s/i);
    expect(sessionHeader).toBeTruthy();

    // Click header to expand work session
    await fireEvent.click(sessionHeader);

    // Verify thinking card header and tool call card details appear
    const thinkingHeader = await screen.findByText(/Thought for 3s/i);
    expect(thinkingHeader).toBeTruthy();
    expect(await screen.findByText(/run readonly query/i)).toBeTruthy();
    expect(await screen.findByText('24 rows')).toBeTruthy();

    // Click thinking card header to reveal thinking content
    await fireEvent.click(thinkingHeader);
    expect(await screen.findByText(/Inspecting customers table/i)).toBeTruthy();
  });
});
