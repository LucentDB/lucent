import { Channel } from '@tauri-apps/api/core';
import * as nb from '../ipc/notebook';
import type { NotebookModel, NotebookEvent } from './notebook.svelte.ts';

export function createNotebookSession(model: NotebookModel) {
  return {
    async attach(filePath: string | null, profileId: string, database: string) {
      const key = await nb.notebookAttach(filePath, profileId, database);
      model.sessionKey = key;
      return key;
    },

    async detach() {
      if (!model.sessionKey) return;
      await nb.notebookDetach(model.sessionKey);
      model.sessionKey = null;
    },

    async runCell(cellId: string) {
      if (!model.sessionKey) {
        const msg =
          'Notebook not attached — no session key. Run notebook_attach first.';
        console.error(`[notebook] runCell(${cellId}): ${msg}`);
        throw new Error(msg);
      }
      const cell = model.cells.find((c) => c.id === cellId);
      if (!cell) {
        console.error(
          `[notebook] runCell: cell ${cellId} not found in ${model.cells.length} cells`,
        );
        throw new Error('cell not found');
      }
      cell.status = 'running';
      cell.error = null;
      cell.run_token = (cell.run_token ?? 0) + 1;
      console.log(
        `[notebook] runCell(${cellId}): executing ${cell.kind} cell, sessionKey=${model.sessionKey.slice(0, 8)}...`,
      );

      const channel = new Channel<NotebookEvent>();
      // Track whether we're currently streaming a thinking phase.
      // When a tool_call arrives, thinking is finalized (streaming=false).
      // New thinking_chunk after that creates a NEW thinking message.
      let thinkingStreaming = false;
      // Track thinking phase timing for per-card duration display.
      let thinkingStartTime = Date.now();

      const outputPromise = nb.notebookRunCell(
        model.sessionKey,
        cellId,
        model.cells,
        channel,
      );

      channel.onmessage = (event: NotebookEvent) => {
        switch (event.type) {
          case 'thinking_started': {
            if (cell.ai_state && cell.ai_state.messages.length === 0) {
              cell.ai_state.messages = [];
            }
            thinkingStartTime = Date.now();
            break;
          }
          case 'thinking_chunk': {
            if (!cell.ai_state) break;
            const msgs = [...cell.ai_state.messages];
            const last = msgs.at(-1);
            const lastIsThinking =
              last &&
              typeof last === 'object' &&
              last !== null &&
              'thinking' in last &&
              typeof (last as Record<string, unknown>).thinking === 'string';

            if (thinkingStreaming && lastIsThinking) {
              msgs[msgs.length - 1] = {
                ...(last as Record<string, string>),
                thinking:
                  (last as Record<string, string>).thinking +
                  event.payload.chunk,
              };
            } else {
              msgs.push({
                thinking: event.payload.chunk,
                _startedAt: thinkingStartTime,
              });
              thinkingStreaming = true;
            }
            cell.ai_state.messages = msgs;
            break;
          }
          case 'thinking_done':
            thinkingStreaming = false;
            if (cell.ai_state && cell.ai_state.messages.length > 0) {
              cell.ai_state.messages = cell.ai_state.messages.map((m) => {
                if (
                  typeof m === 'object' &&
                  m !== null &&
                  'thinking' in m &&
                  '_startedAt' in m
                ) {
                  return {
                    thinking: (m as Record<string, string>).thinking,
                    durationMs:
                      Date.now() -
                      ((m as Record<string, number>)._startedAt ?? 0),
                  };
                }
                return m;
              });
            }
            break;
          case 'tool_call': {
            thinkingStreaming = false;
            if (cell.ai_state) {
              cell.ai_state.tool_calls = [
                ...cell.ai_state.tool_calls,
                event.payload.tool,
              ];
            }
            break;
          }
          case 'tool_result': {
            if (!cell.ai_state) break;
            const { id, summary, output } = event.payload;
            cell.ai_state.tool_calls = cell.ai_state.tool_calls.map((tc) =>
              (tc as Record<string, unknown>).id === id
                ? { ...(tc as Record<string, unknown>), summary, output }
                : tc,
            );
            break;
          }
          case 'sql_preview':
            break;
          case 'rows_streamed':
            break;
          case 'cell_done': {
            const p = event.payload;
            cell.outputs = p.output;
            cell.status = 'ok';
            cell.execution_order = p.execution_order;
            cell.duration_ms = p.duration_ms;
            cell.stale_since = null;
            model.cellView.resetFrom(p.cell_id);
            if (p.ai_state) {
              cell.ai_state = {
                ...p.ai_state,
                // Preserve locally accumulated data — thinking chunks and tool
                // results stream ahead of cell_done, and the backend's final
                // snapshot may lack the per-event detail the UI has already
                // rendered.
                messages:
                  cell.ai_state && cell.ai_state.messages.length > 0
                    ? cell.ai_state.messages
                    : p.ai_state.messages,
                tool_calls:
                  cell.ai_state && cell.ai_state.tool_calls.length > 0
                    ? cell.ai_state.tool_calls
                    : p.ai_state.tool_calls,
              };
            }
            break;
          }
          case 'cell_error': {
            const p = event.payload;
            cell.error = p.error;
            cell.status = 'error';
            break;
          }
        }
      };

      try {
        const result = await outputPromise;
        // Channel events (CellDone) update the cell for SQL/AI cells.
        // Markdown cells return output directly without a channel event —
        // apply the result as a fallback when the channel hasn't already.
        if (cell.status === 'running') {
          cell.outputs = result;
          cell.status = 'ok';
          cell.duration_ms = 0;
        }
        console.log(
          `[notebook] runCell(${cellId}): done, status=${cell.status}`,
        );
        return result;
      } catch (e) {
        const msg =
          e instanceof Error
            ? e.message
            : typeof e === 'string'
              ? e
              : JSON.stringify(e);
        console.error(`[notebook] runCell(${cellId}): FAILED — ${msg}`);
        cell.status = 'error';
        cell.error = { kind: 'queryError', message: msg, sql_error: '' };
        throw e;
      }
    },

    async cancelCell(cellId: string) {
      if (!model.sessionKey) throw new Error('not attached');
      return nb.notebookCancelCell(model.sessionKey, cellId);
    },

    async resolveRefs(cellId: string) {
      if (!model.sessionKey) throw new Error('not attached');
      return nb.notebookResolveRefs(model.sessionKey, cellId, model.cells);
    },

    async save(path: string) {
      if (!model.sessionKey) throw new Error('not attached');
      const newKey = await nb.notebookSave(
        model.sessionKey,
        path,
        model.metadata,
        model.cells,
      );
      model.sessionKey = newKey;
      model.filePath = path;
      model.markSaved();
    },

    async restart() {
      if (!model.sessionKey) throw new Error('not attached');
      await nb.notebookRestartSession(model.sessionKey);
      model.markSaved();
    },
  };
}
