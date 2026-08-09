import { createNotebookSession } from './notebook-session.ts';
import * as nb from '../ipc/notebook';
import { createCellView } from './notebook-view.ts';

export type CellKind = 'sql' | 'markdown' | 'ai';
export type CellStatus = 'pending' | 'running' | 'ok' | 'error' | 'stale';

export interface ColumnMeta {
  name: string;
  type_name: string;
}

export interface TableOutput {
  columns: ColumnMeta[];
  rows: unknown[][];
  total_count: number | null;
  is_truncated: boolean;
  page_size: number;
  is_wrappable: boolean;
  rows_affected?: number | null;
}

export interface TextOutput {
  content: string;
}

export type CellOutput = TableOutput | TextOutput;

export type CellError =
  | { kind: 'cyclicDependency'; cycle: string[]; hint: string }
  | { kind: 'notExecuted'; cell_id: string; hint: string }
  | { kind: 'textNotReferencable'; cell_id: string; message: string }
  | { kind: 'notATable'; cell_id: string; message: string }
  | { kind: 'notExecutable'; cell_id: string; message: string }
  | { kind: 'staleReference'; cell_id: string; hint: string }
  | { kind: 'unresolvedRef'; cell_id: string; ref_name: string; hint: string }
  | { kind: 'dmlNotReferencable'; cell_id: string; message: string }
  | { kind: 'queryError'; message: string; sql_error: string }
  | { kind: 'connectionLost'; message: string };

export interface AiCellState {
  conversation_id: string;
  final_sql: string | null;
  response: string | null;
  messages: unknown[];
  tool_calls: unknown[];
}

export interface CellModel {
  id: string;
  kind: CellKind;
  source: string;
  alias?: string;
  collapsed: boolean;
  outputs: CellOutput | null;
  status: CellStatus;
  execution_order: number | null;
  duration_ms: number | null;
  error: CellError | null;
  stale_since: number | null;
  ai_state: AiCellState | null;
  /**
   * Bumped when a run starts, so output views can default their tab once per
   * run. Optional: older literals (tests, file loads) omit it; reads use `?? 0`.
   */
  run_token?: number;
  /**
   * Grid view state — session-only. Deliberately absent from NotebookFileCell so
   * it can never be written to a .lucent file.
   */
  view?: import('./notebook-view.ts').CellViewState;
}

export interface NotebookMetadata {
  connectionId: string | null;
  connectionName: string | null;
  connectionHost: string | null;
  database: string | null;
  createdAt: string;
  updatedAt: string;
  lucentVersion: string;
}

/** Document fields only — persisted to disk. Session state (status, error,
 * staleSince, durationMs) is intentionally absent; see NotebookFileV2. */
export interface NotebookFileCell {
  id: string;
  kind: CellKind;
  source: string;
  alias?: string | null;
  collapsed: boolean;
  executionOrder: number | null;
  outputs: CellOutput | null;
  aiState: AiCellState | null;
}

export interface NotebookFileV2 {
  version: 2;
  metadata: NotebookMetadata;
  cells: NotebookFileCell[];
}

export interface ResolvedQuery {
  cte_chain: string[];
  final_sql: string;
  errors: CellError[];
}

export type ToolOutputPayload =
  | { type: 'text'; data: string }
  | {
      type: 'query_result';
      columns: { name: string; type: string }[];
      rows: unknown[][];
      row_count: number;
      sql: string;
      execution_time_ms: number;
      truncated: boolean;
    }
  | {
      type: 'dml_preview';
      sql: string;
      statement_type: string;
      tables_affected: string[];
      description: string;
      estimated_rows_affected: number | null;
    };

export type NotebookEvent =
  | { type: 'thinking_started'; payload: { cell_id: string } }
  | { type: 'thinking_chunk'; payload: { cell_id: string; chunk: string } }
  | { type: 'thinking_done'; payload: { cell_id: string; duration_ms: number } }
  | { type: 'tool_call'; payload: { cell_id: string; tool: unknown } }
  | {
      type: 'tool_result';
      payload: {
        cell_id: string;
        id: string;
        summary: string;
        output: ToolOutputPayload | null;
      };
    }
  | { type: 'sql_preview'; payload: { cell_id: string; sql: string } }
  | {
      type: 'rows_streamed';
      payload: { cell_id: string; rows: unknown[][]; is_end: boolean };
    }
  | {
      type: 'cell_done';
      payload: {
        cell_id: string;
        output: CellOutput;
        ai_state?: AiCellState | null;
        execution_order: number;
        duration_ms: number;
      };
    }
  | { type: 'cell_error'; payload: { cell_id: string; error: CellError } };

function defaultMetadata(): NotebookMetadata {
  return {
    connectionId: null,
    connectionName: null,
    connectionHost: null,
    database: null,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    lucentVersion: '0.1.0',
  };
}

export class NotebookModel {
  filePath = $state<string | null>(null);
  sessionKey = $state<string | null>(null);
  metadata = $state<NotebookMetadata>(defaultMetadata());
  runningCellId = $state<string | null>(null);

  /** Deeply reactive — Svelte tracks all property mutations automatically. */
  private _cells = $state<CellModel[]>([]);
  /** Bumped on source/structure changes only (not execution status/outputs). */
  private _sourceVersion = $state(0);
  private lastSaveSnapshot = $state<string>('');
  /**
   * Lazily-created session, memoised for the lifetime of this model. Not
   * `$state`: it's an imperative helper object (a bag of closures over
   * `this`) with no observable properties of its own — its methods mutate
   * reactive fields on the model directly, so Svelte's reactivity is already
   * driven by those field writes. Making the holder reactive would only add
   * a dependency-tracking cost for a value that never changes after first
   * access.
   */
  private _session: ReturnType<typeof createNotebookSession> | null = null;

  /**
   * Digest of document state only. Outputs are excluded deliberately: including
   * them meant re-serializing every result row on every keystroke, and running a
   * query is not an unsaved edit.
   */
  private _digest(): string {
    return JSON.stringify(
      this._cells.map((c) => [
        c.id,
        c.kind,
        c.source,
        c.alias ?? null,
        c.collapsed,
      ]),
    );
  }

  /** True when source/structure has changed since last save. */
  isDirty = $derived.by(() => {
    this._sourceVersion;
    return this._digest() !== this.lastSaveSnapshot;
  });

  /** One session per model — previously re-created on every call. */
  get session() {
    if (!this._session) this._session = createNotebookSession(this);
    return this._session;
  }

  private _cellView: ReturnType<typeof createCellView> | null = null;

  /** Per-cell grid view state helper — memoised like `session`. */
  get cellView() {
    if (!this._cellView) this._cellView = createCellView(this);
    return this._cellView;
  }

  get cells(): CellModel[] {
    return this._cells;
  }

  set cells(v: CellModel[]) {
    this._cells = v;
    this._sourceVersion++;
  }

  constructor(filePath?: string) {
    this.filePath = filePath ?? null;
    if (!filePath) {
      this.cells = [this._createCell('sql')];
    }
    this.lastSaveSnapshot = this._digest();
  }

  /** Set a cell's source and trigger downstream invalidation. */
  setCellSource(cellId: string, newSource: string): void {
    const cell = this._cells.find((c) => c.id === cellId);
    if (!cell || cell.source === newSource) return;
    cell.source = newSource;
    this._invalidateDownstreamOf(cellId);
    this._sourceVersion++;
  }

  private _createCell(kind: CellKind): CellModel {
    return {
      id: this._generateId(),
      kind,
      source: '',
      alias: undefined,
      collapsed: false,
      outputs: null,
      status: 'pending',
      execution_order: null,
      duration_ms: null,
      error: null,
      stale_since: null,
      run_token: 0,
      ai_state:
        kind === 'ai'
          ? {
              conversation_id: crypto.randomUUID(),
              final_sql: null,
              response: null,
              messages: [],
              tool_calls: [],
            }
          : null,
    };
  }

  private _generateId(): string {
    const chars = 'abcdef0123456789';
    let id = '';
    for (let i = 0; i < 8; i++)
      id += chars[Math.floor(Math.random() * chars.length)];
    return id;
  }

  addCell(afterId: string | null, kind: CellKind) {
    const idx = afterId ? this._cells.findIndex((c) => c.id === afterId) : -1;
    const newCell = this._createCell(kind);
    if (idx >= 0) {
      this.cells = [
        ...this._cells.slice(0, idx + 1),
        newCell,
        ...this._cells.slice(idx + 1),
      ];
    } else {
      this.cells = [...this._cells, newCell];
    }
  }

  deleteCell(id: string) {
    this.cells = this._cells.filter((c) => c.id !== id);
    this._invalidateDownstreamOf(id);
  }

  selectedCellId = $state<string | null>(null);
  /** Jupyter two-mode keyboard: command mode navigates, edit mode types. */
  mode = $state<'command' | 'edit'>('command');

  select(id: string | null): void {
    this.selectedCellId = id;
  }

  selectRelative(delta: number): void {
    const cells = this._cells;
    if (cells.length === 0) return;
    const current = cells.findIndex((c) => c.id === this.selectedCellId);
    const from = current < 0 ? 0 : current;
    const next = Math.max(0, Math.min(from + delta, cells.length - 1));
    this.selectedCellId = cells[next].id;
  }

  enterEditMode(): void {
    if (this.selectedCellId) this.mode = 'edit';
  }

  enterCommandMode(): void {
    this.mode = 'command';
  }

  /** Inserts relative to a cell, selects the new cell, and returns its id. */
  insertCell(
    relativeToId: string,
    position: 'above' | 'below',
    kind: CellKind,
  ): string {
    const idx = this._cells.findIndex((c) => c.id === relativeToId);
    const at =
      idx < 0 ? this._cells.length : position === 'below' ? idx + 1 : idx;
    const cell = this._createCell(kind);
    this.cells = [...this._cells.slice(0, at), cell, ...this._cells.slice(at)];
    this.selectedCellId = cell.id;
    return cell.id;
  }

  /**
   * Changes a cell's type in place. Identity and authored content survive; every
   * execution artefact is dropped, because an output produced as SQL means nothing
   * once the cell is Markdown.
   */
  convertCell(id: string, kind: CellKind): void {
    const idx = this._cells.findIndex((c) => c.id === id);
    if (idx < 0) return;
    const prev = this._cells[idx];
    if (prev.kind === kind) return;

    const next: CellModel = {
      ...prev,
      kind,
      outputs: null,
      status: 'pending',
      execution_order: null,
      duration_ms: null,
      error: null,
      stale_since: null,
      view: undefined,
      ai_state:
        kind === 'ai'
          ? {
              conversation_id: crypto.randomUUID(),
              final_sql: null,
              response: null,
              messages: [],
              tool_calls: [],
            }
          : null,
    };
    this.cells = [
      ...this._cells.slice(0, idx),
      next,
      ...this._cells.slice(idx + 1),
    ];
    // A converted cell's grid state describes the OLD kind's output — drop it.
    this.cellView.resetFrom(id);
  }

  /** Runs the selected cell then advances, appending a cell if it was the last. */
  async runAndAdvance(id: string): Promise<void> {
    const idx = this._cells.findIndex((c) => c.id === id);
    const isLast = idx === this._cells.length - 1;
    this.enterCommandMode();
    const run = this.runCell(id);
    if (isLast) {
      this.insertCell(id, 'below', 'sql');
    } else {
      this.selectedCellId = this._cells[idx + 1].id;
    }
    await run;
  }

  moveCell(id: string, delta: number) {
    const idx = this._cells.findIndex((c) => c.id === id);
    if (idx < 0) return;
    const newIdx = idx + delta;
    if (newIdx < 0 || newIdx >= this._cells.length) return;
    // Splice-and-insert: remove cell at idx, insert at newIdx.
    const arr = [...this._cells];
    const [moved] = arr.splice(idx, 1);
    arr.splice(newIdx, 0, moved);
    this.cells = arr;
    this.markAllCellsStale();
  }

  toggleCollapse(id: string) {
    const cell = this._cells.find((c) => c.id === id);
    if (!cell) return;
    cell.collapsed = !cell.collapsed;
    this._sourceVersion++;
  }

  markAllCellsStale() {
    const now = Date.now();
    this.cells = this._cells.map((c) =>
      c.status === 'ok'
        ? { ...c, status: 'stale' as const, stale_since: now }
        : c,
    );
  }

  markSaved(): void {
    this.lastSaveSnapshot = this._digest();
  }

  /** Replaces cell state from a parsed v2 file. Status is derived, never loaded. */
  loadFromFile(file: NotebookFileV2): void {
    this.metadata = file.metadata;
    this.cells = file.cells.map((fc) => ({
      id: fc.id,
      kind: fc.kind,
      source: fc.source,
      alias: fc.alias ?? undefined,
      collapsed: fc.collapsed,
      outputs: fc.outputs,
      status: fc.outputs ? ('ok' as const) : ('pending' as const),
      execution_order: fc.executionOrder,
      duration_ms: null,
      error: null,
      stale_since: null,
      run_token: 0,
      ai_state: fc.aiState,
    }));
    this.markSaved();
  }

  private _invalidateDownstreamOf(id: string) {
    const now = Date.now();
    const filtered = this._cells.map((c) => {
      if (c.status !== 'ok') return c;
      const refs = this._extractRefs(c.source);
      if (refs.some((r) => r.cellId === id)) {
        return { ...c, status: 'stale' as const, stale_since: now };
      }
      return c;
    });
    this.cells = filtered;
  }

  private _extractRefs(source: string): { cellId: string; column?: string }[] {
    const re = /\$\{([a-f0-9]{8})\}|\$([a-f0-9]{8})\.([a-z_][a-z0-9_]*)/g;
    const refs: { cellId: string; column?: string }[] = [];
    let match;
    while ((match = re.exec(source)) !== null) {
      if (match[1]) {
        refs.push({ cellId: match[1] });
      } else {
        refs.push({ cellId: match[2], column: match[3] });
      }
    }
    return refs;
  }

  /**
   * A cell with no statement in it has nothing to execute. Running it anyway
   * burns an execution counter and stores the backend's empty result envelope,
   * which the UI then renders as a "no rows found" panel on a cell the user
   * never typed into. Silently skipped, not an error.
   */
  private _isBlank(cell: CellModel): boolean {
    return cell.source.trim().length === 0;
  }

  async runCell(id: string): Promise<void> {
    const cell = this._cells.find((c) => c.id === id);
    if (cell && this._isBlank(cell)) return;
    await this.session.runCell(id);
  }

  runAllProgress = $state<{ current: number; total: number } | null>(null);

  async runAll(continueOnError = false): Promise<void> {
    const runnable = this._cells.filter(
      (c) => c.kind !== 'markdown' && !this._isBlank(c),
    );
    this.runAllProgress = { current: 0, total: runnable.length };
    try {
      for (let i = 0; i < runnable.length; i++) {
        const cell = runnable[i];
        this.runAllProgress = { current: i + 1, total: runnable.length };
        this.runningCellId = cell.id;
        try {
          await this.session.runCell(cell.id);
        } catch (e) {
          if (!continueOnError) throw e;
        }
      }
    } finally {
      this.runningCellId = null;
      this.runAllProgress = null;
    }
  }

  async cancelCell(id: string): Promise<void> {
    const cell = this._cells.find((c) => c.id === id);
    await this.session.cancelCell(id);
    if (cell) {
      cell.status = 'stale';
      cell.error = null;
    }
  }

  async clearOutputs(): Promise<void> {
    this.cells = this._cells.map((c) => ({
      ...c,
      outputs: null,
      status: 'pending' as const,
      execution_order: null,
      duration_ms: null,
      error: null,
      stale_since: null,
      ai_state:
        c.kind === 'ai'
          ? {
              conversation_id: crypto.randomUUID(),
              final_sql: null,
              response: null,
              messages: [],
              tool_calls: [],
            }
          : null,
    }));
    if (this.sessionKey) {
      await nb.notebookClearOutputs(this.sessionKey);
    }
  }

  async restartSession(): Promise<void> {
    await this.clearOutputs();
    await this.session.restart();
  }
}

export function createNotebookModel(filePath?: string): NotebookModel {
  return new NotebookModel(filePath);
}
