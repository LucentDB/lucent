import { describe, it, expect, vi, afterEach } from 'vitest';
import {
  render,
  fireEvent,
  cleanup,
  waitFor,
  type RenderResult,
} from '@testing-library/svelte';

// MemoryDrawer reaches for the Tauri IPC boundary on open (listMemories /
// listGoldenQueries). Stub the module so these stay pure interaction tests.
vi.mock('../../ipc/ai.ts', () => ({
  listMemories: vi.fn(async () => []),
  saveMemoryManual: vi.fn(async () => {}),
  deleteMemory: vi.fn(async () => true),
  toggleMemoryStatus: vi.fn(async () => {}),
  resolveDrift: vi.fn(async () => {}),
  exportMemoriesMarkdown: vi.fn(async () => '# LUCENT.md'),
  importMemoriesMarkdown: vi.fn(async () => 0),
  listGoldenQueries: vi.fn(async () => []),
  deleteGoldenQuery: vi.fn(async () => true),
  runConsolidation: vi.fn(async () => ({
    archived_memory_count: 0,
    pruned_session_json_count: 0,
    drift_alerts: [],
  })),
}));

// The drift store registers a Tauri event listener; stub the boundary and
// capture the handler so the test can deliver a real `memory:drift_detected`.
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
    (globalThis as any).__listeners[event] = handler;
    return Promise.resolve(() => {});
  }),
}));

import MemoryDrawerHarness from './MemoryDrawerHarness.svelte';
import {
  listMemories,
  listGoldenQueries,
  deleteMemory,
  deleteGoldenQuery,
  runConsolidation,
} from '../../ipc/ai.ts';
import type { DriftAlert, GoldenQuery, MemoryItem } from '../../ipc/ai.ts';
import memoryDrawerSource from './MemoryDrawer.svelte?raw';
import {
  initMemoryDriftListeners,
  __resetForTests as resetMemoryDrift,
} from '../../stores/memoryDrift.svelte.ts';

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  resetMemoryDrift();
  (globalThis as any).__listeners = {};
  vi.mocked(listMemories).mockResolvedValue([]);
  vi.mocked(listGoldenQueries).mockResolvedValue([]);
  vi.mocked(runConsolidation).mockResolvedValue({
    archived_memory_count: 0,
    pruned_session_json_count: 0,
    drift_alerts: [],
  });
});

function memory(overrides: Partial<MemoryItem> = {}): MemoryItem {
  return {
    id: 'mem-1',
    connection_key: 'conn-1',
    scope: 'connection',
    category: 'metric',
    key_phrase: 'active_subscribers',
    rule_text: 'Count active subscribers only',
    importance: 0.8,
    stability_hours: 24,
    last_accessed_at: 0,
    access_count: 3,
    source_trust: 'user_explicit',
    status: 'active',
    valid_from: 0,
    learned_at: 0,
    tombstone: false,
    doc_hash: 'hash',
    embedding_model: 'model',
    embedding_version: 1,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

function goldenQuery(overrides: Partial<GoldenQuery> = {}): GoldenQuery {
  return {
    id: 'gq-1',
    connection_id: 'conn-1',
    schema_name: 'public',
    natural_prompt: 'how many active users',
    sql_text: 'SELECT count(*) FROM users',
    tables_used: ['users'],
    verified: true,
    run_count: 2,
    last_run_at: 0,
    embedding_model: 'model',
    embedding_version: 1,
    created_at: 0,
    ...overrides,
  };
}

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
}

async function openDrawer(
  utils: RenderResult<typeof MemoryDrawerHarness>,
): Promise<HTMLElement> {
  const trigger = utils.getByTestId('trigger');
  trigger.focus();
  await fireEvent.click(trigger);
  return waitFor(() =>
    utils.getByRole('dialog', { name: 'AI Memory Manager' }),
  );
}

describe('MemoryDrawer — modal dialog semantics (F-C3)', () => {
  it('adds aria-modal and a programmatic focus target without clobbering the label', async () => {
    const utils = render(MemoryDrawerHarness);
    const drawer = await openDrawer(utils);

    expect(drawer.getAttribute('role')).toBe('dialog');
    expect(drawer.getAttribute('aria-modal')).toBe('true');
    expect(drawer.getAttribute('aria-label')).toBe('AI Memory Manager');
    expect(drawer.getAttribute('tabindex')).toBe('-1');
    // The brief forbids aria-labelledby — it would duplicate the aria-label.
    expect(drawer.getAttribute('aria-labelledby')).toBeNull();
  });

  it('moves focus to the search input when the drawer opens', async () => {
    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);

    const search = utils.getByPlaceholderText('Search rules...');
    await waitFor(() => expect(document.activeElement).toBe(search));
  });

  it('wraps Tab forward from the last control back to the first', async () => {
    const utils = render(MemoryDrawerHarness);
    const drawer = await openDrawer(utils);

    const items = focusables(drawer);
    const first = items[0];
    const last = items[items.length - 1];
    last.focus();

    await fireEvent.keyDown(last, { key: 'Tab' });

    expect(document.activeElement).toBe(first);
    expect(drawer.contains(document.activeElement)).toBe(true);
  });

  it('wraps Shift+Tab backward from the first control to the last', async () => {
    const utils = render(MemoryDrawerHarness);
    const drawer = await openDrawer(utils);

    const items = focusables(drawer);
    const first = items[0];
    const last = items[items.length - 1];
    first.focus();

    await fireEvent.keyDown(first, { key: 'Tab', shiftKey: true });

    expect(document.activeElement).toBe(last);
    expect(drawer.contains(document.activeElement)).toBe(true);
  });

  it('traps Tab inside the Add Rule child modal, not the drawer behind it', async () => {
    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);

    await fireEvent.click(utils.getByText('+ Add Rule'));
    const modal = utils
      .getByText('Add Learned Rule')
      .closest('[role="dialog"]') as HTMLElement;
    expect(modal).toBeTruthy();

    const items = focusables(modal);
    const first = items[0];
    const last = items[items.length - 1];
    last.focus();

    await fireEvent.keyDown(last, { key: 'Tab' });

    expect(document.activeElement).toBe(first);
    expect(modal.contains(document.activeElement)).toBe(true);
  });

  it('pulls focus into the Add Rule child modal when Tab is pressed from behind it', async () => {
    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);

    await fireEvent.click(utils.getByText('+ Add Rule'));
    const modal = utils
      .getByText('Add Learned Rule')
      .closest('[role="dialog"]') as HTMLElement;
    const search = utils.getByPlaceholderText('Search rules...');
    search.focus();

    await fireEvent.keyDown(search, { key: 'Tab' });

    expect(modal.contains(document.activeElement)).toBe(true);
    expect(document.activeElement).toBe(focusables(modal)[0]);
  });

  it('traps Tab inside the Import Markdown child modal', async () => {
    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);

    await fireEvent.click(utils.getByText('Import MD'));
    const modal = utils
      .getByText('Import Markdown Rules (LUCENT.md)')
      .closest('[role="dialog"]') as HTMLElement;

    const items = focusables(modal);
    const first = items[0];
    const last = items[items.length - 1];
    last.focus();

    await fireEvent.keyDown(last, { key: 'Tab' });

    expect(document.activeElement).toBe(first);
    expect(modal.contains(document.activeElement)).toBe(true);
  });

  it('restores focus to the trigger when the drawer closes', async () => {
    const utils = render(MemoryDrawerHarness);
    const trigger = utils.getByTestId('trigger');
    await openDrawer(utils);
    const search = utils.getByPlaceholderText('Search rules...');
    // Focus must actually have left the trigger first, or the close assertion
    // below would pass even with no focus management at all.
    await waitFor(() => expect(document.activeElement).toBe(search));

    await fireEvent.keyDown(window, { key: 'Escape' });

    await waitFor(() =>
      expect(
        utils.queryByRole('dialog', { name: 'AI Memory Manager' }),
      ).toBeNull(),
    );
    expect(document.activeElement).toBe(trigger);
  });

  // Regression for review Important #1: in a browser, inerting the background
  // blurs the opener to <body> before any effect runs, so restoring from
  // `document.activeElement` would send focus to <body>. This emulates that
  // strip; it fails unless the explicit trigger ref is used.
  it('restores focus to the explicit trigger even after inert blurs the opener', async () => {
    const utils = render(MemoryDrawerHarness, { emulateInertBlur: true });
    const trigger = utils.getByTestId('trigger');
    await openDrawer(utils);

    const search = utils.getByPlaceholderText('Search rules...');
    await waitFor(() => expect(document.activeElement).toBe(search));

    await fireEvent.keyDown(window, { key: 'Escape' });

    await waitFor(() =>
      expect(
        utils.queryByRole('dialog', { name: 'AI Memory Manager' }),
      ).toBeNull(),
    );
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });
});

describe('MemoryDrawer — stacked Escape (F-C4)', () => {
  it('closes the Add Rule modal first, then the drawer on the next Escape', async () => {
    const onClose = vi.fn();
    const utils = render(MemoryDrawerHarness, { onClose });
    await openDrawer(utils);

    await fireEvent.click(utils.getByText('+ Add Rule'));
    expect(utils.getByText('Add Learned Rule')).toBeTruthy();

    await fireEvent.keyDown(window, { key: 'Escape' });

    // Child modal dismissed, drawer and its unsubmitted state survive.
    expect(utils.queryByText('Add Learned Rule')).toBeNull();
    expect(
      utils.getByRole('dialog', { name: 'AI Memory Manager' }),
    ).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();

    await fireEvent.keyDown(window, { key: 'Escape' });

    await waitFor(() =>
      expect(
        utils.queryByRole('dialog', { name: 'AI Memory Manager' }),
      ).toBeNull(),
    );
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes the Import Markdown modal first, then the drawer', async () => {
    const onClose = vi.fn();
    const utils = render(MemoryDrawerHarness, { onClose });
    await openDrawer(utils);

    await fireEvent.click(utils.getByText('Import MD'));
    expect(utils.getByText('Import Markdown Rules (LUCENT.md)')).toBeTruthy();

    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(utils.queryByText('Import Markdown Rules (LUCENT.md)')).toBeNull();
    expect(
      utils.getByRole('dialog', { name: 'AI Memory Manager' }),
    ).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();

    await fireEvent.keyDown(window, { key: 'Escape' });

    await waitFor(() =>
      expect(
        utils.queryByRole('dialog', { name: 'AI Memory Manager' }),
      ).toBeNull(),
    );
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe('MemoryDrawer — two-step delete confirmation (F-I1)', () => {
  it('requires a second click before deleting a rule', async () => {
    vi.mocked(listMemories).mockResolvedValue([memory()]);
    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);

    const del = await utils.findByText('Delete');
    await fireEvent.click(del);

    expect(deleteMemory).not.toHaveBeenCalled();
    expect(del.textContent).toBe('Confirm?');

    await fireEvent.click(del);
    await waitFor(() => expect(deleteMemory).toHaveBeenCalledWith('mem-1'));
  });

  it('moves the confirmation to the newly targeted rule', async () => {
    vi.mocked(listMemories).mockResolvedValue([
      memory({ id: 'mem-1', key_phrase: 'first' }),
      memory({ id: 'mem-2', key_phrase: 'second' }),
    ]);
    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);

    const dels = await utils.findAllByText('Delete');
    expect(dels).toHaveLength(2);

    await fireEvent.click(dels[0]);
    expect(dels[0].textContent).toBe('Confirm?');

    await fireEvent.click(dels[1]);
    expect(dels[0].textContent).toBe('Delete');
    expect(dels[1].textContent).toBe('Confirm?');
    expect(deleteMemory).not.toHaveBeenCalled();
  });

  it('requires a second click before deleting a golden query', async () => {
    vi.mocked(listGoldenQueries).mockResolvedValue([goldenQuery()]);
    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);
    await fireEvent.click(utils.getByText(/Golden Queries/));

    const del = await utils.findByText('Delete');
    await fireEvent.click(del);

    expect(deleteGoldenQuery).not.toHaveBeenCalled();
    expect(del.textContent).toBe('Confirm?');

    await fireEvent.click(del);
    await waitFor(() => expect(deleteGoldenQuery).toHaveBeenCalledWith('gq-1'));
  });
});

describe('MemoryDrawer — genuine drift alerts (F-I2)', () => {
  const realAlert: DriftAlert = {
    memory_id: 'mem-stale',
    rule_text: 'stale rule',
    reason: 'column users.email was dropped',
    schema_name: 'public',
    table_name: 'users',
    column_name: 'email',
  };

  it('keeps genuine backend alerts instead of synthesizing fallbacks', async () => {
    vi.mocked(listMemories).mockResolvedValue([
      memory({
        id: 'mem-stale',
        key_phrase: 'stale_phrase',
        status: 'stale_invalid',
      }),
    ]);
    vi.mocked(runConsolidation).mockResolvedValue({
      archived_memory_count: 0,
      pruned_session_json_count: 0,
      drift_alerts: [realAlert],
    });

    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);

    await fireEvent.click(utils.getByText('Consolidate'));
    await waitFor(() =>
      expect(utils.getByText(/Maintenance complete/)).toBeTruthy(),
    );

    await fireEvent.click(utils.getByText(/Drift Alerts/));

    await waitFor(() =>
      expect(utils.getByText('column users.email was dropped')).toBeTruthy(),
    );
    expect(utils.getByText('users')).toBeTruthy();
    // The synthesized fallback reason must not replace the real one.
    expect(
      utils.queryByText(
        'Referenced table or column was altered or dropped in live catalog',
      ),
    ).toBeNull();
  });

  it('synthesizes a fallback only when the backend reports none', async () => {
    vi.mocked(listMemories).mockResolvedValue([
      memory({
        id: 'mem-stale',
        key_phrase: 'stale_phrase',
        status: 'stale_invalid',
      }),
    ]);

    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);
    await fireEvent.click(utils.getByText(/Drift Alerts/));

    await waitFor(() =>
      expect(
        utils.getByText(
          'Referenced table or column was altered or dropped in live catalog',
        ),
      ).toBeTruthy(),
    );
  });

  it('shows genuine alerts delivered by a memory:drift_detected event (B-C5)', async () => {
    vi.mocked(listMemories).mockResolvedValue([
      memory({
        id: 'mem-stale',
        key_phrase: 'stale_phrase',
        status: 'stale_invalid',
      }),
    ]);

    // A real backend event arrives for this connection before the drawer opens.
    await initMemoryDriftListeners();
    const handler = (globalThis as any).__listeners['memory:drift_detected'];
    handler({
      payload: [
        {
          connection_key: 'conn-1',
          memory_id: 'mem-stale',
          rule_text: 'stale rule',
          reason: "Table 'public.users' was dropped",
          schema_name: 'public',
          table_name: 'users',
          column_name: null,
        },
      ],
    });

    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);
    await fireEvent.click(utils.getByText(/Drift Alerts/));

    await waitFor(() =>
      expect(utils.getByText("Table 'public.users' was dropped")).toBeTruthy(),
    );
    // The backend reason is preferred over the synthesized fallback.
    expect(
      utils.queryByText(
        'Referenced table or column was altered or dropped in live catalog',
      ),
    ).toBeNull();
  });
});

describe('MemoryDrawer — accessible tabs (F-I3)', () => {
  it('exposes the navigation as an ARIA tablist with one selected tab', async () => {
    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);

    const tablist = utils.getByRole('tablist');
    const tabs = utils.getAllByRole('tab');
    expect(tabs).toHaveLength(4);
    expect(tablist.contains(tabs[0])).toBe(true);

    const selected = tabs.filter(
      (t) => t.getAttribute('aria-selected') === 'true',
    );
    expect(selected).toHaveLength(1);
    expect(tabs[0].getAttribute('aria-selected')).toBe('true');
  });

  it('moves selection with ArrowRight and roving tabindex', async () => {
    const utils = render(MemoryDrawerHarness);
    await openDrawer(utils);

    const tabs = utils.getAllByRole('tab');
    tabs[0].focus();
    await fireEvent.keyDown(tabs[0], { key: 'ArrowRight' });

    await waitFor(() =>
      expect(tabs[1].getAttribute('aria-selected')).toBe('true'),
    );
    expect(tabs[1].getAttribute('tabindex')).toBe('0');
    expect(tabs[0].getAttribute('tabindex')).toBe('-1');
  });
});

describe('MemoryDrawer — reduced motion & typography (F-I7, F-I8)', () => {
  it('disables the drawer slide-in under prefers-reduced-motion', () => {
    expect(memoryDrawerSource).toMatch(
      /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.drawer\s*\{[^}]*animation:\s*none/,
    );
  });

  it('keeps no sub-11px font sizes', () => {
    const styles =
      memoryDrawerSource.match(/<style>([\s\S]*?)<\/style>/)?.[1] ?? '';
    expect(styles).not.toMatch(/font-size:\s*(?:9|10)px/);
  });
});
