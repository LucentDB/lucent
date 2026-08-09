import { describe, it, expect, afterEach, vi } from 'vitest';
import { render, screen, cleanup } from '@testing-library/svelte';
import NotebookToolbar from './NotebookToolbar.svelte';

afterEach(cleanup);

describe('NotebookToolbar', () => {
  it('shows Run All button', () => {
    render(NotebookToolbar, {
      props: {
        onRunAll: vi.fn(),
        onClearOutputs: vi.fn(),
        onRestartSession: vi.fn(),
      },
    });
    expect(screen.getByText('Run All')).toBeTruthy();
  });

  it('disables Run All when already running', () => {
    render(NotebookToolbar, { props: { onRunAll: vi.fn(), isRunning: true } });
    expect(
      (screen.getByText('Run All').closest('button') as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it('shows the clear-outputs button, labelled in full on its tooltip', () => {
    render(NotebookToolbar, {
      props: {
        onRunAll: vi.fn(),
        onClearOutputs: vi.fn(),
        onRestartSession: vi.fn(),
      },
    });
    const btn = screen
      .getByText('Clear')
      .closest('button') as HTMLButtonElement;
    expect(btn.title).toBe('Clear all outputs');
  });

  it('shows the restart-session button, labelled in full on its tooltip', () => {
    render(NotebookToolbar, {
      props: {
        onRunAll: vi.fn(),
        onClearOutputs: vi.fn(),
        onRestartSession: vi.fn(),
      },
    });
    const btn = screen
      .getByText('Restart')
      .closest('button') as HTMLButtonElement;
    expect(btn.title).toBe('Restart session');
  });

  it('disables all buttons when running', () => {
    render(NotebookToolbar, { props: { onRunAll: vi.fn(), isRunning: true } });
    const buttons = screen.getAllByRole('button');
    for (const btn of buttons) {
      expect((btn as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it('shows connection badge with database name', () => {
    // The badge is composed of separate spans so the connection and database
    // can be styled apart, so assert on the badge's collapsed text.
    const { container } = render(NotebookToolbar, {
      props: {
        onRunAll: vi.fn(),
        connectionName: 'MyDB',
        databaseName: 'postgres',
      },
    });
    const badge = container.querySelector('.connection-badge') as HTMLElement;
    expect(badge.textContent?.replace(/\s+/g, '')).toBe('MyDB/postgres');
  });

  it('does not show connection badge when no connection info', () => {
    render(NotebookToolbar, { props: { onRunAll: vi.fn() } });
    expect(screen.queryByText(/\//)).toBeNull();
  });
});
