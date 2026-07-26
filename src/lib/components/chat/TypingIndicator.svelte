<script lang="ts">
  let { visible = false }: { visible?: boolean } = $props();
</script>

{#if visible}
  <div class="typing">
    <div class="dots">
      <span class="dot"></span>
      <span class="dot"></span>
      <span class="dot"></span>
    </div>
    <span class="typing-label">Thinking…</span>
  </div>
{/if}

<style>
  .typing {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0 8px;
  }
  .dots {
    display: flex;
    gap: 4px;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-muted);
    animation: typing-dot 1.4s infinite both;
  }
  .dot:nth-child(2) {
    animation-delay: 0.2s;
  }
  .dot:nth-child(3) {
    animation-delay: 0.4s;
  }
  @keyframes typing-dot {
    0%,
    60%,
    100% {
      opacity: 0.3;
      transform: scale(0.8);
    }
    30% {
      opacity: 1;
      transform: scale(1);
    }
  }
  .typing-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
</style>
