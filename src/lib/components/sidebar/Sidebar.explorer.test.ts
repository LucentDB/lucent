import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  Channel: class {},
}));

import { render, screen, waitFor, cleanup } from '@testing-library/svelte';
import SidebarHarness from './SidebarHarness.svelte';

afterEach(cleanup);

function mockCatalog() {
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'get_databases') return [{ name: 'app', is_current: true }];
    if (cmd === 'get_schemas') return [{ name: 'public', path: ['public'] }];
    if (cmd === 'get_schema_objects') {
      return { objects: [{ name: 'users', kind: 'table', row_count: 3 }] };
    }
    if (cmd === 'list_connections') return [];
    if (cmd === 'list_drivers') return [];
    return null;
  });
}

beforeEach(() => {
  invoke.mockReset();
  mockCatalog();
});

describe('Sidebar explorer', () => {
  it('renders the current database tree on mount', async () => {
    render(SidebarHarness, { props: { onObjectClick: () => {} } });
    await waitFor(() => expect(screen.getByText('app')).toBeTruthy());
  });

  it('keeps an expanded schema expanded across a refresh', async () => {
    const { getByText, getByTitle } = render(SidebarHarness, {
      props: { onObjectClick: () => {} },
    });
    await waitFor(() => expect(getByText('public')).toBeTruthy());

    getByText('public').click();
    await waitFor(() => expect(getByText('users')).toBeTruthy());

    getByTitle(/refresh/i).click();

    // The assertion the generation counters existed to protect: a refresh must
    // not collapse the branch the user is exploring.
    await waitFor(() => expect(getByText('users')).toBeTruthy());
  });

  it('does not fetch a schema objects list until its branch is expanded', async () => {
    render(SidebarHarness, { props: { onObjectClick: () => {} } });
    await waitFor(() => expect(screen.getByText('public')).toBeTruthy());
    expect(
      invoke.mock.calls.filter(([c]) => c === 'get_schema_objects'),
    ).toHaveLength(0);
  });
});
