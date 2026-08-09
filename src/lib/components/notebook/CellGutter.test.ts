import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, cleanup, fireEvent } from '@testing-library/svelte';
import CellGutter from './CellGutter.svelte';

afterEach(cleanup);

function props(over: Record<string, unknown> = {}) {
  return {
    executionOrder: null,
    status: 'pending',
    runnable: true,
    collapsed: false,
    ...over,
  };
}

describe('CellGutter execution counter', () => {
  it('shows the execution order in brackets once the cell has run', () => {
    const { container } = render(CellGutter, {
      props: props({ executionOrder: 3, status: 'ok' }),
    });
    expect(container.querySelector('.order-number')?.textContent).toBe(
      'In [3]',
    );
  });

  it('shows empty brackets for a never-run cell', () => {
    const { container } = render(CellGutter, { props: props() });
    const order = container.querySelector('.order-number');
    expect(order?.textContent).toBe('In [ ]');
    expect(order?.classList.contains('order-empty')).toBe(true);
  });

  it('shows a spinner instead of a counter while running', () => {
    const { container } = render(CellGutter, {
      props: props({ executionOrder: 1, status: 'running' }),
    });
    expect(container.querySelector('.spinner')).toBeTruthy();
    expect(container.querySelector('.order-number')).toBeNull();
  });
});

describe('CellGutter run control', () => {
  // Jupyter/Colab/Databricks all fuse the counter and the run button into one
  // control at the first line of code. There must be exactly one run affordance.
  it('runs the cell when the counter is clicked', async () => {
    const onRun = vi.fn();
    const { container } = render(CellGutter, { props: props({ onRun }) });
    await fireEvent.click(container.querySelector('.gutter-run')!);
    expect(onRun).toHaveBeenCalledOnce();
  });

  it('does not re-run a cell that is already running', async () => {
    const onRun = vi.fn();
    const { container } = render(CellGutter, {
      props: props({ status: 'running', onRun }),
    });
    const btn = container.querySelector('.gutter-run') as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    await fireEvent.click(btn);
    expect(onRun).not.toHaveBeenCalled();
  });

  it('offers no run control for a non-runnable cell', () => {
    const { container } = render(CellGutter, {
      props: props({ runnable: false }),
    });
    expect(container.querySelector('.gutter-run')).toBeNull();
    expect(container.querySelector('.gutter-spacer')).toBeTruthy();
  });

  it('exposes only one run control, never a second floating button', () => {
    const { container } = render(CellGutter, { props: props() });
    const runControls = container.querySelectorAll('[aria-label="Run cell"]');
    expect(runControls.length).toBe(1);
  });
});

describe('CellGutter collapse and reorder', () => {
  it('toggles collapse when the chevron is clicked', async () => {
    const onToggleCollapse = vi.fn();
    const { container } = render(CellGutter, {
      props: props({ onToggleCollapse }),
    });
    await fireEvent.click(container.querySelector('.gutter-collapse')!);
    expect(onToggleCollapse).toHaveBeenCalledOnce();
  });

  it('keeps the collapse control reachable while collapsed', () => {
    const { container } = render(CellGutter, {
      props: props({ collapsed: true }),
    });
    const btn = container.querySelector('.gutter-collapse');
    expect(btn?.getAttribute('aria-expanded')).toBe('false');
    expect(btn?.getAttribute('aria-label')).toBe('Expand cell');
  });

  it('hides the drag grip while collapsed, since there is no body to drag', () => {
    const { container } = render(CellGutter, {
      props: props({ collapsed: true }),
    });
    expect(container.querySelector('.gutter-grip')).toBeNull();
  });

  it('reorders with Alt and the arrow keys, not only by dragging', async () => {
    const onMoveUp = vi.fn();
    const onMoveDown = vi.fn();
    const { container } = render(CellGutter, {
      props: props({ onMoveUp, onMoveDown }),
    });
    const grip = container.querySelector('.gutter-grip')!;
    await fireEvent.keyDown(grip, { key: 'ArrowUp', altKey: true });
    await fireEvent.keyDown(grip, { key: 'ArrowDown', altKey: true });
    expect(onMoveUp).toHaveBeenCalledOnce();
    expect(onMoveDown).toHaveBeenCalledOnce();
  });
});
