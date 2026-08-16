/// <reference types="vite/client" />

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import ConnectionForm from './ConnectionForm.svelte';
import type { ConnectionProfile } from '../../stores/connections.svelte';
import componentSource from './ConnectionForm.svelte?raw';

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn(async (command: string) => {
    if (command === 'list_connections') return [];
    if (command === 'list_drivers') {
      return [
        {
          id: 'postgres',
          displayName: 'PostgreSQL',
          fields: [],
          hasSecret: false,
        },
      ];
    }
    if (command === 'test_connection') {
      return {
        success: false,
        message:
          'Authentication failed: FATAL: password authentication failed for user "postgres"',
        serverVersion: null,
      };
    }
    return null;
  }),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));

const componentStyles = componentSource.match(
  /<style>([\s\S]*?)<\/style>/,
)?.[1];

let styleElement: HTMLStyleElement;
beforeEach(() => {
  styleElement = document.createElement('style');
  styleElement.textContent = componentStyles ?? '';
  document.head.appendChild(styleElement);
});

afterEach(() => {
  cleanup();
  styleElement.remove();
});

const profile: ConnectionProfile = {
  id: 'profile-1',
  name: 'PostgreSQL',
  driver: 'postgres',
  alias: null,
  params: {},
  sshTunnelId: null,
  group: null,
  color: '#3b82f6',
  icon: null,
  lastUsed: null,
  createdAt: '',
  updatedAt: '',
};

describe('connection form actions', () => {
  it('keeps action buttons intact when a connection test returns a long error', async () => {
    render(ConnectionForm, { profile, onCancel: vi.fn() });

    await screen.getByRole('button', { name: 'Test Connection' }).click();
    await screen.findByText(
      'password authentication failed for user "postgres"',
      {
        exact: false,
      },
    );

    const saveButton = screen.getByRole('button', { name: /Save Connection/ });
    const cancelButton = screen.getByRole('button', { name: 'Cancel' });
    const errorBadge = document.querySelector('.test-error');

    expect(errorBadge).not.toBeNull();
    expect(getComputedStyle(saveButton).whiteSpace).toBe('nowrap');
    expect(getComputedStyle(saveButton).flexShrink).toBe('0');
    expect(getComputedStyle(cancelButton).whiteSpace).toBe('nowrap');
    expect(getComputedStyle(cancelButton).flexShrink).toBe('0');
    expect(getComputedStyle(errorBadge as HTMLElement).height).toBe('auto');
  });
});
