/// <reference types="vite/client" />

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import ConnectionProfileDrawer from './ConnectionProfileDrawer.svelte';

afterEach(cleanup);

describe('ConnectionProfileDrawer', () => {
  it('renders external agents toggle, warning notice, and copy snippet button when enabled', async () => {
    const { getByLabelText, getByText } = render(ConnectionProfileDrawer, {
      props: {
        profile: { id: 'p1', name: 'Analytics', enableExternalAgents: true },
      },
    });

    const checkbox = getByLabelText(/Allow external AI assistants/i);
    expect((checkbox as HTMLInputElement).checked).toBe(true);

    expect(
      getByText(/strictly read-only with local file system access disabled/i),
    ).toBeTruthy();
    expect(getByText(/Copy MCP Config/i)).toBeTruthy();
  });

  it('hides notice and copy button when external agents is disabled', async () => {
    const { getByLabelText, queryByText } = render(ConnectionProfileDrawer, {
      props: {
        profile: { id: 'p2', name: 'Dev', enableExternalAgents: false },
      },
    });

    const checkbox = getByLabelText(/Allow external AI assistants/i);
    expect((checkbox as HTMLInputElement).checked).toBe(false);

    expect(
      queryByText(/strictly read-only with local file system access disabled/i),
    ).toBeNull();
    expect(queryByText(/Copy MCP Config/i)).toBeNull();
  });

  it('copies MCP configuration JSON to clipboard when button clicked', async () => {
    const writeTextMock = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, {
      clipboard: {
        writeText: writeTextMock,
      },
    });

    const { getByText } = render(ConnectionProfileDrawer, {
      props: {
        profile: { id: 'p1', name: 'Analytics', enableExternalAgents: true },
      },
    });

    const copyBtn = getByText(/Copy MCP Config/i);
    await fireEvent.click(copyBtn);

    expect(writeTextMock).toHaveBeenCalledTimes(1);
    const copiedText = writeTextMock.mock.calls[0][0];
    expect(copiedText).toContain('lucent-db-tools-mcp');
    expect(copiedText).toContain('mcpServers');
  });

  it('emits onSave with updated toggles on save button click', async () => {
    const onSave = vi.fn();
    const { getByLabelText, getByText } = render(ConnectionProfileDrawer, {
      props: {
        profile: { id: 'p1', name: 'Analytics', enableExternalAgents: false },
        onSave,
      },
    });

    const agentCheckbox = getByLabelText(/Allow external AI assistants/i);
    await fireEvent.click(agentCheckbox);

    const historyCheckbox = getByLabelText(
      /Allow query history search in MCP/i,
    );
    await fireEvent.click(historyCheckbox);

    const saveBtn = getByText('Save');
    await fireEvent.click(saveBtn);

    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({
        id: 'p1',
        name: 'Analytics',
        enableExternalAgents: true,
        allowQueryHistory: true,
      }),
    );
  });
});
