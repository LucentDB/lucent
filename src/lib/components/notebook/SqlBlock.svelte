<script lang="ts">
  import { tokenizeSql } from '../../utils/sql-highlight.ts';

  let { code, maxHeight = '420px' }: { code: string; maxHeight?: string } =
    $props();

  let tokens = $derived(tokenizeSql(code));
</script>

<!-- prettier-ignore -->
<pre class="sql-block" style="--sql-max-height: {maxHeight}"><code>{#each tokens as token}{#if token.cls}<span class={token.cls}>{token.text}</span>{:else}{token.text}{/if}{/each}</code></pre>

<style>
  .sql-block {
    margin: 0;
    padding: 12px 14px;
    font-family: var(--font-mono);
    font-size: 12.5px;
    line-height: 1.65;
    color: var(--text);
    background: var(--bg-subtle);
    max-height: var(--sql-max-height);
    overflow: auto;
    tab-size: 2;
    -moz-tab-size: 2;
  }

  /* Token classes come from Lezer's classHighlighter at runtime, so Svelte
     cannot match them statically — hence :global, scoped under .sql-block. */
  .sql-block :global(.tok-keyword),
  .sql-block :global(.tok-modifier),
  .sql-block :global(.tok-operatorKeyword) {
    color: var(--syn-keyword);
    font-weight: var(--weight-medium);
  }
  .sql-block :global(.tok-string),
  .sql-block :global(.tok-string2),
  .sql-block :global(.tok-character) {
    color: var(--syn-string);
  }
  .sql-block :global(.tok-number),
  .sql-block :global(.tok-bool),
  .sql-block :global(.tok-null) {
    color: var(--syn-number);
  }
  .sql-block :global(.tok-comment),
  .sql-block :global(.tok-lineComment),
  .sql-block :global(.tok-blockComment) {
    color: var(--syn-comment);
    font-style: italic;
  }
  .sql-block :global(.tok-typeName),
  .sql-block :global(.tok-className),
  .sql-block :global(.tok-standard) {
    color: var(--syn-type);
  }
  .sql-block :global(.tok-variableName),
  .sql-block :global(.tok-propertyName) {
    color: var(--syn-variable);
  }
  .sql-block :global(.tok-operator),
  .sql-block :global(.tok-punctuation) {
    color: var(--syn-operator);
  }
</style>
