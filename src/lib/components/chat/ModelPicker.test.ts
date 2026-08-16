import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/svelte';
import ModelPicker from './ModelPicker.svelte';

afterEach(cleanup);

const MODELS = [
  { id: 'gpt-4o', displayName: 'gpt-4o' },
  { id: 'gpt-4o-mini', displayName: 'gpt-4o-mini' },
  { id: 'o3', displayName: 'o3' },
];

describe('ModelPicker', () => {
  it('shows an idle placeholder before any fetch', () => {
    render(ModelPicker, {
      status: 'idle',
      models: [],
      value: '',
      onChange: vi.fn(),
      providerLabel: 'OpenAI',
    });
    expect(screen.getByText(/Fetch Models to load/)).toBeTruthy();
    // Manual fallback is always present, even before the first fetch.
    expect(screen.getByLabelText(/model name/i)).toBeTruthy();
  });

  it('shows skeleton rows while loading', () => {
    render(ModelPicker, {
      status: 'loading',
      models: [],
      value: '',
      onChange: vi.fn(),
      providerLabel: 'OpenAI',
    });
    expect(screen.getAllByTestId('model-skeleton-row')).toHaveLength(4);
  });

  it('shows a searchable list and count on success', async () => {
    render(ModelPicker, {
      status: 'success',
      models: MODELS,
      value: '',
      onChange: vi.fn(),
      providerLabel: 'OpenAI',
    });
    expect(screen.getByText('3 models')).toBeTruthy();
    await fireEvent.input(screen.getByPlaceholderText(/search models/i), {
      target: { value: 'mini' },
    });
    expect(screen.getByText('gpt-4o-mini')).toBeTruthy();
    expect(screen.queryByText('o3')).toBeNull();
  });

  it('calls onChange when a model is picked', async () => {
    const onChange = vi.fn();
    render(ModelPicker, {
      status: 'success',
      models: MODELS,
      value: '',
      onChange,
      providerLabel: 'OpenAI',
    });
    await fireEvent.click(screen.getByText('o3'));
    expect(onChange).toHaveBeenCalledWith('o3');
  });

  it('falls back to a manual text field on error, without blocking entry', () => {
    render(ModelPicker, {
      status: 'error',
      models: [],
      value: '',
      onChange: vi.fn(),
      providerLabel: 'Custom',
      errorMessage:
        "This endpoint doesn't return a model list — type the model name directly below.",
    });
    expect(screen.getByText(/doesn't return a model list/)).toBeTruthy();
    expect(screen.getByLabelText(/model name/i)).toBeTruthy();
  });

  it('navigates the success list with ArrowDown then Enter', async () => {
    const onChange = vi.fn();
    render(ModelPicker, {
      status: 'success',
      models: MODELS,
      value: '',
      onChange,
      providerLabel: 'OpenAI',
    });

    const searchInput = screen.getByPlaceholderText(/search models/i);
    await fireEvent.keyDown(searchInput, { key: 'ArrowDown' });
    await fireEvent.keyDown(searchInput, { key: 'Enter' });

    // ArrowDown moves to gpt-4o-mini (index 1 in the 3-model list)
    expect(onChange).toHaveBeenCalledWith('gpt-4o-mini');
  });

  it('filters then picks with ArrowDown+Enter within the filtered set', async () => {
    const onChange = vi.fn();
    render(ModelPicker, {
      status: 'success',
      models: MODELS,
      value: '',
      onChange,
      providerLabel: 'OpenAI',
    });

    const searchInput = screen.getByPlaceholderText(/search models/i);
    await fireEvent.input(searchInput, { target: { value: 'mini' } });
    // Only gpt-4o-mini matches; ArrowDown then Enter picks it
    await fireEvent.keyDown(searchInput, { key: 'ArrowDown' });
    await fireEvent.keyDown(searchInput, { key: 'Enter' });

    expect(onChange).toHaveBeenCalledWith('gpt-4o-mini');
  });
});
