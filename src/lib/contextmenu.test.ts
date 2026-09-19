import { describe, it, expect } from 'vitest';
import {
  initContextMenuSuppression,
  preventDefaultContextMenu,
} from './utils/contextmenu.ts';

describe('contextmenu default prevention', () => {
  it('prevents default on contextmenu events when initialized', () => {
    const unlisten = initContextMenuSuppression(window);

    const event = new MouseEvent('contextmenu', {
      bubbles: true,
      cancelable: true,
    });

    const notCancelled = window.dispatchEvent(event);

    expect(notCancelled).toBe(false);
    expect(event.defaultPrevented).toBe(true);

    unlisten();

    const secondEvent = new MouseEvent('contextmenu', {
      bubbles: true,
      cancelable: true,
    });

    const allowed = window.dispatchEvent(secondEvent);
    expect(allowed).toBe(true);
    expect(secondEvent.defaultPrevented).toBe(false);
  });

  it('preventDefaultContextMenu explicitly calls preventDefault on event', () => {
    const event = new MouseEvent('contextmenu', { cancelable: true });
    expect(event.defaultPrevented).toBe(false);
    preventDefaultContextMenu(event);
    expect(event.defaultPrevented).toBe(true);
  });
});
