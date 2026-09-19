/**
 * Shared shape for the tab context menu.
 *
 * DB tabs (AppHeader) and chat tabs (ChatPanel) both build a `TabMenuItem[]`
 * and hand it to `TabContextMenu`, so the two tab strips always offer the
 * same actions with the same icons, shortcuts, and ordering.
 */
export type TabMenuIcon =
  | 'save'
  | 'save-as'
  | 'open-notebook'
  | 'close'
  | 'close-others'
  | 'close-right'
  | 'close-left'
  | 'close-all';

export interface TabMenuAction {
  label: string;
  icon: TabMenuIcon;
  /** Rendered right-aligned, e.g. `⌘S`. */
  shortcut?: string;
  disabled?: boolean;
  action: () => void;
}

export interface TabMenuSeparator {
  separator: true;
}

export type TabMenuItem = TabMenuAction | TabMenuSeparator;
