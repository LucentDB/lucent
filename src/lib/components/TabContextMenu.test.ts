import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import TabContextMenu from './TabContextMenu.svelte';
import type { TabMenuItem } from './tab-menu.ts';

afterEach(cleanup);

function makeItems(action: () => void): TabMenuItem[] {
  return [
    { label: 'Close Tab', icon: 'close', action },
    { label: 'Close Others', icon: 'close-others', action },
    { label: 'Close to the Right', icon: 'close-right', action },
    { label: 'Close to the Left', icon: 'close-left', action },
    { separator: true },
    { label: 'Close All', icon: 'close-all', action },
  ];
}

function setup(overrides: Record<string, unknown> = {}) {
  const action = vi.fn();
  const onClose = vi.fn();
  const result = render(TabContextMenu, {
    x: 10,
    y: 20,
    items: makeItems(action),
    onClose,
    ...overrides,
  });
  return { ...result, action, onClose };
}

describe('TabContextMenu', () => {
  it('renders every close action with a separator', () => {
    const { getByText, getByRole } = setup();
    expect(getByText('Close Tab')).toBeTruthy();
    expect(getByText('Close Others')).toBeTruthy();
    expect(getByText('Close to the Right')).toBeTruthy();
    expect(getByText('Close to the Left')).toBeTruthy();
    expect(getByText('Close All')).toBeTruthy();
    expect(getByRole('menu')).toBeTruthy();
  });

  it('runs the clicked action and then dismisses', async () => {
    const { getByText, action, onClose } = setup();
    await fireEvent.click(getByText('Close to the Right'));
    expect(action).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('ignores clicks on a disabled item', async () => {
    const action = vi.fn();
    const onClose = vi.fn();
    const { getByText } = render(TabContextMenu, {
      items: [
        { label: 'Close Tab', icon: 'close', disabled: true, action },
      ] satisfies TabMenuItem[],
      onClose,
    });
    await fireEvent.click(getByText('Close Tab'));
    expect(action).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('dismisses on Escape', async () => {
    const { onClose } = setup();
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('dismisses on a pointerdown outside but keeps clicks inside', async () => {
    const { getByText, onClose } = setup();
    await fireEvent.pointerDown(getByText('Close Tab'));
    expect(onClose).not.toHaveBeenCalled();
    await fireEvent.pointerDown(document.body);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
