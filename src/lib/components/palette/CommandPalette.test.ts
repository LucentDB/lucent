import { describe, test, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import CommandPalette from './CommandPalette.svelte';

describe('CommandPalette', () => {
  test('renders command items and sanitizes custom SVG icons containing scripts', () => {
    const commands = [
      {
        id: '1',
        label: 'Safe Command',
        icon: 'terminal',
      },
      {
        id: '2',
        label: 'Untrusted Command',
        icon: '<svg><script>alert("xss")</script><circle cx="5" cy="5" r="5"/></svg>',
      },
    ];

    const { container } = render(CommandPalette, {
      props: {
        commands,
        onSelect: vi.fn(),
        onClose: vi.fn(),
      },
    });

    expect(screen.getByText('Safe Command')).toBeTruthy();
    expect(screen.getByText('Untrusted Command')).toBeTruthy();

    // Script tag in the malicious icon should be sanitized away by DOMPurify
    expect(container.querySelector('script')).toBeNull();
    // Circle element should survive sanitization
    expect(container.querySelector('circle[cx="5"]')).not.toBeNull();
  });
});
