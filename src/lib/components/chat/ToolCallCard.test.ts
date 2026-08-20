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
    render(ToolCallCard, { tool: { ...base, status: 'completed', summary: '1 row' } });
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
      tool: { ...base, status: 'completed', summary: '3 rows', output: queryOutput },
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

  it('legacy: an error-prefix summary renders as failed', () => {
    render(ToolCallCard, { tool: { ...base, summary: 'error: boom' } });
    expect(screen.getByText('error: boom')).toBeTruthy();
  });

  it('legacy: cellCompleted fallback works when no summary and no status', () => {
    render(ToolCallCard, { tool: { ...base }, cellCompleted: true });
    expect(screen.getByText('Done')).toBeTruthy();
  });
});
