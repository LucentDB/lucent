<script lang="ts">
  import { chat } from '../../stores/chat.svelte.ts';
  let { onSend }: { onSend: (msg: string) => void } = $props();
  let value = $state('');
  let inputEl: HTMLTextAreaElement;

  function handleKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      submit();
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  function submit() {
    const t = value.trim();
    if (!t || chat.isStreaming) return;
    onSend(t);
    value = '';
    if (inputEl) inputEl.style.height = 'auto';
  }

  function autoResize() {
    if (inputEl) {
      inputEl.style.height = 'auto';
      inputEl.style.height = Math.min(inputEl.scrollHeight, 160) + 'px';
    }
  }
</script>

<div class="chat-input">
  <div class="input-wrap" class:focused={false}>
    <textarea
      bind:this={inputEl}
      bind:value
      onkeydown={handleKeydown}
      oninput={autoResize}
      placeholder="Ask anything about your database…"
      disabled={chat.isStreaming}
      rows={1}></textarea>
    <button
      class="send-btn"
      onclick={submit}
      disabled={chat.isStreaming || !value.trim()}
    >
      {#if chat.isStreaming}
        <span class="spinner"></span>
      {:else}
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
          <path
            d="M1 8L15 1L8 15L6 10L1 8Z"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linejoin="round"
            fill="currentColor"
          />
        </svg>
      {/if}
    </button>
  </div>
  <div class="hint">Enter to send · Shift+Enter for newline</div>
</div>

<style>
  .chat-input {
    padding: 12px 20px 10px;
    border-top: 1px solid var(--border);
    background: var(--bg-surface);
  }
  .input-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: 4px;
    transition:
      border-color var(--transition-fast),
      box-shadow var(--transition-fast);
  }
  .input-wrap:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  textarea {
    flex: 1;
    resize: none;
    border: none;
    padding: 6px 10px;
    font-size: var(--text-md);
    line-height: 1.5;
    background: transparent;
    color: var(--text);
    max-height: 160px;
  }
  textarea:focus {
    outline: none;
  }
  textarea::placeholder {
    color: var(--text-muted);
  }
  textarea:disabled {
    opacity: 0.5;
  }
  .send-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-md);
    background: var(--accent);
    color: #fff;
    border: none;
    cursor: pointer;
    flex-shrink: 0;
    transition:
      opacity var(--transition-fast),
      transform var(--transition-fast);
  }
  .send-btn:hover:not(:disabled) {
    opacity: 0.9;
    transform: scale(1.05);
  }
  .send-btn:disabled {
    opacity: 0.3;
    cursor: not-allowed;
    transform: none;
  }
  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .hint {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-align: right;
    margin-top: 4px;
    padding-right: 4px;
  }
</style>
