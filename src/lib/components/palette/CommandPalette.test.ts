// @vitest-environment jsdom
import { render, screen } from '@testing-library/svelte';
import { test, expect, vi } from 'vitest';
import CommandPalette from './CommandPalette.svelte';

test('sanitizes malicious SVG icons in command palette items', () => {
  const maliciousCommand = {
    id: 'cmd-1',
    label: 'Malicious Command',
    icon: '<svg><image href="x" onerror="alert(1)" /></svg>',
  };

  const { container } = render(CommandPalette, {
    props: {
      commands: [maliciousCommand],
      onSelect: vi.fn(),
      onClose: vi.fn(),
    },
  });

  expect(screen.getByText('Malicious Command')).toBeTruthy();
  const html = container.innerHTML;
  expect(html.toLowerCase()).not.toContain('onerror');
  expect(html.toLowerCase()).not.toContain('alert(1)');
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
