import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/svelte';
import PermissionRequestCard from './PermissionRequestCard.svelte';
import type { AgentPermissionPayload } from '../../ipc/ai.ts';

afterEach(cleanup);

const permission: AgentPermissionPayload = {
  conversationId: 'conv_1',
  title: 'Run a shell command',
  description: 'The agent wants to run: ls -la',
  options: [
    { id: 'allow_once', name: 'Allow once' },
    { id: 'allow_always', name: 'Allow always' },
    { id: 'deny', name: 'Deny' },
  ],
};

describe('PermissionRequestCard', () => {
  it('renders the agent title, description, and permission options', () => {
    render(PermissionRequestCard, {
      permission,
      onAllow: vi.fn(),
      onReject: vi.fn(),
    });
    expect(screen.getByText('Run a shell command')).toBeTruthy();
    expect(screen.getByText('The agent wants to run: ls -la')).toBeTruthy();
    expect(screen.getByText('Allow always')).toBeTruthy();
    expect(screen.getByText('Deny')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Allow once' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Reject' })).toBeTruthy();
  });

  it('emits allow when the Allow once button is clicked', async () => {
    const onAllow = vi.fn();
    const onReject = vi.fn();
    render(PermissionRequestCard, { permission, onAllow, onReject });
    await fireEvent.click(screen.getByRole('button', { name: 'Allow once' }));
    expect(onAllow).toHaveBeenCalledTimes(1);
    expect(onReject).not.toHaveBeenCalled();
  });

  it('emits reject when the Reject button is clicked', async () => {
    const onAllow = vi.fn();
    const onReject = vi.fn();
    render(PermissionRequestCard, { permission, onAllow, onReject });
    await fireEvent.click(screen.getByRole('button', { name: 'Reject' }));
    expect(onReject).toHaveBeenCalledTimes(1);
    expect(onAllow).not.toHaveBeenCalled();
  });

  it('routes the dismiss close button through the reject path', async () => {
    const onAllow = vi.fn();
    const onReject = vi.fn();
    render(PermissionRequestCard, { permission, onAllow, onReject });
    await fireEvent.click(
      screen.getByRole('button', { name: 'Dismiss permission request' }),
    );
    expect(onReject).toHaveBeenCalledTimes(1);
    expect(onAllow).not.toHaveBeenCalled();
  });
});
