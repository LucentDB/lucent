<script>
  import { onMount } from 'svelte';
  import ProviderLogo, { PROVIDER_BRANDS } from './ProviderLogo.svelte';
  import { listInstalledAcpAgents } from '../../ipc/ai.ts';

  let {
    value = 'openai',
    acpAgentId = undefined,
    onChange = () => {},
  } = $props();

  const PROVIDERS = [
    { id: 'openai', label: 'OpenAI', group: 'Cloud providers' },
    { id: 'anthropic', label: 'Anthropic', group: 'Cloud providers' },
    { id: 'gemini', label: 'Gemini', group: 'Cloud providers' },
    { id: 'openrouter', label: 'OpenRouter', group: 'Cloud providers' },
    { id: 'mistral', label: 'Mistral', group: 'Cloud providers' },
    { id: 'deepseek', label: 'DeepSeek', group: 'Cloud providers' },
    { id: 'groq', label: 'Groq', group: 'Cloud providers' },
    { id: 'xai', label: 'xAI', group: 'Cloud providers' },
    { id: 'opencode', label: 'OpenCode', group: 'Cloud providers' },
    { id: 'ollama', label: 'Ollama (local)', group: 'Local & self-hosted' },
    {
      id: 'custom',
      label: 'Custom (OpenAI-compatible)',
      group: 'Local & self-hosted',
    },
  ];

  // Generic brand for ACP entries — `PROVIDER_BRANDS` has no `acp` key, so
  // cards fall back to this tint instead of crashing on an undefined lookup.
  const ACP_BRAND = { color: '#8b5cf6', tint: 'rgba(139,92,246,0.12)' };

  // Installed ACP agents, loaded on mount (one card per agent, all with the
  // `acp` provider id — the agent id travels in `sub`).
  let acpAgents = $state([]);

  let allOptions = $derived([
    ...PROVIDERS,
    ...acpAgents.map((a) => ({
      id: 'acp',
      label: 'ACP Agent',
      sub: a.id,
      group: 'Agents (ACP)',
    })),
  ]);

  let groups = $derived(
    [...new Set(allOptions.map((p) => p.group))].map((group) => ({
      group,
      options: allOptions.filter((p) => p.group === group),
    })),
  );

  let cards = $state({});

  // Cards are keyed by agent id when present, because several ACP cards
  // share the provider id `acp`.
  function cardKey(p) {
    return p.sub ?? p.id;
  }

  onMount(async () => {
    try {
      const installed = await listInstalledAcpAgents();
      acpAgents = installed ?? [];
    } catch {
      acpAgents = [];
    }
  });

  function pick(p) {
    if (p.sub !== undefined) onChange(p.id, p.sub);
    else onChange(p.id);
  }

  function handleGridKeydown(options, e) {
    const keys = options.map(cardKey);
    const focusedIdx = keys.findIndex(
      (k) => cards[k] === document.activeElement,
    );
    const current = focusedIdx >= 0 ? focusedIdx : Math.max(0, keys.indexOf(value));

    let next = -1;
    if (e.key === 'ArrowRight') {
      e.preventDefault();
      next = (current + 1) % keys.length;
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      next = (current - 1 + keys.length) % keys.length;
    } else if (e.key === 'Home') {
      e.preventDefault();
      next = 0;
    } else if (e.key === 'End') {
      e.preventDefault();
      next = keys.length - 1;
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      const focusedKey = keys.find((k) => cards[k] === document.activeElement);
      const focused = options.find((p) => cardKey(p) === focusedKey);
      if (focused) pick(focused);
      return;
    }

    if (next >= 0) {
      const opt = options[next];
      pick(opt);
      cards[cardKey(opt)]?.focus();
    }
  }
</script>

<div class="provider-picker">
  {#each groups as g}
    <div class="group-caption">{g.group}</div>
    <div
      class="provider-grid"
      role="radiogroup"
      aria-label={g.group}
      tabindex="0"
      onkeydown={(e) => handleGridKeydown(g.options, e)}
    >
      {#each g.options as p (cardKey(p))}
        <button
          type="button"
          class="provider-card"
          class:selected={value === p.id &&
            (p.sub === undefined || acpAgentId === p.sub)}
          role="radio"
          aria-checked={value === p.id &&
            (p.sub === undefined || acpAgentId === p.sub)}
          aria-label={p.sub !== undefined ? `${p.label} — ${p.sub}` : p.label}
          bind:this={cards[cardKey(p)]}
          onclick={() => pick(p)}
          style="--provider-tint: {(PROVIDER_BRANDS[p.id] ?? ACP_BRAND).tint};"
        >
          <span class="logo-tile">
            <ProviderLogo provider={p.id} size={16} />
          </span>
          <span class="card-name">
            {p.label}
            {#if p.sub !== undefined}
              <span class="card-sub">{p.sub}</span>
            {/if}
          </span>
          {#if value === p.id && (p.sub === undefined || acpAgentId === p.sub)}
            <span class="check-badge">
              <svg viewBox="0 0 16 16" width="10" height="10">
                <path
                  d="M3 8.5l3.2 3.2L13 5"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
            </span>
          {/if}
        </button>
      {/each}
    </div>
  {/each}
</div>

<style>
  .provider-picker {
    display: flex;
    flex-direction: column;
  }
  .group-caption {
    margin: 4px 0 12px;
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .provider-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
    margin-bottom: 20px;
    outline: none;
  }
  .provider-grid:last-child {
    margin-bottom: 0;
  }
  .provider-card {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    cursor: pointer;
    text-align: left;
    font: inherit;
    color: var(--text);
    overflow: visible;
    transition:
      transform 0.25s cubic-bezier(0.34, 1.56, 0.64, 1),
      border-color 0.25s ease,
      background 0.25s ease,
      box-shadow 0.25s ease;
  }
  .provider-card:hover {
    transform: scale(1.03);
    border-color: var(--provider-tint);
    background: var(--bg-hover);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.06);
    z-index: 1;
  }
  .provider-card.selected {
    border-color: var(--provider-tint);
    background: color-mix(in srgb, var(--provider-tint) 12%, var(--bg-surface));
    box-shadow: 
      0 4px 16px rgba(0, 0, 0, 0.08),
      inset 0 0 0 1px var(--provider-tint),
      inset 0 2px 12px color-mix(in srgb, var(--provider-tint) 20%, transparent);
  }
  .provider-card:focus-visible {
    outline: 2px solid var(--accent-selection);
    outline-offset: 2px;
  }
  .logo-tile {
    width: 28px;
    height: 28px;
    border-radius: 7px;
    background: var(--provider-tint);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    transition: transform 0.25s cubic-bezier(0.34, 1.56, 0.64, 1);
  }
  .provider-card.selected .logo-tile {
    transform: scale(1.08);
  }
  .card-name {
    font-size: 12px;
    font-weight: 600;
    line-height: 1.2;
    letter-spacing: -0.01em;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .card-sub {
    display: block;
    font-size: 10.5px;
    font-weight: 500;
    color: var(--text-muted);
    letter-spacing: 0;
  }
  .check-badge {
    position: absolute;
    top: -6px;
    right: -6px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--provider-tint);
    color: #ffffff;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 0 0 2px var(--bg-surface), 0 2px 6px rgba(0,0,0,0.15);
    animation: badge-pop 0.4s cubic-bezier(0.34, 1.56, 0.64, 1) forwards;
  }
  @keyframes badge-pop {
    0% {
      opacity: 0;
      transform: scale(0.3) rotate(-15deg);
    }
    100% {
      opacity: 1;
      transform: scale(1) rotate(0deg);
    }
  }
</style>
