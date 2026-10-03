// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import CommandPalette from './CommandPalette.svelte';

describe('CommandPalette XSS prevention', () => {
  afterEach(() => {
    cleanup();
  });

  it('renders standard preset icons safely', () => {
    const commands = [{ id: '1', label: 'Open Terminal', icon: 'terminal' }];
    render(CommandPalette, {
      commands,
      onSelect: vi.fn(),
      onClose: vi.fn(),
    });

    const label = screen.getByText('Open Terminal');
    expect(label).not.toBeNull();
    const item = label.closest('button');
    expect(item).not.toBeNull();
    expect(item?.querySelector('svg')).not.toBeNull();
  });

  it('sanitizes malicious SVG icon inputs containing scripts or event handlers', () => {
    const commands = [
      {
        id: '2',
        label: 'Unsafe Command',
        icon: '<svg><g><script>alert(1)</script><rect onerror="alert(2)" width="10" height="10"/></g></svg>',
      },
    ];
    render(CommandPalette, {
      commands,
      onSelect: vi.fn(),
      onClose: vi.fn(),
    });

    const label = screen.getByText('Unsafe Command');
    expect(label).not.toBeNull();
    const item = label.closest('button');
    expect(item).not.toBeNull();
    const iconSpan = item?.querySelector('.item-icon');
    expect(iconSpan).not.toBeNull();
    const html = iconSpan?.innerHTML.toLowerCase() || '';

    expect(html).not.toContain('<script');
    expect(html).not.toContain('onerror');
  });
});
