<script lang="ts">
  import ProviderLogo, { PROVIDER_BRANDS } from './ProviderLogo.svelte';
  import type { InstalledAcpAgent } from '../../ipc/ai.ts';

  let {
    value = 'openai',
    acpAgentId = undefined,
    installedAgents = [],
    onChange = () => {},
  }: {
    value?: string;
    acpAgentId?: string;
    installedAgents?: InstalledAcpAgent[];
    onChange?: (id: string, agentId?: string) => void;
  } = $props();

  interface ProviderOption {
    id: string;
    label: string;
    group: string;
    sub?: string;
  }

  const PROVIDERS: ProviderOption[] = [
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

  let allOptions: ProviderOption[] = $derived([
    ...PROVIDERS,
    ...installedAgents.map((a) => ({
      id: 'acp',
      label: a.name ?? 'ACP Agent',
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

  let cards = $state<Record<string, HTMLButtonElement>>({});

  // Cards are keyed by agent id when present, because several ACP cards
  // share the provider id `acp`.
  function cardKey(p: { id: string; sub?: string }) {
    return p.sub ?? p.id;
  }

  function pick(p: { id: string; sub?: string }) {
    if (p.sub !== undefined) onChange(p.id, p.sub);
    else onChange(p.id);
  }

  function handleGridKeydown(
    options: { id: string; sub?: string }[],
    e: KeyboardEvent,
  ) {
    const keys = options.map(cardKey);
    const focusedIdx = keys.findIndex(
      (k) => cards[k] === document.activeElement,
    );
    const current =
      focusedIdx >= 0 ? focusedIdx : Math.max(0, keys.indexOf(value));

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
          style="--provider-tint: {((PROVIDER_BRANDS as Record<string, { color: string; tint: string }>)[p.id] ?? ACP_BRAND).tint};"
        >
          <span class="logo-tile">
            <ProviderLogo provider={p.id} size={13} />
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
    margin: 2px 0 6px;
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
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
    gap: 7px;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
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
    border-color: var(--provider-tint);
    background: var(--bg-hover);
  }
  .provider-card.selected {
    border-color: var(--provider-tint);
    background: color-mix(
      in oklch,
      var(--provider-tint) 10%,
      var(--bg-elevated)
    );
    box-shadow: inset 0 0 0 1px var(--provider-tint);
  }
  .provider-card:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .logo-tile {
    width: 20px;
    height: 20px;
    border-radius: var(--radius-sm);
    background: var(--provider-tint);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    transition: transform var(--transition-normal);
  }
  .card-name {
    font-size: 12px;
    font-weight: 600;
    line-height: 1.2;
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
  /* Inside the card, on the accent, at the trailing edge: a checkmark is a
     selection mark, so it belongs in the row it marks and in the colour the
     rest of the app selects with. Hung outside the corner it clipped against
     the neighbouring card and popped in on a bounce curve, reading as a
     notification badge rather than a state. */
  .check-badge {
    margin-left: auto;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--accent);
  }
</style>
