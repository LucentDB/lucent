<script lang="ts">
  let { code, lang = '' }: { code: string; lang?: string } = $props();
  let copied = $state(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      /* ignore */
    }
  }
</script>

<div class="code-block-wrap">
  <div class="code-block-hdr">
    <span class="code-lang">{lang || 'code'}</span>
    <button class="copy-btn" onclick={copy}
      >{copied ? 'Copied!' : 'Copy'}</button
    >
  </div>
  <pre class="code-block"><code>{code.trimEnd()}</code></pre>
</div>

<style>
  .code-block-wrap {
    margin: 10px 0;
    border-radius: var(--radius-md);
    overflow: hidden;
    border: 1px solid var(--border);
  }
  .code-block-hdr {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 12px;
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border);
    font-size: var(--text-xs);
  }
  .code-lang {
    color: var(--text-muted);
    text-transform: uppercase;
    font-weight: var(--weight-semibold);
    letter-spacing: 0.05em;
  }
  .copy-btn {
    background: none;
    border: 1px solid var(--border);
    padding: 2px 10px;
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    color: var(--text-secondary);
    cursor: pointer;
    transition: all var(--transition-fast);
  }
  .copy-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .code-block {
    margin: 0;
    padding: 14px 16px;
    overflow-x: auto;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: 1.6;
    background: var(--bg-surface);
  }
  .code-block code {
    font-family: inherit;
  }
</style>
