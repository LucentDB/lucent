import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/svelte';
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
  it('renders the prompt placeholder when pending', () => {
    const cell = createCell();
    const model = createNotebookModel();
    const { container } = render(AiCell, { props: { cell, model } });
    expect(
      container.querySelector('textarea')?.getAttribute('placeholder'),
    ).toMatch(/ask a question about your data/i);
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

  it('renders a plain prompt verbatim, never as markdown', () => {
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    model.cells[0].source = 'Which airplanes fly the most';
    const { container } = render(AiCell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('textarea')?.value).toBe(
      'Which airplanes fly the most',
    );
    expect(container.querySelector('.markdown-body')).toBeNull();
  });

  it('keeps a live input while the cell is selected, so one click types', async () => {
    // A SQL cell feels immediate because its editor is already mounted for the
    // selected cell. Without the same treatment an AI cell needed two clicks:
    // the first only entered edit mode, the second finally reached an input.
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    const { container } = render(AiCell, {
      props: { cell: model.cells[0], model, editing: false, selected: true },
    });
    expect(container.querySelector('textarea')).toBeTruthy();
    expect(container.querySelector('.display')).toBeNull();
  });

  it('keeps a live input on an unselected cell too, so the first click types', async () => {
    // Gating the live input on `selected` is what made the AI cell need two
    // clicks. The first pointerdown selected the cell, which swapped the
    // rendered prompt for a textarea in the same tick — so the click that
    // would have entered edit mode landed on a button that no longer existed.
    // The SQL cell never had this problem: its editor is already mounted for
    // any cell near the viewport, so the click lands *in* the input.
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    model.cells[0].source = 'which flights are delayed';
    const { container } = render(AiCell, {
      props: { cell: model.cells[0], model, editing: false, selected: false },
    });
    expect(container.querySelector('textarea')).toBeTruthy();
    expect(container.querySelector('.display')).toBeNull();
  });

  it('enters edit mode from a single click on an unselected cell', async () => {
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    let entered = 0;
    const { container } = render(AiCell, {
      props: {
        cell: model.cells[0],
        model,
        editing: false,
        selected: false,
        onEnterEdit: () => entered++,
      },
    });
    // One gesture: the browser focuses the textarea the click landed in.
    const ta = container.querySelector('textarea')!;
    await fireEvent.focus(ta);
    expect(entered).toBe(1);
  });

  it('never shows a live input while the cell is running', async () => {
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    model.cells[0].status = 'running';
    const { container } = render(AiCell, {
      props: { cell: model.cells[0], model, editing: false, selected: true },
    });
    expect(container.querySelector('textarea')).toBeNull();
  });

  it('focusing the live input enters edit mode', async () => {
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    let entered = 0;
    const { container } = render(AiCell, {
      props: {
        cell: model.cells[0],
        model,
        editing: false,
        selected: true,
        onEnterEdit: () => entered++,
      },
    });
    const ta = container.querySelector('textarea')!;
    await fireEvent.focus(ta);
    expect(entered).toBe(1);
  });

  // The input is always mounted now, so notebook-controlled edit mode governs
  // focus rather than which element exists.
  it('focuses the input when notebook edit mode opens', async () => {
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    const { container, rerender } = render(AiCell, {
      props: { cell: model.cells[0], model, editing: false },
    });
    const ta = container.querySelector('textarea')!;
    expect(document.activeElement).not.toBe(ta);

    await rerender({ cell: model.cells[0], model, editing: true });
    expect(document.activeElement).toBe(ta);
  });

  it('gives up focus when edit mode closes, so command keys still navigate', async () => {
    // Notebook's keymap ignores keys whose target is a textarea, so a cell that
    // keeps focus after Escape kills J/K navigation until you click elsewhere.
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    const { container, rerender } = render(AiCell, {
      props: { cell: model.cells[0], model, editing: true },
    });
    const ta = container.querySelector('textarea')!;
    expect(document.activeElement).toBe(ta);

    await rerender({ cell: model.cells[0], model, editing: false });
    expect(document.activeElement).not.toBe(ta);
  });

  it('falls back to the rendered prompt while the cell runs', async () => {
    const model = createNotebookModel();
    model.cells[0].kind = 'ai';
    model.cells[0].source = 'which flights are delayed';
    model.cells[0].status = 'running';
    const { container } = render(AiCell, {
      props: { cell: model.cells[0], model, editing: false, selected: true },
    });
    expect(container.querySelector('textarea')).toBeNull();
    expect(container.querySelector('.display')).toBeTruthy();
  });
});
