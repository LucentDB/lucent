import type { NotebookModel } from '../../stores/notebook.svelte.ts';

export interface KeyEventLike {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

/** Window in which a second `d` completes the delete chord. */
export const DELETE_CHORD_MS = 500;

/**
 * Command-mode key dispatch, as a closure over the model rather than a component
 * method, so the whole table is unit-testable without a DOM. Returns true when the
 * key was consumed and the caller should preventDefault.
 *
 * Run bindings (Cmd+Enter, Shift+Enter) are handled here for command mode and
 * separately in CodeMirror's keymap for edit mode — CodeMirror consumes keydown
 * before it reaches the document, so one handler cannot cover both.
 */
export function createCommandKeymap(model: NotebookModel) {
  let pendingDeleteAt = 0;

  return function handle(e: KeyEventLike): boolean {
    const id = model.selectedCellId;
    if (!id) return false;

    const mod = e.metaKey || e.ctrlKey;

    // Run bindings are the only ones that take a modifier.
    if (e.key === 'Enter' && mod) {
      void model.runCell(id);
      return true;
    }
    if (e.key === 'Enter' && e.shiftKey) {
      void model.runAndAdvance(id);
      return true;
    }
    // Everything below is unmodified; never swallow app shortcuts like Cmd+K.
    if (mod || e.altKey) return false;

    const isD = e.key === 'd' || e.key === 'D';
    if (!isD) pendingDeleteAt = 0;

    switch (e.key) {
      case 'Enter':
        model.enterEditMode();
        return true;
      case 'ArrowDown':
      case 'j':
        model.selectRelative(1);
        return true;
      case 'ArrowUp':
      case 'k':
        model.selectRelative(-1);
        return true;
      case 'a':
      case 'A':
        model.insertCell(id, 'above', 'sql');
        return true;
      case 'b':
      case 'B':
        model.insertCell(id, 'below', 'sql');
        return true;
      case 'y':
      case 'Y':
        model.convertCell(id, 'sql');
        return true;
      case 'i':
      case 'I':
        model.convertCell(id, 'ai');
        return true;
      case 'm':
      case 'M':
        model.convertCell(id, 'markdown');
        return true;
      case ' ':
        model.toggleCollapse(id);
        return true;
      case 'd':
      case 'D': {
        const now = Date.now();
        if (pendingDeleteAt && now - pendingDeleteAt <= DELETE_CHORD_MS) {
          pendingDeleteAt = 0;
          const idx = model.cells.findIndex((c) => c.id === id);
          model.deleteCell(id);
          const next = model.cells[Math.min(idx, model.cells.length - 1)];
          model.select(next ? next.id : null);
        } else {
          pendingDeleteAt = now;
        }
        return true;
      }
      default:
        return false;
    }
  };
}
