import { confirm, open, save } from '@tauri-apps/plugin-dialog';
import { notebooks } from './notebooks.svelte.ts';

export const LUCENT_FILTER = {
  name: 'Lucent Notebook',
  extensions: ['lucent'],
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

export async function saveNotebookAs(tabId: string): Promise<string | null> {
  const model = notebooks.get(tabId);
  if (!model) return null;
  const suggested = model.filePath ?? 'Untitled.lucent';
  const path = await save({ defaultPath: suggested, filters: [LUCENT_FILTER] });
  if (!path) return null;
  return writeTo(tabId, path);
}

/** Save falls through to Save As for an untitled notebook. */
export async function saveNotebook(tabId: string): Promise<string | null> {
  const model = notebooks.get(tabId);
  if (!model) return null;
  if (!model.filePath) return saveNotebookAs(tabId);
  return writeTo(tabId, model.filePath);
}

export async function pickNotebookToOpen(): Promise<string | null> {
  const picked = await open({ multiple: false, filters: [LUCENT_FILTER] });
  return typeof picked === 'string' ? picked : null;
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
