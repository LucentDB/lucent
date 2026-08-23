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
        // Every closed segment carries its own duration: the live stream
        // stamps one per segment and the backend's restored snapshot stamps
        // the run's. A card no longer borrows `cell.duration_ms`, which is
        // what made every segment claim the whole run's time.
        messages: [{ thinking: 'pondering', durationMs: 4200 }],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    const status = container.querySelector(
      '.work-summary',
    ) as HTMLButtonElement;
    expect(status).toBeTruthy();
    expect(status.disabled).toBe(false);
    expect(container.querySelector('.activity-body')).toBeNull();

    // ThinkingCard shows itself collapsed, so its summary is the signal that
    // the record survived, not the thinking text itself.
    await fireEvent.click(status);
    expect(container.querySelector('.activity-body')?.textContent).toContain(
      'Thought for 4s',
    );
  });

  it('states the run cost without offering a log that does not exist', () => {
    // The duration is run metadata and stays in the header; a cell that did no
    // work gets no work row at all rather than an empty one.
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
    expect(container.querySelector('.run-duration')?.textContent).toContain(
      '1.2s',
    );
    expect(container.querySelector('.work-strip')).toBeNull();
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
    expect(container.querySelector('.work-summary')?.textContent).toContain(
      'Thinking',
    );
    expect(container.querySelector('.pulse')).toBeTruthy();
  });
});

describe('AiCellOutput work summary', () => {
  function ranAQuery(overrides: Record<string, unknown> = {}) {
    return aiModel({
      status: 'ok',
      duration_ms: 68_000,
      ai_state: {
        conversation_id: 'c',
        final_sql: 'SELECT 1',
        response: 'done',
        messages: [{ thinking: 'pondering', durationMs: 25_000 }],
        tool_calls: [
          { id: 't1', name: 'run_readonly_query', args: { sql: 'SELECT 1' } },
        ],
        ...(overrides.ai_state as object),
      },
      ...overrides,
    });
  }

  it('names the work instead of counting tool calls', () => {
    const model = ranAQuery();
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    const summary = container.querySelector('.work-summary')?.textContent ?? '';
    expect(summary).toContain('Ran 1 query');
    expect(summary).toContain('thought for 25s');
    expect(summary).not.toContain('tool call');
  });

  it('reads from the left, on its own row below the tabs', () => {
    const model = ranAQuery();
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    // Not inside the header, which is where the right-aligned pill used to sit.
    expect(container.querySelector('.output-header .work-summary')).toBeNull();
    expect(container.querySelector('.work-strip .work-summary')).toBeTruthy();
    const header = container.querySelector('.output-header');
    const strip = container.querySelector('.work-strip');
    expect(strip?.previousElementSibling).toBe(header);
  });

  it('keeps the run duration out of the work row', () => {
    const model = ranAQuery();
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.work-summary')?.textContent).not.toContain(
      '68',
    );
    expect(container.querySelector('.run-duration')?.textContent).toContain(
      '68.0s',
    );
  });

  it('expands the step log from the work row', async () => {
    const model = ranAQuery();
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.activity-body')).toBeNull();
    await fireEvent.click(container.querySelector('.work-summary')!);
    expect(container.querySelector('.activity-body')).toBeTruthy();
  });
});

describe('AiCellOutput thinking durations', () => {
  it('gives each thinking segment its own duration', async () => {
    // The regression: every card fell back to `cell.duration_ms`, so a
    // one-sentence closing thought claimed the whole run's time.
    const model = aiModel({
      status: 'ok',
      duration_ms: 89_000,
      ai_state: {
        conversation_id: 'c1',
        final_sql: null,
        response: 'done',
        messages: [
          { thinking: 'a long analysis', durationMs: 80_000 },
          { thinking: 'I have the data.', durationMs: 2_000 },
        ],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    await fireEvent.click(container.querySelector('.work-summary')!);
    const labels = [...container.querySelectorAll('.tc-label')].map((l) =>
      (l.textContent ?? '').trim(),
    );
    expect(labels).toEqual(['Thought for 80s', 'Thought for 2s']);
    expect(labels.filter((l) => l.includes('89'))).toHaveLength(0);
  });

  it('renders a segment with no duration as still thinking', async () => {
    const model = aiModel({
      status: 'running',
      duration_ms: null,
      ai_state: {
        conversation_id: 'c1',
        final_sql: null,
        response: null,
        messages: [{ thinking: 'working on it' }],
        tool_calls: [],
      },
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    expect(container.textContent).toContain('Thinking…');
  });
});

describe('AiCellOutput tool cards', () => {
  const queryCall = {
    id: 'tc1',
    name: 'run_readonly_query',
    args: { sql: 'SELECT 1 AS n' },
    summary: '3 rows',
    output: {
      type: 'query_result',
      columns: [{ name: 'n', type: 'int4' }],
      rows: [[1], [2], [3]],
      row_count: 3,
      sql: 'SELECT 1 AS n',
      execution_time_ms: 4,
      truncated: false,
    },
  };

  function withCall(call: Record<string, unknown>) {
    return aiModel({
      status: 'ok',
      duration_ms: 1_200,
      ai_state: {
        conversation_id: 'c1',
        final_sql: 'SELECT 1 AS n',
        response: 'here you go',
        messages: [],
        tool_calls: [call],
      },
    });
  }

  it('shows the arguments and the result grid, not a bare null', async () => {
    const model = withCall(queryCall);
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    await fireEvent.click(container.querySelector('.work-summary')!);
    await fireEvent.click(container.querySelector('.tcc-hdr')!);
    const body = container.querySelector('.tcc-body')!;
    expect(body.textContent).toContain('SELECT 1 AS n');
    expect(body.textContent).not.toContain('null');
    expect(body.querySelector('.tcc-table')).toBeTruthy();
    expect(body.textContent).toContain('4ms');
  });

  it('names the tool rather than the shell command that invoked it', async () => {
    // An ACP agent on the CLI path announces a shell command; the backend
    // backfills the real tool name and arguments from the bridge.
    const model = withCall(queryCall);
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    await fireEvent.click(container.querySelector('.work-summary')!);
    const name = container.querySelector('.tcc-name')!;
    expect(name.textContent).toBe('run readonly query');
  });

  it('keeps the card header short when the summary is a table preview', async () => {
    const model = withCall({
      ...queryCall,
      summary: 'Query: SELECT 1\nResult: 3 rows\n\n| n |\n|---|\n| 1 |',
    });
    const { container } = render(AiCellOutput, {
      props: { cell: model.cells[0], model },
    });
    await fireEvent.click(container.querySelector('.work-summary')!);
    const status = container.querySelector('.tcc-status')!;
    expect(status.textContent).toBe('Query: SELECT 1');
  });
});
