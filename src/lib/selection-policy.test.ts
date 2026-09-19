import { describe, it, expect } from 'vitest';

// The selection policy lives in the global stylesheet, so it cannot be
// exercised through a component render. These assertions pin the contract:
// chrome does not select by default, data surfaces opt in, controls and code
// editors always select.
//
// Vitest stubs CSS imports, so the file is read from disk instead. The repo
// does not ship @types/node; the dynamic import is the one Node API used here.
// @ts-expect-error -- node:fs is a runtime-only dependency of the test runner.
const fs = await import('node:fs');
const root = fs.realpathSync('.');
const compactCss = fs
  .readFileSync(`${root}/src/app.css`, 'utf8')
  .replace(/\s+/g, ' ')
  .trim();

/** Declaration blocks for every rule with the given selector list. */
function rules(selectorList: string): string[] {
  const selector = selectorList.replace(/\s+/g, ' ').trim();
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return [
    ...compactCss.matchAll(new RegExp(`${escaped} \\{([^}]*)\\}`, 'g')),
  ].map((match) => match[1]);
}

function hasRule(selectorList: string, declaration: RegExp): boolean {
  return rules(selectorList).some((block) => declaration.test(block));
}

describe('text selection policy (app.css)', () => {
  it('turns selection off on the window chrome by default', () => {
    expect(hasRule('body', /user-select: none/)).toBe(true);
    expect(hasRule('body', /-webkit-user-select: none/)).toBe(true);
  });

  it('keeps form controls and CodeMirror editors selectable', () => {
    const selector = 'input, textarea, select, [contenteditable], .cm-content';
    expect(hasRule(selector, /user-select: text/)).toBe(true);
    expect(hasRule(selector, /-webkit-user-select: text/)).toBe(true);
  });

  it('provides a .selectable opt-in for data surfaces', () => {
    expect(hasRule('.selectable', /user-select: text/)).toBe(true);
    expect(hasRule('.selectable', /-webkit-user-select: text/)).toBe(true);
  });

  it('keeps controls unselectable, and lets an explicit .selectable opt-in win', () => {
    const controls =
      "button, [role='button'], [role='menuitem'], [role='menuitemcheckbox'], [role='option'], [role='tab'], summary, kbd";
    expect(hasRule(controls, /user-select: none/)).toBe(true);
    expect(hasRule(controls, /-webkit-user-select: none/)).toBe(true);

    // Equal specificity: source order decides, and the explicit opt-in must
    // come after the reset (the notebook's static SQL preview is a
    // `role="button"` element that carries `.selectable`).
    expect(compactCss.lastIndexOf('.selectable {')).toBeGreaterThan(
      compactCss.indexOf("button, [role='button'],"),
    );
  });
});
