import { describe, it, expect } from 'vitest';
import { QueryClient } from '@tanstack/svelte-query';
import { queryClient } from './client.ts';

describe('queryClient', () => {
  it('is a QueryClient instance', () => {
    expect(queryClient).toBeInstanceOf(QueryClient);
  });

  it('retries once by default so a flaky IPC call does not surface immediately', () => {
    const defaults = queryClient.getDefaultOptions();
    expect(defaults.queries?.retry).toBe(1);
  });

  it('does not refetch on window focus — Tauri windows refocus constantly', () => {
    const defaults = queryClient.getDefaultOptions();
    expect(defaults.queries?.refetchOnWindowFocus).toBe(false);
  });

  it('is the same instance on repeated import', async () => {
    const again = await import('./client.ts');
    expect(again.queryClient).toBe(queryClient);
  });
});
