// Shared vitest setup. Loaded once per test file via vitest.config.ts.
//
// jsdom implements neither ResizeObserver nor IntersectionObserver. Svelte 5's
// `bind:clientWidth` (used by the results grid to gate pin menu items on
// container width) is backed by ResizeObserver, so every component test that
// mounts the grid needs the stub to exist.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}

globalThis.ResizeObserver ??= ResizeObserverStub;
