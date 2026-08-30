import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import UpdateBanner from './UpdateBanner.svelte';

// This project does not enable vitest `globals`, so @testing-library/svelte
// cannot auto-register its cleanup. Without this every render in the file
// stays mounted and queries match components from earlier tests.
afterEach(cleanup);

// The plugin talks to the Tauri runtime, which does not exist under jsdom.
const check = vi.fn();
const relaunch = vi.fn();
vi.mock('@tauri-apps/plugin-updater', () => ({ check: () => check() }));
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: () => relaunch() }));

describe('UpdateBanner', () => {
  beforeEach(() => {
    check.mockReset();
    relaunch.mockReset();
  });

  it('renders nothing when no update is available', async () => {
    check.mockResolvedValue(null);
    const { container } = render(UpdateBanner);
    await vi.waitFor(() => expect(check).toHaveBeenCalled());
    expect(container.textContent).not.toContain('Update');
  });

  it('offers the new version when one is available', async () => {
    check.mockResolvedValue({ version: '0.2.0', downloadAndInstall: vi.fn() });
    const { container } = render(UpdateBanner);
    await vi.waitFor(() => expect(container.textContent).toContain('0.2.0'));
  });

  // A failed check must never surface as a broken UI: the app works fine
  // offline, and a network blip is not something to interrupt the user with.
  it('stays silent when the check fails', async () => {
    check.mockRejectedValue(new Error('offline'));
    const { container } = render(UpdateBanner);
    await vi.waitFor(() => expect(check).toHaveBeenCalled());
    expect(container.textContent).not.toContain('Update');
  });

  it('installs and relaunches when the user accepts', async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    check.mockResolvedValue({ version: '0.2.0', downloadAndInstall });
    const { container, getByRole } = render(UpdateBanner);
    await vi.waitFor(() => expect(container.textContent).toContain('0.2.0'));
    await fireEvent.click(getByRole('button', { name: /install/i }));
    await vi.waitFor(() => expect(relaunch).toHaveBeenCalled());
    expect(downloadAndInstall).toHaveBeenCalled();
  });
});
