import { describe, it, expect, afterEach } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';
import SqlCell from './SqlCell.svelte';
import { createNotebookModel } from '../../../stores/notebook.svelte.ts';

afterEach(cleanup);

function setup(source: string, focused = true) {
  const model = createNotebookModel();
  return render(SqlCell, {
    props: {
      source,
      status: 'pending',
      cellId: model.cells[0].id,
      model,
      focused,
    },
  });
}

describe('SqlCell', () => {
  it('renders CodeMirror editor', () => {
    const { container } = setup('SELECT 1');
    expect(container.querySelector('.cm-editor')).toBeTruthy();
  });

  it('highlights ${cellId} references as pills', () => {
    const { container } = setup('SELECT * FROM ${a1b2c3d4}');
    expect(container.querySelector('.cm-cell-ref-pill')).toBeTruthy();
  });

  it('does NOT highlight $1, $$, $user', () => {
    const { container } = setup('SELECT $$body$$, $1, $user');
    expect(container.querySelector('.cm-cell-ref-pill')).toBeFalsy();
  });

  it('shows ref preview below editor', () => {
    const { container } = setup('SELECT * FROM ${a1b2c3d4}');
    expect(container.textContent).toContain('_cell_a1b2c3d4');
  });
});

it('renders a static preview instead of an editor when far off-screen', () => {
  // jsdom has no IntersectionObserver, so the component must fall back to
  // static rendering rather than throwing.
  const { container } = setup('SELECT 1', false);
  expect(container.querySelector('.sql-static')?.textContent).toContain(
    'SELECT 1',
  );
});

it('mounts the editor when the cell is focused', async () => {
  const model = createNotebookModel();
  const { container } = render(SqlCell, {
    props: {
      source: 'SELECT 1',
      status: 'ok',
      cellId: model.cells[0].id,
      model,
      focused: true,
    },
  });
  await Promise.resolve();
  expect(container.querySelector('.cm-editor')).toBeTruthy();
});

it('blurs CodeMirror when the cell leaves edit mode', async () => {
  const model = createNotebookModel();
  const cellId = model.cells[0].id;
  const props = {
    source: 'SELECT 1',
    status: 'ok',
    cellId,
    model,
    selected: true,
    focused: true,
  };
  const { container, rerender } = render(SqlCell, { props });
  await Promise.resolve();

  const content = container.querySelector('.cm-content') as HTMLElement;
  content.focus();
  expect(document.activeElement).toBe(content);

  await rerender({ ...props, focused: false });
  await Promise.resolve();

  expect(document.activeElement).not.toBe(content);
});
