import { confirm } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { notebooks } from './notebooks.svelte.ts';

export const LUCENT_FILTER = {
  name: 'Lucent Notebook',
  extensions: ['lucent'],
};

/** Combined open-dialog filter: notebooks AND .sql sources can be opened. */
export const NOTEBOOK_OPEN_FILTER = {
  name: 'Lucent Notebook / SQL File',
  extensions: ['lucent', 'sql'],
};

async function writeTo(tabId: string, path: string): Promise<string | null> {
  const model = notebooks.get(tabId);
  if (!model) return null;
  try {
    await model.session.save(path);
    // sessionKey is the backend's canonical answer (notebook_save re-keys the
    // session to the path it was given); filePath keeps the caller's spelling.
    // In production they coincide — sessionKey is the identity that matters.
    return model.sessionKey;
  } catch (e) {
    console.error(`[notebook] save failed for ${path}:`, e);
    await confirm(
      `Could not save the notebook.\n\n${e instanceof Error ? e.message : String(e)}`,
      { title: 'Save failed', kind: 'error' },
    );
    return null;
  }
}

/**
 * Pick a save destination through the Rust-side native dialog. The returned
 * path is already approved (recorded in the backend's approved-path set), so
 * the subsequent `notebook_save` write passes validation — the frontend is an
 * untrusted boundary and never supplies a raw write path itself.
 */
async function chooseNotebookSavePath(model: {
  filePath?: string | null;
}): Promise<string | null> {
  const suggested = model.filePath?.split('/').pop() ?? 'Untitled.lucent';
  return invoke<string | null>('choose_save_path', {
    defaultName: suggested.endsWith('.lucent')
      ? suggested
      : `${suggested}.lucent`,
    filterName: LUCENT_FILTER.name,
    extensions: LUCENT_FILTER.extensions,
  });
}

export async function saveNotebookAs(tabId: string): Promise<string | null> {
  const model = notebooks.get(tabId);
  if (!model) return null;
  const path = await chooseNotebookSavePath(model);
  if (!path) return null;
  return writeTo(tabId, path);
}

/**
 * Save falls through to Save As for an untitled notebook — and for a notebook
 * imported from a `.sql` file: its `filePath` still points at the user's SQL
 * source, and writing Lucent JSON over it would silently destroy the file.
 * Routing to Save As forces a `.lucent` target via the dialog instead.
 */
export async function saveNotebook(tabId: string): Promise<string | null> {
  const model = notebooks.get(tabId);
  if (!model) return null;
  if (!model.filePath) return saveNotebookAs(tabId);
  if (model.filePath.toLowerCase().endsWith('.sql'))
    return saveNotebookAs(tabId);
  return writeTo(tabId, model.filePath);
}

/**
 * Pick a notebook to open through the Rust-side native dialog. The picked
 * path is recorded in the approved set (a native open dialog is a user
 * choice, same trust level as Save-As), so saving an opened file back to the
 * same path passes the notebook_save gate.
 */
export async function pickNotebookToOpen(): Promise<string | null> {
  return invoke<string | null>('choose_open_path', {
    filterName: NOTEBOOK_OPEN_FILTER.name,
    extensions: NOTEBOOK_OPEN_FILTER.extensions,
  });
}

/**
 * Closing a dirty notebook must not silently discard work. Returns the user's
 * intent; the caller performs the close (or aborts).
 */
export async function confirmDiscardIfDirty(
  tabId: string,
): Promise<'save' | 'discard' | 'cancel'> {
  const model = notebooks.get(tabId);
  if (!model || !model.isDirty) return 'discard';

  const name = model.filePath?.split('/').pop() ?? 'Untitled.lucent';
  // plugin-dialog's confirm is two-button, so this is a save/discard choice with
  // Cancel reachable by dismissing. A three-button native ask is not available.
  const shouldSave = await confirm(
    `"${name}" has unsaved changes.\n\nSave before closing?`,
    {
      title: 'Unsaved changes',
      kind: 'warning',
      okLabel: 'Save',
      cancelLabel: 'Discard',
    },
  );
  return shouldSave ? 'save' : 'discard';
}
