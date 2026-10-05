// @vitest-environment jsdom
import { render, screen } from '@testing-library/svelte';
import { test, expect, vi } from 'vitest';
import CommandPalette from './CommandPalette.svelte';

test('sanitizes malicious SVG icons in command palette items', () => {
  const maliciousCommand = {
    id: 'cmd-1',
    label: 'Malicious Command',
    icon: '<svg><script>alert(1)</script><foreignObject><iframe src="javascript:alert(1)"></iframe></foreignObject><image href="x" onerror="alert(1)" onload="alert(2)" /></svg>',
  };

  const { container } = render(CommandPalette, {
    props: {
      commands: [maliciousCommand],
      onSelect: vi.fn(),
      onClose: vi.fn(),
    },
  });

  expect(screen.getByText('Malicious Command')).toBeTruthy();
  const html = container.innerHTML.toLowerCase();
  expect(html).not.toContain('script');
  expect(html).not.toContain('foreignobject');
  expect(html).not.toContain('iframe');
  expect(html).not.toContain('onerror');
  expect(html).not.toContain('onload');
  expect(html).not.toContain('alert(1)');
});

test('falls back to default icon when icon id is unknown string', () => {
  const unknownIconCommand = {
    id: 'cmd-3',
    label: 'Unknown Icon Command',
    icon: 'unknown_icon_name',
  };

  const { container } = render(CommandPalette, {
    props: {
      commands: [unknownIconCommand],
      onSelect: vi.fn(),
      onClose: vi.fn(),
    },
  });

  expect(screen.getByText('Unknown Icon Command')).toBeTruthy();
  const html = container.innerHTML;
  expect(html).toContain('polyline points="9 18 15 12 9 6"');
  expect(html).not.toContain('unknown_icon_name');
});

test('renders standard icon IDs safely', () => {
  const standardCommand = {
    id: 'cmd-2',
    label: 'Terminal Command',
    icon: 'terminal',
  };

  const { container } = render(CommandPalette, {
    props: {
      commands: [standardCommand],
      onSelect: vi.fn(),
      onClose: vi.fn(),
    },
  });

  expect(screen.getByText('Terminal Command')).toBeTruthy();
  const html = container.innerHTML;
  expect(html).toContain('<svg');
  expect(html).toContain('polyline');
});
