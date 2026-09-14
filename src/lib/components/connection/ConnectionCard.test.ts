import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import ConnectionCard from './ConnectionCard.svelte';
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

describe('ConnectionCard accessible labels', () => {
  afterEach(() => {
    cleanup();
  });

  it('renders icon-only action buttons with accessible ARIA labels', () => {
    render(ConnectionCard, { profile: sampleProfile });

    expect(
      screen.getByRole('button', { name: 'Test connection' }),
    ).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Edit profile' })).not.toBeNull();
    expect(
      screen.getByRole('button', { name: 'Duplicate connection profile' }),
    ).not.toBeNull();
    expect(
      screen.getByRole('button', { name: 'Delete connection profile' }),
    ).not.toBeNull();
  });
});
