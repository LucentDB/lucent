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

  it('navigates menu items with ArrowDown, ArrowUp, Home, and End', async () => {
    const { getByText } = setup();
    const closeTabBtn = getByText('Close Tab');
    const closeOthersBtn = getByText('Close Others');
    const closeAllBtn = getByText('Close All');

    // ArrowDown focuses first item
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(closeTabBtn);

    // ArrowDown moves to second item
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(closeOthersBtn);

    // End moves to last item
    await fireEvent.keyDown(window, { key: 'End' });
    expect(document.activeElement).toBe(closeAllBtn);

    // ArrowDown wraps around to first item
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(closeTabBtn);

    // ArrowUp wraps to last item
    await fireEvent.keyDown(window, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(closeAllBtn);

    // Home moves to first item
    await fireEvent.keyDown(window, { key: 'Home' });
    expect(document.activeElement).toBe(closeTabBtn);
  });

  it('restores focus on close', async () => {
    const trigger = document.createElement('button');
    document.body.appendChild(trigger);
    trigger.focus();
    expect(document.activeElement).toBe(trigger);

    const { unmount } = render(TabContextMenu, {
      items: makeItems(vi.fn()),
      onClose: vi.fn(),
    });

    unmount();
    expect(document.activeElement).toBe(trigger);
    document.body.removeChild(trigger);
  });
});
