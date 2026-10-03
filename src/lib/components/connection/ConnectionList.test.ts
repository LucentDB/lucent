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
});
