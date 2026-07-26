<script lang="ts">
  import ChatInput from './ChatInput.svelte';
  import Icon from '../icons/Icon.svelte';
  let { onSend }: { onSend: (msg: string) => void } = $props();

  const suggestions = [
    { label: 'Show recent orders with user details', icon: 'chart' },
    { label: 'What tables track user activity?', icon: 'search' },
    { label: 'Delete old cancelled orders', icon: 'clean' },
    { label: 'Which tables have the most rows?', icon: 'trending' },
  ];
</script>

<div class="landing">
  <div class="hero">
    <div class="hero-icon"><Icon name="star" size={20} /></div>
    <h1>AI Copilot</h1>
    <p>
      Ask questions, run queries, and manage your database with natural
      language.
    </p>
  </div>

  <div class="input-wrap">
    <ChatInput {onSend} />
  </div>

  <div class="suggestions">
    <span class="suggest-label">Try asking:</span>
    <div class="chips">
      {#each suggestions as s}
        <button class="chip" onclick={() => onSend(s.label)}>
          <span class="chip-icon"><Icon name={s.icon} size={14} /></span>
          <span>{s.label}</span>
        </button>
      {/each}
    </div>
  </div>
</div>

<style>
  .landing {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: 32px 28px;
    gap: 28px;
  }
  .hero {
    text-align: center;
    max-width: 480px;
  }
  .hero-icon {
    width: 40px;
    height: 40px;
    border-radius: var(--radius-lg);
    background: var(--accent-soft);
    color: var(--accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 20px;
    margin: 0 auto 12px;
  }
  .hero h1 {
    font-size: var(--text-2xl);
    font-weight: var(--weight-bold);
    margin: 0 0 6px;
    color: var(--text);
  }
  .hero p {
    color: var(--text-secondary);
    font-size: var(--text-md);
    margin: 0;
    line-height: 1.5;
  }
  .input-wrap {
    width: 100%;
    max-width: 600px;
  }
  .suggestions {
    width: 100%;
    max-width: 600px;
  }
  .suggest-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: var(--weight-semibold);
    display: block;
    margin-bottom: 8px;
  }
  .chips {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .chip {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 8px 12px;
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--bg-surface);
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-secondary);
    text-align: left;
    transition: all var(--transition-fast);
  }
  .chip:hover {
    border-color: var(--accent);
    color: var(--text);
    background: var(--accent-soft);
  }
  .chip-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    color: var(--accent);
    opacity: 0.7;
  }
</style>
