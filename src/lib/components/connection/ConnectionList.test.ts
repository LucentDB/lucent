import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import ConnectionList from './ConnectionList.svelte';
import type { ConnectionProfile } from '../../stores/connections.svelte';

const sampleProfile: ConnectionProfile = {
  id: 'profile-1',
  name: 'Test Database',
  driver: 'postgres',
  alias: null,
  params: { host: 'localhost', port: '5432' },
  sshTunnelId: null,
  group: null,
  color: '#3b82f6',
  icon: null,
  lastUsed: null,
  createdAt: '',
  updatedAt: '',
};

describe('ConnectionList accessible elements', () => {
  afterEach(() => {
    cleanup();
  });

  it('renders search input and view toggle button with accessible ARIA labels', () => {
    render(ConnectionList, {
      props: {
        profiles: [sampleProfile],
        groupedProfiles: [{ name: 'Default', profiles: [sampleProfile] }],
        loading: false,
        activeProfileId: null,
        testingIds: new Set<string>(),
      },
    });

    expect(
      screen.getByRole('textbox', { name: 'Search connections' }),
    ).not.toBeNull();
    expect(
      screen.getByRole('button', { name: 'Toggle view mode' }),
    ).not.toBeNull();
  });

  it('focuses search input on "/" keydown and clears search query on "Escape"', async () => {
    const { fireEvent } = await import('@testing-library/svelte');
    render(ConnectionList, {
      props: {
        profiles: [sampleProfile],
        groupedProfiles: [{ name: 'Default', profiles: [sampleProfile] }],
        loading: false,
        activeProfileId: null,
        testingIds: new Set<string>(),
      },
    });

    const searchInput = screen.getByRole('textbox', {
      name: 'Search connections',
    }) as HTMLInputElement;

    expect(document.activeElement).not.toBe(searchInput);

    window.dispatchEvent(new KeyboardEvent('keydown', { key: '/' }));
    expect(document.activeElement).toBe(searchInput);

    await fireEvent.input(searchInput, { target: { value: 'test' } });
    expect(searchInput.value).toBe('test');

    const { tick } = await import('svelte');
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await tick();
    expect(searchInput.value).toBe('');
    expect(document.activeElement).not.toBe(searchInput);
  });
});
