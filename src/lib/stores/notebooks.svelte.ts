import { SvelteMap } from 'svelte/reactivity';
import { createNotebookModel, type NotebookModel } from './notebook.svelte.ts';
import * as nb from '../ipc/notebook';

export interface AttachSpec {
  filePath: string | null;
  connectionId: string;
  database: string;
}

/**
 * One NotebookModel per notebook tab, keyed by tab id.
 *
 * The model cannot live in Notebook.svelte's initializer: Svelte reuses the
 * component instance across tab switches, so an initializer runs once and every
 * notebook tab would share one model. Keying the component would fix the sharing
 * but destroy the model — and its DB session — on every tab switch. Owning the
 * models here keeps sessions and outputs alive, and lets App.svelte read isDirty
 * for the tab bar and route File-menu commands to the active notebook.
 */
class NotebookRegistry {
  private models = new SvelteMap<string, NotebookModel>();
  /**
   * Tracks each tab's in-flight (fire-and-forget) attach call. `ensure` does not
   * await attach, so a `release` called immediately after `ensure` (e.g. opening
   * and instantly closing a tab) could otherwise run before `model.sessionKey`
   * is populated — `session.detach()` no-ops when `sessionKey` is unset, so the
   * backend session would never be detached and the DB session would leak.
   * `release` awaits this before detaching so it always sees the settled
   * `sessionKey`, whichever way attach resolved.
   */
  private pendingAttach = new Map<string, Promise<void>>();

  has(tabId: string): boolean {
    return this.models.has(tabId);
  }

  get(tabId: string): NotebookModel | undefined {
    return this.models.get(tabId);
  }

  /**
   * Synchronous and idempotent. Attach is fired but not awaited, so callers can
   * render immediately; cell execution already guards on sessionKey being set,
   * so a run attempted before attach lands fails with the existing clear error
   * rather than racing.
   */
  ensure(tabId: string, spec: AttachSpec): NotebookModel {
    const existing = this.models.get(tabId);
    if (existing) return existing;

    const model = createNotebookModel(spec.filePath ?? undefined);
    this.models.set(tabId, model);

    if (spec.connectionId) {
      const attachPromise = model.session
        .attach(spec.filePath, spec.connectionId, spec.database)
        .then(async () => {
          if (!spec.filePath) return;
          const file = await nb.notebookOpen(spec.filePath);
          model.loadFromFile(file);
        })
        .catch((e) => {
          console.error(`[notebook] open/attach failed for tab ${tabId}:`, e);
        })
        .finally(() => {
          this.pendingAttach.delete(tabId);
        });
      this.pendingAttach.set(tabId, attachPromise);
    }
    return model;
  }

  async release(tabId: string): Promise<void> {
    const model = this.models.get(tabId);
    if (!model) return;
    this.models.delete(tabId);
    // Let any in-flight attach settle first, so sessionKey is up to date and
    // detach doesn't no-op on a session that's about to attach.
    const pending = this.pendingAttach.get(tabId);
    if (pending) await pending;
    try {
      await model.session.detach();
    } catch (e) {
      console.error(`[notebook] detach failed for tab ${tabId}:`, e);
    }
  }
}

export const notebooks = new NotebookRegistry();
