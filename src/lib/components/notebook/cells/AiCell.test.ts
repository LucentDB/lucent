import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/svelte';
import AiCell from './AiCell.svelte';
import AiContextIndicator from '../AiContextIndicator.svelte';
import { createNotebookModel } from '../../../stores/notebook.svelte.ts';

afterEach(cleanup);

function createCell(overrides: Record<string, unknown> = {}) {
  return {
    id: 'test-ai-cell',
    kind: 'ai' as const,
    source: '',
    collapsed: false,
    outputs: null,
    status: 'pending' as const,
    execution_order: null,
    duration_ms: null,
    error: null,
    stale_since: null,
    ai_state: {
      conversation_id: 'conv-1',
      final_sql: null,
      response: null,
      messages: [],
      tool_calls: [],
    },
    ...overrides,
  };
}

describe('AiCell', () => {
  it('renders prompt display when pending', () => {
    const cell = createCell();
    const model = createNotebookModel();
    render(AiCell, { props: { cell, model } });
    expect(screen.getByText(/ask a question about your data/i)).toBeTruthy();
  });

  it('shows output when status is ok with table result', () => {
    const cell = createCell({
      source: 'find data',
      status: 'ok',
      outputs: {
        type: 'table',
        columns: [{ name: 'x', type: 'int4' }],
        rows: [[1]],
        total_count: 1,
        is_truncated: false,
        page_size: 10,
        is_wrappable: true,
      },
      ai_state: {
        conversation_id: 'c1',
        final_sql: 'SELECT 1 AS x',
        response: null,
        messages: [],
        tool_calls: [],
      },
    });
    const model = createNotebookModel();
    const { container } = render(AiCell, { props: { cell, model } });
    expect(container.textContent).toContain('SELECT 1 AS x');
  });

  it('shows context indicator with prior cell count', () => {
    const { container } = render(AiContextIndicator, {
      props: { priorCellCount: 3, isBudgetCapped: false },
    });
    expect(container.textContent).toContain('3 prior cells');
  });

  it('has no inline run button — cells run from the gutter and keyboard', () => {
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    const { container } = render(AiCell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.run-btn')).toBeNull();
  });

  it('renders a plain prompt as plain text, not wrapped markdown', () => {
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    model.cells[0].source = 'Which airplanes fly the most';
    const { container } = render(AiCell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.plain')).toBeTruthy();
    expect(container.querySelector('.markdown-body')).toBeNull();
  });

  it('follows notebook-controlled edit mode', async () => {
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    const { container, rerender } = render(AiCell, {
      props: { cell: model.cells[0], model, editing: false },
    });

    await rerender({ cell: model.cells[0], model, editing: true });
    expect(container.querySelector('textarea')).toBeTruthy();

    await rerender({ cell: model.cells[0], model, editing: false });
    expect(container.querySelector('.display')).toBeTruthy();
  });
});
