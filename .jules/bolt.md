## 2025-02-12 - Prevent Repeated String Allocation in Loops
**Learning:** In a Svelte application with hierarchical trees mapping large underlying datasets (e.g., PostgreSQL databases, schemas, objects), rendering loops using `filter` / `map` can trigger massive numbers of `String.prototype.toLowerCase()` calls if the query normalization occurs inside the iteration block. This introduces a heavy toll (measurably up to ~5x overhead on large arrays).
**Action:** When filtering across large arrays (or loops iterating heavily nested components), always lift `query.toLowerCase()` (or `new RegExp`) outside the iteration loop. Using Svelte `$derived(searchQuery.toLowerCase())` is ideal.
## 2026-08-09 - Cached string operations outside reactive loops
**Learning:** String operations inside reactive array methods (like `filter`) evaluate on every iteration. This causes O(N) string allocations during rapid updates (e.g. searching/filtering).
**Action:** Use `$derived` to cache the result of static transformations (e.g., `query.toLowerCase()`) outside the loop, saving processing time and allocations.
