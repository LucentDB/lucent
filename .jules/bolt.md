## 2025-02-12 - Prevent Repeated String Allocation in Loops
**Learning:** In a Svelte application with hierarchical trees mapping large underlying datasets (e.g., PostgreSQL databases, schemas, objects), rendering loops using `filter` / `map` can trigger massive numbers of `String.prototype.toLowerCase()` calls if the query normalization occurs inside the iteration block. This introduces a heavy toll (measurably up to ~5x overhead on large arrays).
**Action:** When filtering across large arrays (or loops iterating heavily nested components), always lift `query.toLowerCase()` (or `new RegExp`) outside the iteration loop. Using Svelte `$derived(searchQuery.toLowerCase())` is ideal.

## 2025-02-13 - Optimize Connection List Search Filtering
**Learning:** In list filtering operations on multiple object properties, continuously calling `.toLowerCase()` on string properties inside a filter loop causes heavy allocation per keystroke/render cycle.
**Action:** Lift `.toLowerCase()` outside the active filter loop by precomputing lowercased string properties using a parallel cache derived pattern (e.g., `$derived(profiles.map(p => ({ profile: p, nameL: p.name.toLowerCase(), ... })))`). This avoids per-keystroke allocations, drastically speeding up list filtering while preserving object identity.
