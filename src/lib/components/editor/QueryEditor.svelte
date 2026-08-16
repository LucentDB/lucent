<script>
  import { onMount } from 'svelte';
  import { EditorView, basicSetup } from 'codemirror';
  import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
  import { oneDarkTheme } from '@codemirror/theme-one-dark';
  import { tags } from '@lezer/highlight';
  import { Compartment } from '@codemirror/state';
  import { getTheme } from '../../stores/theme.svelte.js';
  import { buildSqlExtension } from './sql-schema-extension.ts';
  import { editorSchema } from '../../stores/editor-schema.svelte.ts';
  import { connections } from '../../stores/connections.svelte.ts';

  let {
    onExecute,
    tabId = null,
    content = '',
    onContentChange = () => {},
    isRunning = false,
    onCancel = () => {},
  } = $props();
  let executing = $state(false);
  let error = $state(null);
  let container;
  let view;
  const themeStore = getTheme();
  const themeCompartment = new Compartment();
  const schemaCompartment = new Compartment();

  // One Dark's default violet is legible on its own background, but becomes
  // muddy against Lucent's darker surfaces. Reuse the app syntax tokens so the
  // editor and read-only SQL blocks share one high-contrast palette.
  const darkSqlHighlightStyle = HighlightStyle.define([
    { tag: tags.keyword, color: 'var(--syn-keyword)', fontWeight: '600' },
    {
      tag: [tags.name, tags.variableName, tags.propertyName],
      color: 'var(--syn-variable)',
    },
    {
      tag: [tags.typeName, tags.className, tags.standard(tags.name)],
      color: 'var(--syn-type)',
    },
    {
      tag: [
        tags.string,
        tags.character,
        tags.special(tags.string),
      ],
      color: 'var(--syn-string)',
    },
    {
      tag: [tags.number, tags.bool, tags.null, tags.atom],
      color: 'var(--syn-number)',
    },
    {
      tag: [tags.comment, tags.lineComment, tags.blockComment, tags.docComment],
      color: 'var(--syn-comment)',
      fontStyle: 'italic',
    },
    {
      tag: [
        tags.operator,
        tags.operatorKeyword,
        tags.punctuation,
        tags.separator,
      ],
      color: 'var(--syn-operator)',
    },
  ]);
  const darkSqlTheme = [
    oneDarkTheme,
    syntaxHighlighting(darkSqlHighlightStyle),
  ];

  function createEditor() {
    view = new EditorView({
      doc: content,
      extensions: [
        basicSetup,
        schemaCompartment.of(
          buildSqlExtension({
            tables: editorSchema.tables,
            sqlDialect: connections.capabilities?.dialect,
          }),
        ),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            onContentChange(update.state.doc.toString());
          }
        }),
        themeCompartment.of(themeStore.current === 'dark' ? darkSqlTheme : []),
        EditorView.theme({
          '&': {
            height: '100%',
            color: 'var(--text)',
            backgroundColor: 'var(--bg-surface)',
          },
          '.cm-scroller': {
            fontFamily: 'var(--font-mono)',
            backgroundColor: 'var(--bg-surface)',
          },
          '.cm-content': {
            fontSize: '14px',
            padding: '12px 0',
            color: 'var(--text)',
          },
          '.cm-gutters': {
            borderRight: '1px solid var(--border)',
            backgroundColor: 'var(--bg-surface)',
          },
          '.cm-activeLineGutter': { backgroundColor: 'transparent' },
          '.cm-activeLine': { backgroundColor: 'var(--accent-active-line)' },
          '.cm-cursor': { borderLeftColor: 'var(--accent)' },
          '.cm-lineNumbers .cm-gutterElement': {
            color: 'var(--text-muted)',
            padding: '0 12px',
          },
          '.cm-selectionBackground': {
            background: 'var(--accent-selection, #c7d2fe)',
          },
          '&.cm-focused .cm-selectionBackground': {
            background: 'var(--accent-selection, #c7d2fe)',
          },
          '&.cm-focused': { outline: 'none' },
        }),
      ],
      parent: container,
    });
  }

  $effect(() => {
    // Read the theme before the view guard so changes made before mount still
    // invalidate this effect and changes after mount reconfigure the editor.
    const currentTheme = themeStore.current;
    if (view) {
      view.dispatch({
        effects: themeCompartment.reconfigure(
          currentTheme === 'dark' ? darkSqlTheme : [],
        ),
      });
    }
  });

  // Rebuilds the schema-fed sql() extension whenever the editor-schema store
  // or the connection's dialect changes, in place — an in-progress edit or
  // cursor position is never disturbed by a schema refresh.
  $effect(() => {
    if (view) {
      view.dispatch({
        effects: schemaCompartment.reconfigure(
          buildSqlExtension({
            tables: editorSchema.tables,
            sqlDialect: connections.capabilities?.dialect,
          }),
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
    <div class="toolbar-actions">
      <button
        class="toolbar-btn"
        onclick={() => editorSchema.refresh()}
        title="Refresh table/column list for autocomplete"
        aria-label="Refresh schema"
        type="button">↻</button
      >
      {#if isRunning}
        <button
          class="toolbar-btn stop"
          onclick={onCancel}
          title="Cancel query (Esc)"
          type="button">Stop</button
        >
      {/if}
      <button
        class="run-btn"
        onclick={handleExecute}
        disabled={executing}
        type="button"
      >
        <span class="run-icon">▶</span>
        {executing ? 'Running...' : 'Run'}
      </button>
    </div>
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
  .toolbar-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .shortcut-hint {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .toolbar-btn {
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    color: var(--text-secondary);
    cursor: pointer;
  }
  .toolbar-btn:hover {
    background: var(--bg-hover);
  }
  /* Stop button — always visible while running, styled like CellToolbar's stop. */
  .toolbar-btn.stop {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 6px 12px;
    border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    color: var(--danger);
    font-size: var(--text-base);
    font-weight: var(--weight-medium);
    cursor: pointer;
  }
  .toolbar-btn.stop:hover {
    background: var(--danger-bg);
    color: var(--danger);
  }
  .run-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 16px;
    background: var(--accent);
    color: var(--accent-foreground);
    border: none;
    border-radius: var(--radius-md);
    font-size: var(--text-base);
    font-weight: var(--weight-semibold);
    cursor: pointer;
    transition:
      background var(--transition-fast),
      box-shadow var(--transition-fast);
  }
  .run-btn:hover:not(:disabled) {
    background: var(--accent-hover);
    box-shadow: var(--shadow-sm);
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
  /* One Dark injects a fixed dark gutter rule. Keep the line-number rail on
     Lucent's surface in both themes, including after a live theme switch. */
  :global(.query-editor .cm-gutters) {
    background-color: var(--bg-surface) !important;
    color: var(--text-muted);
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
