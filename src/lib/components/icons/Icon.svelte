<script lang="ts">
  let { name = 'search', size = 14 }: { name: string; size?: number } =
    $props();
</script>

{#if name === 'search'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <circle cx="11" cy="11" r="8" /><path d="M21 21l-4.35-4.35" />
  </svg>
{:else if name === 'chart'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <line x1="18" y1="20" x2="18" y2="10" /><line
      x1="12"
      y1="20"
      x2="12"
      y2="4"
    /><line x1="6" y1="20" x2="6" y2="14" />
  </svg>
{:else if name === 'trending'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <polyline points="23 6 13.5 15.5 8.5 10.5 1 18" /><polyline
      points="17 6 23 6 23 12"
    />
  </svg>
{:else if name === 'clean'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d="M3 6h18" /><path
      d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"
    /><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /><line
      x1="10"
      y1="11"
      x2="10"
      y2="17"
    /><line x1="14" y1="11" x2="14" y2="17" />
  </svg>
{:else if name === 'star'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <polygon
      points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"
    />
  </svg>
{:else if name === 'chat'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
  </svg>
{:else if name === 'query'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <polyline points="16 3 21 3 21 8" /><line
      x1="4"
      y1="20"
      x2="21"
      y2="3"
    /><polyline points="21 16 21 21 16 21" /><line
      x1="15"
      y1="15"
      x2="21"
      y2="21"
    /><line x1="4" y1="4" x2="9" y2="9" />
  </svg>
{:else if name === 'table'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <rect x="3" y="3" width="18" height="18" rx="2" /><path d="M3 9h18" /><path
      d="M3 15h18"
    /><path d="M9 3v18" />
  </svg>
{:else if name === 'source'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path
      d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"
    /><polyline points="14 2 14 8 20 8" /><line
      x1="16"
      y1="13"
      x2="8"
      y2="13"
    /><line x1="16" y1="17" x2="8" y2="17" />
  </svg>
{:else if name === 'close'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
  </svg>
{:else if name === 'chevron'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d="M6 4l4 4-4 4" />
  </svg>
{:else if name === 'sparkle'}
  <!-- Filled four-point sparkle. Reads as "AI" far better than an outlined
       five-point star, which looks like a favourite/bookmark control. -->
  <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
    <path
      d="M12 2.6l1.55 4.3a3.6 3.6 0 0 0 2.55 2.55L20.4 11l-4.3 1.55a3.6 3.6 0 0 0-2.55 2.55L12 19.4l-1.55-4.3a3.6 3.6 0 0 0-2.55-2.55L3.6 11l4.3-1.55A3.6 3.6 0 0 0 10.45 6.9L12 2.6z"
    />
    <path
      d="M18.7 15.6l.62 1.72 1.72.62-1.72.62-.62 1.72-.62-1.72-1.72-.62 1.72-.62.62-1.72z"
      opacity="0.7"
    />
  </svg>
{:else if name === 'arrow'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2.2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <line x1="5" y1="12" x2="18" y2="12" /><polyline
      points="12 6 18 12 12 18"
    />
  </svg>
{:else if name === 'settings'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <line x1="4" y1="8" x2="20" y2="8" /><line
      x1="4"
      y1="16"
      x2="20"
      y2="16"
    /><circle cx="10" cy="8" r="2.2" /><circle cx="15" cy="16" r="2.2" />
  </svg>
{:else if name === 'replay'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d="M3 10a9 9 0 1 1 3 6.7" /><polyline points="3 4 3 10 9 10" />
  </svg>
{:else if name === 'database'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <ellipse cx="12" cy="5" rx="8" ry="3" /><path
      d="M4 5v14c0 1.7 3.6 3 8 3s8-1.3 8-3V5"
    /><path d="M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3" />
  </svg>
{/if}
