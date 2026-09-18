/// <reference types="vite/client" />

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

afterEach(() => {
  cleanup();
});

describe('ConnectionCard accessibility', () => {
  it('renders action buttons with explicit ARIA labels', () => {
    render(ConnectionCard, { profile: sampleProfile });

    const testBtn = screen.getByRole('button', { name: 'Test connection' });
    const editBtn = screen.getByRole('button', { name: 'Edit profile' });
    const duplicateBtn = screen.getByRole('button', {
      name: 'Duplicate connection',
    });
    const deleteBtn = screen.getByRole('button', { name: 'Delete connection' });

    expect(testBtn).not.toBeNull();
    expect(editBtn).not.toBeNull();
    expect(duplicateBtn).not.toBeNull();
    expect(deleteBtn).not.toBeNull();
  });
});
