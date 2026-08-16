// @vitest-environment jsdom
import { render, cleanup } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, expect, test, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
}));

import QueryEditor from './QueryEditor.svelte';
import { getTheme } from '../../stores/theme.svelte.js';

afterEach(() => {
  cleanup();
  document.documentElement.classList.remove('dark');
  document.documentElement.style.removeProperty('--bg-surface');
});

test('mounts the query editor without invalid highlight tags', () => {
  expect(() =>
    render(QueryEditor, {
      props: { onExecute: vi.fn() },
    }),
  ).not.toThrow();
});

test('reconfigures the editor when switching from dark to light mode', async () => {
  const theme = getTheme();
  if (theme.current === 'light') theme.toggle();

  const { container } = render(QueryEditor, {
    props: { onExecute: vi.fn() },
  });
  const editor = container.querySelector('.cm-editor') as HTMLElement;
  const darkClasses = editor.className;

  theme.toggle();
  await tick();

  expect(editor.className).not.toBe(darkClasses);
});

test('uses the light surface for the CodeMirror gutter in light mode', () => {
  document.documentElement.classList.remove('dark');
  document.documentElement.style.setProperty('--bg-surface', '#ffffff');
  const { container } = render(QueryEditor, {
    props: { onExecute: vi.fn() },
  });
  const gutter = container.querySelector('.cm-gutters');
  const gutterRules = [...document.styleSheets].flatMap((sheet) =>
    [...sheet.cssRules]
      .map((rule) => rule.cssText)
      .filter((cssText) => cssText.includes('cm-gutters')),
  );

  expect(gutter).toBeTruthy();
  expect(
    gutterRules.some((cssText) =>
      cssText.includes('background-color: var(--bg-surface)'),
    ),
  ).toBe(true);
});
