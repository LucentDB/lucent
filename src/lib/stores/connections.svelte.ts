// Connection session store — Svelte 5 runes.
// Owns live-connection state only: which profile is active, connection
// status, capabilities. The saved-profile CACHE lives in the connections
// query (src/lib/queries/connections.ts); live status has no server copy to
// revalidate against, so it must not move there.

import { invoke } from '@tauri-apps/api/core';
import { queryClient } from '../queries/client.ts';

// ─── Types ──────────────────────────────────────────────────────────────────

export interface DriverField {
  key: string;
  label: string;
  kind: 'text' | 'number' | 'password' | 'path' | 'select';
  required: boolean;
  default: string | null;
  options: string[];
  placeholder: string | null;
}

export interface DriverDescriptor {
  id: string;
  displayName: string;
  fields: DriverField[];
  hasSecret: boolean;
}

export interface ConnectionCapabilities {
  driver: string;
  displayName: string;
  engineEnforcedReadonly: boolean;
  readonlyDisclosure: string | null;
  dialect: string;
}

export interface ConnectionProfile {
  id: string;
  name: string;
  driver: string;
  /** `@mention` handle used by the AI to address this connection. */
  alias: string | null;
  /** Driver-defined connection parameters — see `drivers`. */
  params: Record<string, string>;
  sshTunnelId: string | null;
  group: string | null;
  color: string | null;
  icon: string | null;
  lastUsed: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface TestConnectionResult {
  success: boolean;
  message: string;
  serverVersion: string | null;
}

export type ConnectionStatus =
  'disconnected' | 'connecting' | 'connected' | 'error';

// ─── Store ──────────────────────────────────────────────────────────────────

class ConnectionsStore {
  /** Currently active profile ID (connected or connecting) */
  activeProfileId = $state<string | null>(null);
  /** Connection status */
  status = $state<ConnectionStatus>('disconnected');
  /** Error message when status === 'error' */
  errorMessage = $state<string | null>(null);
  /** Loading states per profile ID for test-connection */
  testingIds = $state<Set<string>>(new Set());
  /** Capabilities of the live connection, or null when disconnected. */
  capabilities = $state<ConnectionCapabilities | null>(null);

  async getProfile(id: string): Promise<ConnectionProfile | null> {
    try {
      return await invoke<ConnectionProfile>('get_connection', { id });
    } catch {
      return null;
    }
  }

  async testConnection(id: string): Promise<TestConnectionResult> {
    this.testingIds = new Set([...this.testingIds, id]);
    try {
      return await invoke<TestConnectionResult>('test_connection', { id });
    } finally {
      const next = new Set(this.testingIds);
      next.delete(id);
      this.testingIds = next;
    }
  }

  async connectToProfile(id: string) {
    this.status = 'connecting';
    this.errorMessage = null;
    try {
      const result = await invoke('connect', {
        connectionId: id,
        config: null,
      });
      this.activeProfileId = id;
      this.status = 'connected';
      this.capabilities = await invoke<ConnectionCapabilities | null>(
        'connection_capabilities',
      );
      // A new connection means a different catalog. Drop cached explorer
      // branches outright rather than letting a stale tree flash on screen.
      queryClient.removeQueries({ queryKey: ['explorer'] });
      return result;
    } catch (e) {
      const msg =
        typeof e === 'string'
          ? e
          : ((e as any)?.message ?? 'Connection failed');
      this.status = 'error';
      this.errorMessage = msg;
      throw e;
    }
  }

  /**
   * Connect with an inline driver config (no saved profile). `secret` is the
   * keychain-format password for drivers that use one.
   */
  async connectInline(config: {
    driver: string;
    params: Record<string, string>;
    secret?: string;
  }) {
    this.status = 'connecting';
    this.errorMessage = null;
    try {
      await invoke('connect', { connectionId: null, config });
      this.activeProfileId = null;
      this.status = 'connected';
      this.capabilities = await invoke<ConnectionCapabilities | null>(
        'connection_capabilities',
      );
      // A new connection means a different catalog. Drop cached explorer
      // branches outright rather than letting a stale tree flash on screen.
      queryClient.removeQueries({ queryKey: ['explorer'] });
    } catch (e) {
      const msg =
        typeof e === 'string'
          ? e
          : ((e as any)?.message ?? 'Connection failed');
      this.status = 'error';
      this.errorMessage = msg;
      throw e;
    }
  }

  async disconnect() {
    try {
      await invoke('disconnect');
    } finally {
      this.status = 'disconnected';
      this.activeProfileId = null;
      this.errorMessage = null;
      this.capabilities = null;
    }
  }

  /** Set status externally (e.g. when App detects disconnect) */
  setDisconnected() {
    this.status = 'disconnected';
    this.activeProfileId = null;
    this.errorMessage = null;
    this.capabilities = null;
  }
}

export const connections = new ConnectionsStore();
