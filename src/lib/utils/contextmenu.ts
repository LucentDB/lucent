/**
 * Suppresses default webview context menu (Reload, Inspect Element, etc.).
 */
export function preventDefaultContextMenu(e: Event): void {
  e.preventDefault();
}

/**
 * Attaches a contextmenu listener to suppress native browser context menus.
 * Returns an unlisten function.
 */
export function initContextMenuSuppression(
  target: EventTarget = window,
): () => void {
  target.addEventListener('contextmenu', preventDefaultContextMenu, true);
  return () => {
    target.removeEventListener('contextmenu', preventDefaultContextMenu, true);
  };
}
