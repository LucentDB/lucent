<script>
  import { onMount } from 'svelte';
  import { EditorView, basicSetup } from 'codemirror';
  import { sql } from '@codemirror/lang-sql';
  import { oneDark } from '@codemirror/theme-one-dark';
  import { Compartment } from '@codemirror/state';
  import { getTheme } from '../../stores/theme.svelte.js';

  let {
    onExecute,
    tabId = null,
    content = '',
    onContentChange = () => {},
  } = $props();
  let executing = $state(false);
  let error = $state(null);
  let container;
  let view;
  const themeStore = getTheme();
  const themeCompartment = new Compartment();

  function createEditor() {
    view = new EditorView({
      doc: content,
      extensions: [
        basicSetup,
        sql(),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            onContentChange(update.state.doc.toString());
          }
        }),
        themeCompartment.of(themeStore.current === 'dark' ? oneDark : []),
        EditorView.theme({
          '&': { height: '100%' },
          '.cm-scroller': { fontFamily: 'var(--font-mono)' },
          '.cm-content': { fontSize: '14px', padding: '12px 0' },
          '.cm-gutters': {
            borderRight: '1px solid var(--border)',
            background: 'var(--bg-surface)',
          },
          '.cm-activeLineGutter': { backgroundColor: 'var(--accent-soft)' },
          '.cm-activeLine': { backgroundColor: 'var(--accent-soft)' },
          '.cm-cursor': { borderLeftColor: 'var(--accent)' },
          '.cm-lineNumbers .cm-gutterElement': {
            color: 'var(--text-muted)',
            padding: '0 12px',
          },
        }),
      ],
      parent: container,
    });
  }

  $effect(() => {
    if (view) {
      view.dispatch({
        effects: themeCompartment.reconfigure(
          themeStore.current === 'dark' ? oneDark : [],
        ),
      });
    }
  });

  function getSQL() {
    return view ? view.state.doc.toString() : '';
  }

  async function handleExecute() {
    const sql = getSQL().trim();
    if (!sql) return;
    error = null;
    executing = true;
    try {
      await onExecute(sql);
    } catch (e) {
      error = typeof e === 'string' ? e : (e.message ?? 'Query failed');
    } finally {
      executing = false;
    }
  }

  function handleKeydown(e) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      handleExecute();
    }
  }

  // Sync editor content when switching tabs or when external SQL changes.
  $effect(() => {
    void tabId;
    if (view) {
      const current = view.state.doc.toString();
      if (current !== content) {
        view.dispatch({
          changes: { from: 0, to: current.length, insert: content },
        });
      }
    }
  });

  onMount(() => {
    createEditor();
    return () => {
      if (view) view.destroy();
    };
  });
</script>

<div class="query-editor" role="application" onkeydown={handleKeydown}>
  <div class="toolbar">
    <span class="shortcut-hint">⌘ + enter to run</span>
    <button class="run-btn" onclick={handleExecute} disabled={executing}>
      <span class="run-icon">▶</span>
      {executing ? 'Running...' : 'Run'}
    </button>
  </div>

  <div class="editor-container" bind:this={container}></div>

  {#if error}
    <div class="error-panel">
      <div class="error-header">
        <span class="error-icon">!</span>
        <span class="error-title">Query Failed</span>
      </div>
      <pre class="error-message">{error}</pre>
    </div>
  {/if}
</div>

<style>
  .query-editor {
    display: flex;
    flex-direction: column;
    border-bottom: 1px solid var(--border);
    background: var(--bg-surface);
  }
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-1) var(--space-3);
    border-bottom: 1px solid var(--border-light);
  }
  .shortcut-hint {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .run-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 16px;
    background: var(--text);
    color: var(--bg);
    border: none;
    border-radius: var(--radius-md);
    font-size: var(--text-base);
    font-weight: var(--weight-semibold);
    cursor: pointer;
  }
  .run-btn:hover:not(:disabled) {
    opacity: 0.9;
  }
  .run-btn:disabled {
    opacity: 0.5;
  }
  .run-icon {
    font-size: 10px;
  }
  .editor-container {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: var(--bg-surface);
  }
  .error-panel {
    border-top: 1px solid rgba(239, 68, 68, 0.3);
    background: rgba(239, 68, 68, 0.06);
  }
  .error-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) 0;
  }
  .error-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--danger);
    color: white;
    font-size: 11px;
    font-weight: var(--weight-bold);
    line-height: 1;
  }
  .error-title {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--danger);
  }
  .error-message {
    margin: var(--space-1) var(--space-3) var(--space-2);
    padding: var(--space-2);
    background: rgba(0, 0, 0, 0.04);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.6;
    color: var(--danger);
    white-space: pre-wrap;
    word-break: break-word;
    overflow-x: auto;
    max-height: 160px;
  }
  :global(.dark) .error-message {
    background: rgba(255, 255, 255, 0.04);
  }
</style>
