<script lang="ts">
  import { EditorView, basicSetup } from 'codemirror';
  import { oneDark } from '@codemirror/theme-one-dark';
  import { buildSqlExtension } from '../../editor/sql-schema-extension.ts';
  import { editorSchema } from '../../../stores/editor-schema.svelte.ts';
  import { connections } from '../../../stores/connections.svelte.ts';
  import {
    MatchDecorator,
    Decoration,
    ViewPlugin,
    keymap,
  } from '@codemirror/view';
  import { Compartment, Prec } from '@codemirror/state';
  import { untrack } from 'svelte';
  import RefPreview from '../RefPreview.svelte';
  import type { CellModel } from '../../../stores/notebook.svelte.ts';
  import type { NotebookModel } from '../../../stores/notebook.svelte.ts';
  import { getTheme } from '../../../stores/theme.svelte.js';

  let {
    source,
    status: _status,
    cells,
    cellId,
    model,
    focused = false,
    selected = false,
    onSourceChange,
    onEnterEdit,
  }: {
    source: string;
    status: string;
    cells?: CellModel[];
    cellId: string;
    model: NotebookModel;
    focused?: boolean;
    selected?: boolean;
    onSourceChange?: (val: string) => void;
    onEnterEdit?: () => void;
  } = $props();

  // container must be $state so the editor-creation effect re-runs when the
  // {#if shouldMount} branch renders this div and bind:this assigns it.
  let container: HTMLDivElement | undefined = $state();
  let view: EditorView | undefined;
  const themeStore = getTheme();
  const themeCompartment = new Compartment();
  const schemaCompartment = new Compartment();
  let isInternalChange = false;
  // Tracks whether we should focus the editor once it mounts (e.g. user clicked
  // the static placeholder before the CodeMirror instance existed).
  let pendingFocus = $state(false);

  let hostEl: HTMLDivElement | undefined = $state();
  let nearViewport = $state(false);
  // Mount when focused, selected, or within a viewport height of the visible area.
  let shouldMount = $derived(focused || selected || nearViewport);

  $effect(() => {
    if (!hostEl) return;
    if (typeof IntersectionObserver === 'undefined') {
      // jsdom and very old webviews: stay static rather than throwing.
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            nearViewport = true;
          } else if (!focused && model.mode !== 'edit') {
            // Hysteresis: mount at one viewport height, release beyond two, so
            // ordinary scrolling never thrashes mount/unmount.
            nearViewport = false;
          }
        }
      },
      { rootMargin: '100% 0px' },
    );
    io.observe(hostEl);
    return () => io.disconnect();
  });

  const cellRefMatcher = new MatchDecorator({
    regexp: /(\$\{[a-f0-9]{8}\}|\$[a-f0-9]{8}\.[a-z_][a-z0-9_]*)/gi,
    decoration: Decoration.mark({ class: 'cm-cell-ref-pill' }),
  });

  const cellRefPlugin = ViewPlugin.fromClass(
    class {
      decorations: ReturnType<typeof cellRefMatcher.createDeco>;
      constructor(view: EditorView) {
        this.decorations = cellRefMatcher.createDeco(view);
      }
      update(update: any) {
        this.decorations = cellRefMatcher.updateDeco(update, this.decorations);
      }
    },
    { decorations: (v: any) => v.decorations },
  );

  // Sync external source changes into CodeMirror.
  // Skip when the editor already matches — prevents focus loss during typing.
  $effect(() => {
    if (view && !isInternalChange) {
      const current = view.state.doc.toString();
      if (current !== source) {
        view.dispatch({
          changes: { from: 0, to: current.length, insert: source },
        });
      }
    }
  });

  $effect(() => {
    if (view) {
      view.dispatch({
        effects: themeCompartment.reconfigure(
          themeStore.current === 'dark' ? oneDark : [],
        ),
      });
    }
  });

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

  $effect(() => {
    if (!container || !shouldMount) return;
    const initialSource = untrack(() => source);
    const shouldFocusNow = untrack(() => pendingFocus || focused);
    view = new EditorView({
      doc: initialSource,
      extensions: [
        Prec.high(
          keymap.of([
            {
              key: 'Mod-Enter',
              preventDefault: true,
              run: () => {
                model.enterCommandMode();
                void model.runCell(cellId);
                return true;
              },
            },
            {
              key: 'Shift-Enter',
              preventDefault: true,
              run: () => {
                void model.runAndAdvance(cellId);
                return true;
              },
            },
            {
              key: 'Escape',
              preventDefault: true,
              run: (view) => {
                model.enterCommandMode();
                view.contentDOM.blur();
                return true;
              },
            },
          ]),
        ),
        basicSetup,
        schemaCompartment.of(
          buildSqlExtension({
            tables: editorSchema.tables,
            sqlDialect: connections.capabilities?.dialect,
          }),
        ),
        cellRefPlugin,
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            isInternalChange = true;
            onSourceChange?.(update.state.doc.toString());
            requestAnimationFrame(() => {
              isInternalChange = false;
            });
          }
        }),
        EditorView.domEventHandlers({
          focus: () => {
            model.select(cellId);
            model.enterEditMode();
            return false;
          },
        }),
        themeCompartment.of(themeStore.current === 'dark' ? oneDark : []),
        EditorView.theme({
          // Just enough for one line plus its padding. 60px left ~20px of dead
          // space under every short query.
          '&': { height: 'auto', minHeight: '40px' },
          '.cm-scroller': { fontFamily: 'var(--font-mono)' },
          '.cm-content': { fontSize: '14px', padding: '10px 0' },
          '.cm-gutters': {
            borderRight: '1px solid var(--border)',
            background: 'transparent',
            display: 'flex',
          },
          '.cm-activeLineGutter': { backgroundColor: 'transparent' },
          '.cm-activeLine': { backgroundColor: 'var(--accent-active-line)' },
          '.cm-cursor': { borderLeftColor: 'var(--accent)' },
          '.cm-lineNumbers .cm-gutterElement': {
            color: 'var(--text-muted)',
            padding: '0 8px',
          },
          '.cm-selectionBackground': {
            background: 'var(--accent-selection, #c7d2fe)',
          },
          '&.cm-focused .cm-selectionBackground': {
            background: 'var(--accent-selection, #c7d2fe)',
          },
          '&.cm-focused': { outline: 'none' },
          '.cm-cell-ref-pill': {
            background: 'var(--accent-soft)',
            borderRadius: '3px',
            padding: '0 3px',
            border: '1px solid var(--accent-muted)',
          },
        }),
      ],
      parent: container,
    });

    // Focus the editor if the user clicked into it before the instance existed.
    if (shouldFocusNow) {
      view.focus();
      pendingFocus = false;
    }

    return () => {
      view?.destroy();
      view = undefined;
    };
  });

  function focusStaticCell() {
    pendingFocus = true;
    model.select(cellId);
    model.enterEditMode();
    onEnterEdit?.();
  }

  function handleStaticKeydown(e: KeyboardEvent) {
    if (e.key !== 'Enter' && e.key !== ' ') return;
    e.preventDefault();
    focusStaticCell();
  }

  // Focus an already-mounted editor when the cell enters edit mode.
  $effect(() => {
    if (focused && view && !view.hasFocus) {
      view.focus();
    }
  });

  // Command mode must also remove focus from the previous cell. Without this,
  // Shift+Enter changes selection but leaves the old CodeMirror cursor active.
  $effect(() => {
    if (!focused && view?.hasFocus) {
      view.contentDOM.blur();
    }
  });
</script>

<div class="sql-cell" bind:this={hostEl}>
  {#if shouldMount}
    <div bind:this={container} class="editor-container"></div>
  {:else}
    <!-- Static stand-in: same font metrics, no editor instance. Clicking or
         scrolling into view upgrades it to CodeMirror. -->
    <div
      class="sql-static"
      role="button"
      tabindex="0"
      aria-label="Edit SQL cell"
      onclick={focusStaticCell}
      onkeydown={handleStaticKeydown}
    >
      {source || '-- Click to write SQL'}
    </div>
  {/if}
  <RefPreview {source} cells={cells ?? []} />
</div>

<style>
  .sql-cell {
    position: relative;
  }
  /* One line plus padding. The static stand-in must match the mounted editor
     exactly, or upgrading to CodeMirror visibly reflows the cell. */
  .editor-container {
    min-height: 40px;
    background: transparent;
  }
  .sql-static {
    margin: 0;
    padding: 10px 12px 10px 48px;
    min-height: 40px;
    font-family: var(--font-mono);
    font-size: 13px;
    line-height: 1.6;
    white-space: pre-wrap;
    color: var(--text-secondary);
    cursor: text;
  }
  .sql-static:hover {
    color: var(--text);
  }
</style>
