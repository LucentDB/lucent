import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, fireEvent, cleanup, waitFor } from '@testing-library/svelte';
import PreviousChatsDrawer from './PreviousChatsDrawer.svelte';
import { chat, createConversation } from '../../stores/chat.svelte.ts';

const mockListChatConversations = vi.fn();
const mockLoadChatConversation = vi.fn();
const mockDeleteChatConversation = vi.fn();

vi.mock('../../ipc/ai.ts', () => ({
  listChatConversations: (...args: unknown[]) => mockListChatConversations(...args),
  loadChatConversation: (...args: unknown[]) => mockLoadChatConversation(...args),
  deleteChatConversation: (...args: unknown[]) => mockDeleteChatConversation(...args),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => []),
}));

describe('PreviousChatsDrawer', () => {
  const sampleConvs = [
    {
      id: 'c1',
      connection_id: 'conn1',
      title: 'Sales Analysis Query',
      archived: false,
      created_at: 1700000000,
      updated_at: 1700000500,
    },
    {
      id: 'c2',
      connection_id: 'conn1',
      title: 'Customer Churn Model',
      archived: false,
      created_at: 1700001000,
      updated_at: 1700001500,
    },
  ];

  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
    mockListChatConversations.mockReset();
    mockLoadChatConversation.mockReset();
    mockDeleteChatConversation.mockReset();

    mockListChatConversations.mockResolvedValue(sampleConvs);
    mockLoadChatConversation.mockResolvedValue([
      { id: 'm1', role: 'user', content: 'hello', created_at: 1700000000 },
    ]);
    mockDeleteChatConversation.mockResolvedValue(true);
  });

  afterEach(cleanup);

  it('renders previous chats when isOpen is true', async () => {
    const { getByText, getByPlaceholderText } = render(PreviousChatsDrawer, {
      isOpen: true,
    });

    await waitFor(() => {
      expect(getByText('Previous Chats')).toBeTruthy();
      expect(getByText('Sales Analysis Query')).toBeTruthy();
      expect(getByText('Customer Churn Model')).toBeTruthy();
    });

    expect(getByPlaceholderText('Search previous chats...')).toBeTruthy();
  });

  it('shows Open badge when conversation is currently in open tabs', async () => {
    const conv1 = createConversation('conn1');
    conv1.id = 'c1';
    chat.conversations = [conv1];

    const { getByText, queryAllByText } = render(PreviousChatsDrawer, {
      isOpen: true,
    });

    await waitFor(() => {
      expect(getByText('Sales Analysis Query')).toBeTruthy();
    });

    const openBadges = queryAllByText('Open');
    expect(openBadges.length).toBe(1);
  });

  it('filters conversations by search query', async () => {
    const { getByPlaceholderText, queryByText } = render(PreviousChatsDrawer, {
      isOpen: true,
    });

    await waitFor(() => {
      expect(queryByText('Sales Analysis Query')).toBeTruthy();
      expect(queryByText('Customer Churn Model')).toBeTruthy();
    });

    const searchInput = getByPlaceholderText('Search previous chats...');
    await fireEvent.input(searchInput, { target: { value: 'Churn' } });

    await waitFor(() => {
      expect(queryByText('Sales Analysis Query')).toBeNull();
      expect(queryByText('Customer Churn Model')).toBeTruthy();
    });
  });

  it('two-step delete confirmation calls deleteChatConversation', async () => {
    const { getAllByLabelText } = render(PreviousChatsDrawer, {
      isOpen: true,
    });

    await waitFor(() => {
      expect(getAllByLabelText('Delete chat').length).toBe(2);
    });

    const firstDeleteBtn = getAllByLabelText('Delete chat')[0];
    // First click arms the button with "Confirm?"
    await fireEvent.click(firstDeleteBtn);

    const confirmBtn = getAllByLabelText('Confirm delete chat')[0];
    expect(confirmBtn).toBeTruthy();
    expect(confirmBtn.textContent).toContain('Confirm?');

    // Second click executes delete
    await fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(mockDeleteChatConversation).toHaveBeenCalledWith('c1');
    });
  });

  it('selecting a conversation calls onSelectConv and closes drawer', async () => {
    const onSelectConv = vi.fn();
    const onClose = vi.fn();

    const { getByText } = render(PreviousChatsDrawer, {
      isOpen: true,
      onSelectConv,
      onClose,
    });

    await waitFor(() => {
      expect(getByText('Customer Churn Model')).toBeTruthy();
    });

    await fireEvent.click(getByText('Customer Churn Model'));

    expect(onSelectConv).toHaveBeenCalledWith('c2');
    expect(onClose).toHaveBeenCalled();
  });

  it('closes on close button click and on Escape key', async () => {
    const onClose = vi.fn();
    const { getByLabelText } = render(PreviousChatsDrawer, {
      isOpen: true,
      onClose,
    });

    const closeBtn = getByLabelText('Close drawer');
    await fireEvent.click(closeBtn);
    expect(onClose).toHaveBeenCalled();

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('handles keyboard navigation on delete button without selecting conversation', async () => {
    const onSelectConv = vi.fn();
    const { getAllByLabelText } = render(PreviousChatsDrawer, {
      isOpen: true,
      onSelectConv,
    });

    await waitFor(() => {
      expect(getAllByLabelText('Delete chat').length).toBe(2);
    });

    const firstDeleteBtn = getAllByLabelText('Delete chat')[0];
    await fireEvent.keyDown(firstDeleteBtn, { key: 'Enter' });

    const confirmBtn = getAllByLabelText('Confirm delete chat')[0];
    expect(confirmBtn).toBeTruthy();
    expect(onSelectConv).not.toHaveBeenCalled();

    await fireEvent.keyDown(confirmBtn, { key: 'Enter' });
    await waitFor(() => {
      expect(mockDeleteChatConversation).toHaveBeenCalledWith('c1');
    });
    expect(onSelectConv).not.toHaveBeenCalled();
  });

  it('selecting a conversation opens the conversation tabs with full message session history', async () => {
    mockLoadChatConversation.mockResolvedValue([
      {
        id: 'm1',
        conversation_id: 'c2',
        role: 'user',
        content: 'Show me all customers',
        session_json: null,
        created_at: 1700001000,
      },
      {
        id: 'm2',
        conversation_id: 'c2',
        role: 'assistant',
        content: 'Here are the customer rows',
        session_json: JSON.stringify({
          segments: [
            {
              type: 'thinking',
              content: 'Analyzing customer table',
              streaming: false,
              startedAt: 1000,
            },
            {
              type: 'tool_call',
              call: {
                id: 'call-1',
                name: 'run_readonly_query',
                args: { sql: 'SELECT * FROM customers' },
                summary: '5 rows',
                status: 'completed',
              },
            },
          ],
          startedAt: 1000,
          durationMs: 1500,
          active: false,
        }),
        created_at: 1700001005,
      },
    ]);

    const { getByText } = render(PreviousChatsDrawer, {
      isOpen: true,
    });

    await waitFor(() => {
      expect(getByText('Customer Churn Model')).toBeTruthy();
    });

    await fireEvent.click(getByText('Customer Churn Model'));

    await waitFor(() => {
      expect(chat.activeConversationId).toBe('c2');
      const active = chat.conversations.find((c) => c.id === 'c2');
      expect(active).toBeDefined();
      expect(active!.messages).toHaveLength(2);
      expect(active!.messages[0].content).toBe('Show me all customers');
      expect(active!.messages[1].content).toBe('Here are the customer rows');
      expect(active!.messages[1].session).toBeDefined();
      expect(active!.messages[1].session!.segments).toHaveLength(2);
    });
  });
});

