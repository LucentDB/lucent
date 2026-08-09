import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, cleanup, fireEvent } from '@testing-library/svelte';

// ThinkingCard uses the Web Animations API (element.animate), which jsdom does
// not implement — shim it so the activity stack can mount in tests.
if (!HTMLElement.prototype.animate) {
  HTMLElement.prototype.animate = (() => ({
    finished: Promise.resolve(),
    cancel: () => {},
    play: () => {},
    pause: () => {},
  })) as unknown as typeof HTMLElement.prototype.animate;
}

vi.mock('../../ipc/notebook', () => ({
  notebookAttach: vi.fn(),
  notebookDetach: vi.fn(),
  notebookClearOutputs: vi.fn(),
  notebookRunCell: vi.fn(),
  notebookCancelCell: vi.fn(),
  notebookFetchPage: vi.fn(),
  notebookCountRows: vi.fn(),
}));

import AiCellOutput from './AiCellOutput.svelte';
import { createNotebookModel } from '../../stores/notebook.svelte.ts';

afterEach(cleanup);

function aiModel(overrides: Record<string, unknown> = {}) {
  const model = createNotebookModel();
  model.convertCell(model.cells[0].id, 'ai');
  Object.assign(model.cells[0], overrides);
  // convertCell's resetFrom seeded the view from the pre-assign (empty) outputs;
  // re-seed so the view reflects the assigned outputs — the same pairing
  // cell_done performs (outputs assigned, then resetFrom).
  model.cellView.resetFrom(model.cells[0].id);
  return model;
}

function tableOutput(rowCount = 3) {
  return {
    columns: [{ name: 'n', type_name: 'int4' }],
    rows: Array.from({ length: rowCount }, (_, i) => [i + 1]),
    total_count: null,
    is_truncated: false,
    page_size: 10,
    is_wrappable: true,
  };
}

function tabLabels(container: HTMLElement): string[] {
  return [...container.querySelectorAll('.tab-btn')].map((b) =>
    (b.textContent ?? '').trim(),
  );
}

/** By label, not index: which tabs exist now depends on what the cell produced. */
async function clickTab(container: HTMLElement, label: string) {
  const tab = [...container.querySelectorAll('.tab-btn')].find(
    (b) => (b.textContent ?? '').trim() === label,
  );
  if (!tab)
    throw new Error(`no "${label}" tab; found: ${tabLabels(container)}`);
  await fireEvent.click(tab);
}

describe('AiCellOutput tabs', () => {
  it('renders every tab that has content, in a fixed order', () => {
    const model = aiModel({
      status: 'ok',
      outputs: tableOutput(),
      ai_state: {
        conversation_id: 'c',
        final_sql: 'SELECT 1',
        response: 'hello',
        messages: [],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    expect(tabLabels(container)).toEqual(['Response', 'SQL Code', 'Table']);
  });

  it('omits tabs with no content rather than greying them out', () => {
    // Three permanent tabs advertised results the cell had not produced.
    const model = aiModel({
      status: 'ok',
      ai_state: {
        conversation_id: 'c',
        final_sql: null,
        response: 'hello',
        messages: [],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    expect(tabLabels(container)).toEqual(['Response']);
  });

  it('renders no output chrome at all when the cell has produced nothing', () => {
    const model = aiModel({ status: 'ok' });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.output-header')).toBeNull();
    expect(tabLabels(container)).toEqual([]);
  });

  it('falls back to a surviving tab when the active one loses its content', async () => {
    const model = aiModel({
      status: 'ok',
      run_token: 1,
      ai_state: {
        conversation_id: 'c',
        final_sql: 'SELECT 1',
        response: 'hello',
        messages: [],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    await clickTab(container, 'SQL Code');
    expect(
      container.querySelector('.tab-btn.active')?.textContent?.trim(),
    ).toBe('SQL Code');

    model.cells[0].ai_state!.final_sql = null;
    await Promise.resolve();
    expect(
      container.querySelector('.tab-btn.active')?.textContent?.trim(),
    ).toBe('Response');
  });

  it('defaults to the Response tab', () => {
    const model = aiModel({
      status: 'ok',
      outputs: tableOutput(),
      ai_state: {
        conversation_id: 'c',
        final_sql: 'SELECT 1',
        response: 'hello',
        messages: [],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    expect(
      container.querySelector('.tab-btn.active')?.textContent?.trim(),
    ).toBe('Response');
  });

  it('keeps the user’s tab choice when cell state changes mid-run', async () => {
    const model = aiModel({
      status: 'ok',
      run_token: 1,
      outputs: tableOutput(),
      ai_state: {
        conversation_id: 'c',
        final_sql: 'SELECT 1',
        response: 'hello',
        messages: [],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    await clickTab(container, 'SQL Code');
    expect(
      container.querySelector('.tab-btn.active')?.textContent?.trim(),
    ).toBe('SQL Code');

    // A later streaming update must not yank the tab back to Response.
    model.cells[0].ai_state!.response = 'hello, updated';
    await Promise.resolve();
    expect(
      container.querySelector('.tab-btn.active')?.textContent?.trim(),
    ).toBe('SQL Code');
  });

  it('syntax highlights the SQL it produced', async () => {
    // Plain <pre> text made the model's SQL harder to read than the SQL the
    // user types two cells below it.
    const model = aiModel({
      status: 'ok',
      ai_state: {
        conversation_id: 'c',
        final_sql: "SELECT 1 FROM t WHERE x = 'a'",
        response: null,
        messages: [],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    const block = container.querySelector('.sql-block') as HTMLElement;
    expect(block).toBeTruthy();
    expect(block.querySelector('.tok-keyword')?.textContent).toBe('SELECT');
    expect(block.querySelector('.tok-string')).toBeTruthy();
    // The full statement survives tokenisation, whitespace included.
    expect(block.textContent).toBe("SELECT 1 FROM t WHERE x = 'a'");
  });

  it('offers Insert into Next SQL Cell on the SQL tab', async () => {
    const onEditSql = vi.fn();
    const model = aiModel({
      status: 'ok',
      ai_state: {
        conversation_id: 'c',
        final_sql: 'SELECT 42',
        response: null,
        messages: [],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model, onEditSql },
    });
    await clickTab(container, 'SQL Code');
    const insert = container.querySelector(
      '.insert-sql-btn',
    ) as HTMLButtonElement;
    expect(insert).toBeTruthy();
    await fireEvent.click(insert);
    expect(onEditSql).toHaveBeenCalledWith('SELECT 42');
  });

  it('renders the Table tab through the shared grid', async () => {
    const model = aiModel({ status: 'ok', outputs: tableOutput(25) });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    await clickTab(container, 'Table');
    expect(container.querySelector('.results-grid.embedded')).toBeTruthy();
    expect(container.querySelectorAll('tbody tr').length).toBe(10);
  });
});

describe('AiCellOutput activity stack', () => {
  it('has no bordered wrapper around the thinking section', () => {
    const model = aiModel({
      status: 'running',
      ai_state: {
        conversation_id: 'c',
        final_sql: null,
        response: null,
        messages: [{ thinking: 'pondering' }],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.working-section')).toBeNull();
    expect(container.querySelector('.activity-body')).toBeTruthy();
  });

  it('keeps thinking history reachable after the run completes', async () => {
    // Collapsed once the run ends, but never discarded.
    const model = aiModel({
      status: 'ok',
      duration_ms: 4200,
      ai_state: {
        conversation_id: 'c',
        final_sql: null,
        response: 'done',
        messages: [{ thinking: 'pondering' }],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    const status = container.querySelector(
      '.activity-status',
    ) as HTMLButtonElement;
    expect(status).toBeTruthy();
    expect(status.disabled).toBe(false);
    expect(container.querySelector('.activity-body')).toBeNull();

    // ThinkingCard shows itself collapsed, so its summary is the signal that
    // the record survived, not the thinking text itself.
    await fireEvent.click(status);
    expect(container.querySelector('.activity-body')?.textContent).toContain(
      'Thought for',
    );
  });

  it('states the run cost without offering a log that does not exist', () => {
    const model = aiModel({
      status: 'ok',
      duration_ms: 1200,
      ai_state: {
        conversation_id: 'c',
        final_sql: null,
        response: 'done',
        messages: [],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    const status = container.querySelector(
      '.activity-status',
    ) as HTMLButtonElement;
    expect(status.textContent).toContain('1.2s');
    expect(status.disabled).toBe(true);
  });

  it('shows a running status line while the cell runs', () => {
    const model = aiModel({
      status: 'running',
      ai_state: {
        conversation_id: 'c',
        final_sql: null,
        response: null,
        messages: [{ thinking: 'x' }],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.activity-status')?.textContent).toContain(
      'Thinking',
    );
  });
});
