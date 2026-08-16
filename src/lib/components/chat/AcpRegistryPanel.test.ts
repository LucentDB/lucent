import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/svelte';
import AcpRegistryPanel from './AcpRegistryPanel.svelte';
import type { RegistryAgentSummary } from '../../ipc/ai.ts';

afterEach(cleanup);

const installed: RegistryAgentSummary = {
  id: 'opencode',
  name: 'OpenCode',
  version: '2.0.0',
  description: 'A terminal AI agent',
  license: 'MIT',
  icon: 'https://cdn.example/opencode.png',
  installedVersion: '2.0.0',
  updateAvailable: false,
  dbTools: 'supported',
};

const outdated: RegistryAgentSummary = {
  id: 'claude-acp',
  name: 'Claude ACP',
  version: '3.1.0',
  description: 'Anthropic agent',
  license: 'Proprietary',
  icon: null,
  installedVersion: '3.0.0',
  updateAvailable: true,
  dbTools: 'supported',
};

const fresh: RegistryAgentSummary = {
  id: 'qwen',
  name: 'Qwen Agent',
  version: '1.0.0',
  description: 'Alibaba agent',
  license: 'Apache-2.0',
  icon: null,
  installedVersion: null,
  updateAvailable: false,
  dbTools: 'unknown',
};

describe('AcpRegistryPanel', () => {
  it('renders agent name, version, license and install state', () => {
    render(AcpRegistryPanel, {
      agents: [installed, outdated],
      loading: false,
      onInstall: vi.fn(),
      onUninstall: vi.fn(),
    });
    expect(screen.getByText('OpenCode')).toBeTruthy();
    expect(screen.getByText('Claude ACP')).toBeTruthy();
    expect(screen.getByText('MIT')).toBeTruthy();
    expect(screen.getByText('Proprietary')).toBeTruthy();
    // Update badge only on the outdated row
    expect(screen.getAllByText('Update available')).toHaveLength(1);
    // DB-tools badge on the curated rows (both fixtures are 'supported')
    expect(screen.getAllByText('DB tools ✓')).toHaveLength(2);
    // Installed-current shows Uninstall; installed-outdated shows Update + Uninstall
    expect(screen.getAllByRole('button', { name: 'Uninstall' })).toHaveLength(
      2,
    );
    expect(screen.getAllByRole('button', { name: 'Update' })).toHaveLength(1);
    expect(screen.queryByRole('button', { name: 'Install' })).toBeNull();
  });

  it('emits install for a not-installed agent', async () => {
    const onInstall = vi.fn();
    render(AcpRegistryPanel, {
      agents: [fresh],
      loading: false,
      onInstall,
      onUninstall: vi.fn(),
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    expect(onInstall).toHaveBeenCalledTimes(1);
    expect(onInstall).toHaveBeenCalledWith('qwen');
  });

  it('emits uninstall for an installed agent', async () => {
    const onUninstall = vi.fn();
    render(AcpRegistryPanel, {
      agents: [installed],
      loading: false,
      onInstall: vi.fn(),
      onUninstall,
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Uninstall' }));
    expect(onUninstall).toHaveBeenCalledTimes(1);
    expect(onUninstall).toHaveBeenCalledWith('opencode');
  });

  it('shows loading state', () => {
    render(AcpRegistryPanel, {
      agents: [],
      loading: true,
      onInstall: vi.fn(),
      onUninstall: vi.fn(),
    });
    expect(screen.getByText(/Loading agents/i)).toBeTruthy();
  });

  it('shows offline fallback note when agents is empty', () => {
    render(AcpRegistryPanel, {
      agents: [],
      loading: false,
      onInstall: vi.fn(),
      onUninstall: vi.fn(),
    });
    expect(
      screen.getByText('No agents available — check your connection'),
    ).toBeTruthy();
  });
});
