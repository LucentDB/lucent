// Connection profiles store — Svelte 5 runes.
// Provides reactive state for saved connection profiles and manages
// IPC calls to the backend.

import { invoke } from '@tauri-apps/api/core';

// ─── Types ──────────────────────────────────────────────────────────────────

export interface ConnectionProfile {
  id: string;
  name: string;
  driver: string;
  host: string;
  port: number;
  user: string;
  database: string;
  sslMode: 'disable' | 'prefer' | 'require';
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
  /** All saved profiles */
  profiles = $state<ConnectionProfile[]>([]);
  /** Currently active profile ID (connected or connecting) */
  activeProfileId = $state<string | null>(null);
  /** Connection status */
  status = $state<ConnectionStatus>('disconnected');
  /** Error message when status === 'error' */
  errorMessage = $state<string | null>(null);
  /** Loading state for initial load */
  loading = $state(true);
  /** Loading states per profile ID for test-connection */
  testingIds = $state<Set<string>>(new Set());

  /** Active profile object (derived) */
  activeProfile = $derived(
    this.profiles.find((p) => p.id === this.activeProfileId) ?? null,
  );

  /** Grouped profiles for display */
  groupedProfiles = $derived.by(() => {
    const groups: { name: string; profiles: ConnectionProfile[] }[] = [];
    const grouped = new Map<string, ConnectionProfile[]>();

    for (const p of this.profiles) {
      const key = p.group ?? '__ungrouped__';
      if (!grouped.has(key)) grouped.set(key, []);
      grouped.get(key)!.push(p);
    }

    for (const [key, list] of grouped) {
      groups.push({
        name: key === '__ungrouped__' ? '' : key,
        profiles: list,
      });
    }

    // Sort groups: named groups first alphabetically, ungrouped last
    groups.sort((a, b) => {
      if (a.name === '' && b.name === '') return 0;
      if (a.name === '') return 1;
      if (b.name === '') return -1;
      return a.name.localeCompare(b.name);
    });

    return groups;
  });

  constructor() {
    // Call loadProfiles directly — $effect is not valid outside .svelte components.
    // The store is instantiated at module level, so this runs on import.
    this.loadProfiles();
  }

  async loadProfiles() {
    this.loading = true;
    try {
      const profiles = await invoke<ConnectionProfile[]>('list_connections');
      this.profiles = profiles;
    } catch (e) {
      console.error('Failed to load profiles:', e);
    } finally {
      this.loading = false;
    }
  }

  async saveProfile(
    profile: ConnectionProfile,
    password?: string,
  ): Promise<ConnectionProfile> {
    const saved = await invoke<ConnectionProfile>('save_connection', {
      profile,
      password: password ?? null,
    });
    // Reload to get fresh state
    await this.loadProfiles();
    return saved;
  }

  async deleteProfile(id: string) {
    await invoke('delete_connection', { id });
    if (this.activeProfileId === id) {
      this.activeProfileId = null;
    }
    await this.loadProfiles();
  }

  async duplicateProfile(id: string): Promise<ConnectionProfile> {
    const copy = await invoke<ConnectionProfile>('duplicate_connection', {
      id,
    });
    await this.loadProfiles();
    return copy;
  }

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
      // Refresh profiles to update last_used
      this.loadProfiles();
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

  async connectInline(config: {
    host: string;
    port: number;
    user: string;
    password: string;
    database: string;
  }) {
    this.status = 'connecting';
    this.errorMessage = null;
    try {
      await invoke('connect', { connectionId: null, config });
      this.activeProfileId = null;
      this.status = 'connected';
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
    }
  }

  /** Set status externally (e.g. when App detects disconnect) */
  setDisconnected() {
    this.status = 'disconnected';
    this.activeProfileId = null;
    this.errorMessage = null;
  }
}

export const connections = new ConnectionsStore();
