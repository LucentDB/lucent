import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, cleanup, fireEvent } from '@testing-library/svelte';
import CellToolbar from './CellToolbar.svelte';

const writeText = vi.fn();

beforeEach(() => {
  writeText.mockReset();
  writeText.mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText },
    configurable: true,
    writable: true,
  });
});

afterEach(cleanup);

describe('CellToolbar reference chip', () => {
  it('shows the paste-ready reference syntax, not a bare hex id', () => {
    const { container } = render(CellToolbar, {
      props: { cellId: 'a1b2c3d4', cellStatus: 'ok' as const },
    });
    expect(container.querySelector('.ref-chip code')?.textContent).toBe(
      '${a1b2c3d4}',
    );
  });

  it('copies the full reference, wrapper included, so it pastes as-is', async () => {
    const { container } = render(CellToolbar, {
      props: { cellId: 'a1b2c3d4', cellStatus: 'ok' as const },
    });
    await fireEvent.click(container.querySelector('.ref-chip')!);
    expect(writeText).toHaveBeenCalledWith('${a1b2c3d4}');
  });

  it('copies the whole 8-character id, never a truncated one', async () => {
    // A truncated id produces a reference that resolves to nothing.
    const { container } = render(CellToolbar, {
      props: { cellId: 'deadbeef', cellStatus: 'ok' as const },
    });
    await fireEvent.click(container.querySelector('.ref-chip')!);
    const copied = writeText.mock.calls[0][0] as string;
    expect(copied).toMatch(/^\$\{[a-f0-9]{8}\}$/);
  });

  it('confirms the copy in place', async () => {
    const { container } = render(CellToolbar, {
      props: { cellId: 'a1b2c3d4', cellStatus: 'ok' as const },
    });
    await fireEvent.click(container.querySelector('.ref-chip')!);
    expect(container.querySelector('.ref-chip')?.textContent).toContain(
      'copied',
    );
  });

  it('reports a denied clipboard rather than claiming success', async () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    writeText.mockRejectedValue(new Error('denied'));
    const { container } = render(CellToolbar, {
      props: { cellId: 'a1b2c3d4', cellStatus: 'ok' as const },
    });
    await fireEvent.click(container.querySelector('.ref-chip')!);
    expect(container.querySelector('.ref-chip')?.textContent).not.toContain(
      'copied',
    );
    expect(err).toHaveBeenCalled();
    err.mockRestore();
  });

  it('omits the chip for a cell later cells cannot reference', () => {
    const { container } = render(CellToolbar, {
      props: {
        cellId: 'a1b2c3d4',
        cellStatus: 'ok' as const,
        referencable: false,
      },
    });
    expect(container.querySelector('.ref-chip')).toBeNull();
  });
});

describe('CellToolbar actions', () => {
  it('shows Stop only while the cell is running', () => {
    const idle = render(CellToolbar, {
      props: { cellId: 'a1b2c3d4', cellStatus: 'ok' as const },
    });
    expect(idle.container.querySelector('.toolbar-btn.stop')).toBeNull();
    cleanup();

    const running = render(CellToolbar, {
      props: { cellId: 'a1b2c3d4', cellStatus: 'running' as const },
    });
    expect(running.container.querySelector('.toolbar-btn.stop')).toBeTruthy();
  });

  it('wires move and delete to their handlers', async () => {
    const onMoveUp = vi.fn();
    const onMoveDown = vi.fn();
    const onDelete = vi.fn();
    const { getByLabelText } = render(CellToolbar, {
      props: {
        cellId: 'a1b2c3d4',
        cellStatus: 'ok' as const,
        onMoveUp,
        onMoveDown,
        onDelete,
      },
    });
    await fireEvent.click(getByLabelText('Move cell up'));
    await fireEvent.click(getByLabelText('Move cell down'));
    await fireEvent.click(getByLabelText('Delete cell'));
    expect(onMoveUp).toHaveBeenCalledOnce();
    expect(onMoveDown).toHaveBeenCalledOnce();
    expect(onDelete).toHaveBeenCalledOnce();
  });

  it('cancels a running cell through the stop button', async () => {
    const onCancel = vi.fn();
    const { getByLabelText } = render(CellToolbar, {
      props: {
        cellId: 'a1b2c3d4',
        cellStatus: 'running' as const,
        onCancel,
      },
    });
    await fireEvent.click(getByLabelText('Stop cell'));
    expect(onCancel).toHaveBeenCalledOnce();
  });
});
