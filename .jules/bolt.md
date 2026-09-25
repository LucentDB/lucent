## 2025-02-12 - Prevent Repeated String Allocation in Loops
**Learning:** In a Svelte application with hierarchical trees mapping large underlying datasets (e.g., PostgreSQL databases, schemas, objects), rendering loops using `filter` / `map` can trigger massive numbers of `String.prototype.toLowerCase()` calls if the query normalization occurs inside the iteration block. This introduces a heavy toll (measurably up to ~5x overhead on large arrays).
**Action:** When filtering across large arrays (or loops iterating heavily nested components), always lift `query.toLowerCase()` (or `new RegExp`) outside the iteration loop. Using Svelte `$derived(searchQuery.toLowerCase())` is ideal.

## 2025-02-12 - Prevent Repeated String Allocation in UI Filters
**Learning:** In Svelte components with reactive list filters driven by keystrokes, running `toLowerCase()` on multiple properties of every list item during the filter phase causes massive unnecessary string allocations per keystroke. The parallel cache pattern (`item`, `lowerName`, etc.) effectively eliminates these allocations while preserving object identity.
**Action:** When implementing search filters over arrays of objects in Svelte, use a `$derived` parallel cache to store lowercased strings derived from the data, then filter against the cache and map back to `item`.

## 2025-02-12 - Prevent Repeated Table Row Model Lookups in Grid Body Loops
**Learning:** In Svelte 5 data grids wrapping TanStack Table, calling `table.getRowModel()` inside an `{#each}` iteration loop invokes the table row model accessor $N$ times per render ($N$ = page size). Extracting `const allTableRows = $derived(table.getRowModel().rows)` at component scope reduces row model getter invocations from $O(N)$ per render pass to $O(1)$.
**Action:** When rendering table rows from a TanStack Table instance in Svelte 5, derive `$derived(table.getRowModel().rows)` at top-level component scope instead of calling `table.getRowModel()` inside the row loop.
