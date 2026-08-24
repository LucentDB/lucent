import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/svelte';
import DmlApprovalCard from './DmlApprovalCard.svelte';

afterEach(cleanup);

const dml = { sql: 'delete from t', description: 'Remove stale rows', estimatedRowsAffected: 12 };

describe('DmlApprovalCard states', () => {
  it('shows the awaiting caption while waiting for the decision', () => {
    render(DmlApprovalCard, { dml, onRun: vi.fn(), onCancel: vi.fn() });
    expect(screen.getByText('Awaiting your decision')).toBeTruthy();
    expect(screen.getByText('Execute')).toBeTruthy();
  });

  it('shows the executed state when a result arrived', () => {
    const { container } = render(DmlApprovalCard, { dml, result: 3, onRun: vi.fn(), onCancel: vi.fn() });
    expect(screen.getByText('DML Executed')).toBeTruthy();
    expect(container.querySelector('.dml-result')?.textContent).toMatch(/3\s+rows affected/);
  });

  it('shows the error state when execution failed', () => {
    render(DmlApprovalCard, { dml, error: 'stale preview', onRun: vi.fn(), onCancel: vi.fn() });
    expect(screen.getByText('stale preview')).toBeTruthy();
    expect(screen.queryByText('Awaiting your decision')).toBeNull();
  });
});
