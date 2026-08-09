import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent, cleanup, waitFor } from '@testing-library/svelte';

// The drawer tails Tauri IPC through client.getLogs — stub the boundary so
// these stay pure component tests.
vi.mock('../ipc/client.js', () => ({
  getLogs: vi.fn(),
}));

import LogsDrawer from './LogsDrawer.svelte';
import { getLogs } from '../ipc/client.js';

const mockedGetLogs = vi.mocked(getLogs);

beforeEach(() => {
  mockedGetLogs.mockReset();
  mockedGetLogs.mockResolvedValue([]);
});

afterEach(cleanup);

describe('LogsDrawer', () => {
  it('shows a placeholder before any lines exist', () => {
    const { getByText } = render(LogsDrawer, { onClose: vi.fn() });
    expect(getByText(/No log lines yet/)).toBeTruthy();
  });

  it('renders lines fetched on open and tails from the held count', async () => {
    mockedGetLogs.mockResolvedValue(['first', 'second']);
    const { container } = render(LogsDrawer, { onClose: vi.fn() });

    await waitFor(() => {
      expect(container.querySelectorAll('.log-line')).toHaveLength(2);
    });
    // First fetch asks for the whole buffer (after = 0)…
    expect(mockedGetLogs).toHaveBeenCalledWith(0);
    // …and the next interval poll passes the held count so only new lines
    // arrive (interval is 2s, so give waitFor room past it).
    mockedGetLogs.mockResolvedValue(['third']);
    await waitFor(
      () => {
        expect(mockedGetLogs).toHaveBeenCalledWith(2);
      },
      { timeout: 3500, interval: 100 },
    );
  });

  it('shows a fetch error in the list instead of crashing', async () => {
    mockedGetLogs.mockRejectedValueOnce({ message: 'IPC broke' });
    const { container } = render(LogsDrawer, { onClose: vi.fn() });

    await waitFor(() => {
      expect(container.querySelector('.log-line.error')?.textContent).toContain(
        'IPC broke',
      );
    });
  });

  it('closes via the close button', async () => {
    const onClose = vi.fn();
    const { getByTitle } = render(LogsDrawer, { onClose });
    await fireEvent.click(getByTitle('Close logs'));
    expect(onClose).toHaveBeenCalled();
  });

  it('keeps polling on the interval while open', async () => {
    render(LogsDrawer, { onClose: vi.fn() });
    // First call is the mount-time poll; the interval fires one more within
    // the window. Real timers — Svelte 5 effects hang under vitest fake
    // timers (queueMicrotask is faked and effects never flush).
    await waitFor(
      () => {
        expect(mockedGetLogs).toHaveBeenCalledTimes(2);
      },
      { timeout: 3500, interval: 100 },
    );
  });
});
