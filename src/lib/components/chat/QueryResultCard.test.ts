import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import {
  render,
  screen,
  fireEvent,
  cleanup,
  waitFor,
} from '@testing-library/svelte';

// QueryResultCard reaches the Tauri IPC boundary through saveGoldenQuery, and
// pulls in the chat store (which imports listChatConversations / invoke).
// Stub both so this stays a pure interaction test.
const saveGoldenQuery = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => []),
  Channel: class {},
}));

vi.mock('../../ipc/ai.ts', () => ({
  saveGoldenQuery: (...args: unknown[]) => saveGoldenQuery(...args),
  listChatConversations: vi.fn(async () => []),
  loadChatConversation: vi.fn(async () => []),
}));

import QueryResultCard from './QueryResultCard.svelte';
import { chat, createConversation } from '../../stores/chat.svelte.ts';

afterEach(cleanup);

const qr = {
  sql: 'select count(*) from users',
  rowCount: 1,
  executionTimeMs: 3,
};

function seedActiveConversation() {
  const conv = createConversation('conn1');
  conv.messages = [
    {
      id: 'm1',
      role: 'user',
      content: 'count the users',
      createdAt: Date.now(),
    },
  ];
  chat.conversations = [conv];
  chat.activeConversationId = conv.id;
}

async function clickSave() {
  await fireEvent.click(
    screen.getByRole('button', { name: /Save as Golden Query/ }),
  );
}

describe('QueryResultCard save error (F-C5)', () => {
  beforeEach(() => {
    saveGoldenQuery.mockReset();
    seedActiveConversation();
  });

  it('announces an inline alert instead of silently reverting when the save fails', async () => {
    saveGoldenQuery.mockRejectedValueOnce(new Error('database is locked'));
    render(QueryResultCard, { qr });

    await clickSave();

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toMatch(/database is locked/);
    // Not silently reverted: the button is back to the save label but the
    // failure is visible and the user can retry.
    const button = screen.getByRole('button', { name: /Save as Golden Query/ });
    expect(button.hasAttribute('disabled')).toBe(false);
    expect(screen.queryByText('✓ Saved')).toBeNull();
  });

  it('clears the alert on a successful retry', async () => {
    saveGoldenQuery.mockRejectedValueOnce(new Error('database is locked'));
    render(QueryResultCard, { qr });

    await clickSave();
    await screen.findByRole('alert');

    saveGoldenQuery.mockResolvedValueOnce({ id: 'g1' });
    await clickSave();

    expect(await screen.findByText('✓ Saved')).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  });
});
