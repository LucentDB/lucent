import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, fireEvent, cleanup, waitFor } from '@testing-library/svelte';

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
