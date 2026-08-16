import { confirm } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';

export const SQL_FILTER = { name: 'SQL File', extensions: ['sql'] };

export interface QueryTabSaveTarget {
  filePath?: string | null;
  name: string;
  baseSql: string;
}

async function writeQueryFile(path: string, content: string): Promise<void> {
  try {
    await invoke('save_sql_file', { path, content });
  } catch (e) {
    console.error(`[query] save failed for ${path}:`, e);
    await confirm(
      `Could not save the query.\n\n${e instanceof Error ? e.message : String(e)}`,
      { title: 'Save failed', kind: 'error' },
    );
    throw e;
  }
}

/**
 * Pick a save destination through the Rust-side native dialog. The returned
 * path is already approved (recorded in the backend's approved-path set), so
 * the subsequent `save_sql_file` write passes validation.
 */
async function chooseQuerySavePath(
  tab: QueryTabSaveTarget,
): Promise<string | null> {
  const path = await invoke<string | null>('choose_save_path', {
    defaultName: tab.name.endsWith('.sql') ? tab.name : `${tab.name}.sql`,
    filterName: 'SQL File',
    extensions: ['sql'],
  });
  return path;
}

/** Save falls through to Save As for a query with no filePath yet. */
export async function saveQueryTab(
  tab: QueryTabSaveTarget,
): Promise<string | null> {
  if (tab.filePath) {
    await writeQueryFile(tab.filePath, tab.baseSql);
    return tab.filePath;
  }
  return saveQueryTabAs(tab);
}

export async function saveQueryTabAs(
  tab: QueryTabSaveTarget,
): Promise<string | null> {
  const path = await chooseQuerySavePath(tab);
  if (!path) return null;
  await writeQueryFile(path, tab.baseSql);
  return path;
}
