import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/svelte';
import ToolCallCard from './ToolCallCard.svelte';

afterEach(cleanup);

const base = {
  id: 'tc1',
  name: 'run_readonly_query',
  args: { sql: 'select 1' },
  summary: null,
};

const queryOutput = {
  type: 'query_result' as const,
  columns: [{ name: 'x', type: 'INTEGER' }],
  rows: [[1]],
  row_count: 3,
  sql: 'select 1',
  execution_time_ms: 12,
  truncated: false,
};

describe('ToolCallCard status model', () => {
  it('shows Running… while running', () => {
    render(ToolCallCard, { tool: { ...base, status: 'running' } });
    expect(screen.getByText('Running…')).toBeTruthy();
  });

  it('shows the summary for completed calls', () => {
    render(ToolCallCard, {
      tool: { ...base, status: 'completed', summary: '1 row' },
    });
    expect(screen.getByText('1 row')).toBeTruthy();
  });

  it('shows Failed with the error text and expands by default', () => {
    const { container } = render(ToolCallCard, {
      tool: { ...base, status: 'failed', summary: 'read-only guard refused' },
    });
    expect(screen.getByText('Failed')).toBeTruthy();
    expect(screen.getByText('read-only guard refused')).toBeTruthy();
    // Expanded by default → the Input section is visible without a click.
    expect(screen.getByText('Input')).toBeTruthy();
    expect(container.querySelector('.tcc.err')).toBeTruthy();
  });

  it('shows Stopped for cancelled turns', () => {
    render(ToolCallCard, { tool: { ...base, status: 'stopped' } });
    expect(screen.getByText('Stopped')).toBeTruthy();
  });

  it('renders the results chip from the query_result payload', async () => {
    render(ToolCallCard, {
      tool: {
        ...base,
        status: 'completed',
        summary: '3 rows',
        output: queryOutput,
      },
    });
    await fireEvent.click(screen.getByRole('button'));
    expect(screen.getAllByText('3 rows')).toHaveLength(2);
    expect(screen.getByText('12ms')).toBeTruthy();
  });

  it('renders the truncated chip when query result is truncated', async () => {
    render(ToolCallCard, {
      tool: {
        ...base,
        status: 'completed',
        summary: '3 rows',
        output: { ...queryOutput, truncated: true },
      },
    });
    await fireEvent.click(screen.getByRole('button'));
    expect(screen.getByText('truncated')).toBeTruthy();
  });

  it('prettifies a tool id but leaves a free-text title exactly as it is', () => {
    // ACP has no tool name: `name` is the agent's own title, and for a
    // bash-first agent that is the whole shell command. Underscore-stripping
    // plus `text-transform: capitalize` turned those into commands that were
    // never run, so the transform only applies to real identifiers.
    const { container } = render(ToolCallCard, {
      tool: { ...base, name: 'run_readonly_query', status: 'completed' },
    });
    const name = container.querySelector('.tcc-name')!;
    expect(name.textContent).toBe('run readonly query');
    expect(name.classList.contains('raw')).toBe(false);
    cleanup();

    const command =
      './lucent-tool run_readonly_query \'{"sql":"SELECT * FROM bookings.airplanes_data"}\'';
    const { container: c2 } = render(ToolCallCard, {
      tool: { ...base, name: command, status: 'completed' },
    });
    const raw = c2.querySelector('.tcc-name')!;
    expect(raw.textContent).toBe(command);
    expect(raw.textContent).toContain('airplanes_data');
    expect(raw.classList.contains('raw')).toBe(true);
  });

  it('keeps the header to one short line when the summary is a table preview', async () => {
    // An agent that reaches the tools over the CLI reports their stdout — the
    // Markdown row preview — as its result. The body renders those rows as a
    // grid; the header must stay a status indicator.
    const preview =
      'Query: select 1\nResult: 3 rows in 1ms\n\n**Preview (first 3 of 3 rows):**\n\n| x |\n|---|\n| 1 |';
    const { container } = render(ToolCallCard, {
      tool: {
        ...base,
        status: 'completed',
        summary: preview,
        output: queryOutput,
      },
    });
    const status = container.querySelector('.tcc-status')!;
    expect(status.textContent).toBe('Query: select 1');
    expect(status.textContent).not.toContain('|');
    // The rows are still shown — once, in the expanded body.
    await fireEvent.click(screen.getByRole('button'));
    expect(container.querySelector('.tcc-table')).toBeTruthy();
  });

  it('truncates an over-long single-line summary', () => {
    const { container } = render(ToolCallCard, {
      tool: { ...base, status: 'completed', summary: 'x'.repeat(200) },
    });
    const status = container.querySelector('.tcc-status')!;
    expect(status.textContent!.length).toBeLessThanOrEqual(80);
    expect(status.textContent!.endsWith('…')).toBe(true);
  });

  it('legacy: an error-prefix summary renders as failed', () => {
    render(ToolCallCard, { tool: { ...base, summary: 'error: boom' } });
    expect(screen.getByText('error: boom')).toBeTruthy();
  });

  it('legacy: cellCompleted fallback works when no summary and no status', () => {
    render(ToolCallCard, { tool: { ...base }, cellCompleted: true });
    expect(screen.getByText('Done')).toBeTruthy();
  });
});
