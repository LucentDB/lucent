<script>
  import { EditorView, basicSetup } from 'codemirror';
  import { sql } from '@codemirror/lang-sql';
  import { oneDark } from '@codemirror/theme-one-dark';
  import { getTheme } from '../../stores/theme.svelte.js';

  let { title = '', source = '', loading = false, error = null } = $props();

  let container = $state(null);
  let view;
  const themeStore = getTheme();

  function createView(doc) {
    if (view) view.destroy();
    const extensions = [
      basicSetup,
      sql(),
      EditorView.editable.of(false),
      EditorView.theme({
        '&': { height: '100%' },
        '.cm-scroller': { fontFamily: 'var(--font-mono)' },
        '.cm-content': {
          fontSize: '13px',
          padding: '16px',
          caretColor: 'transparent',
        },
        '.cm-gutters': { display: 'none' },
        '.cm-activeLine': { backgroundColor: 'transparent' },
        '.cm-activeLineGutter': { backgroundColor: 'transparent' },
        '.cm-cursor': { visibility: 'hidden' },
        '.cm-line': { padding: '0' },
        '.cm-selectionBackground': {
          background: 'var(--accent-selection, #c7d2fe)',
        },
        '&.cm-focused .cm-selectionBackground': {
          background: 'var(--accent-selection, #c7d2fe)',
        },
      }),
    ];

    if (themeStore.current === 'dark') {
      extensions.push(oneDark);
    }

    view = new EditorView({
      doc: doc || '-- no source found',
      extensions,
      parent: container,
    });
  }

  $effect(() => {
    if (source && !loading && container) {
      createView(source);
    }
    return () => {
      if (view) view.destroy();
    };
  });
</script>

<div class="source-view">
  <div class="header">
    <span class="title">{title}</span>
    {#if loading}
      <span class="loading-badge">Loading...</span>
    {/if}
  </div>

  {#if loading}
    <div class="loading-state">Loading source...</div>
  {:else if error}
    <div class="error-state">{error}</div>
  {:else}
    <div class="editor-wrapper" bind:this={container}></div>
  {/if}
</div>

<style>
  .source-view {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--bg-surface);
  }
  .header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border);
  }
  .title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    font-family: var(--font-mono);
  }
  .loading-badge {
    font-size: 12px;
    color: var(--text-muted);
    font-style: italic;
  }
  .loading-state,
  .error-state {
    padding: 32px;
    text-align: center;
    font-size: 14px;
    color: var(--text-muted);
  }
  .error-state {
    color: var(--danger);
  }
  .editor-wrapper {
    flex: 1;
    overflow: auto;
    min-height: 200px;
  }
</style>
