## 2025-02-12 - Prevent Repeated String Allocation in Loops
**Learning:** In a Svelte application with hierarchical trees mapping large underlying datasets (e.g., PostgreSQL databases, schemas, objects), rendering loops using `filter` / `map` can trigger massive numbers of `String.prototype.toLowerCase()` calls if the query normalization occurs inside the iteration block. This introduces a heavy toll (measurably up to ~5x overhead on large arrays).
**Action:** When filtering across large arrays (or loops iterating heavily nested components), always lift `query.toLowerCase()` (or `new RegExp`) outside the iteration loop. Using Svelte `$derived(searchQuery.toLowerCase())` is ideal.

## 2025-02-12 - Prevent Repeated String Allocation in UI Filters
**Learning:** In Svelte components with reactive list filters driven by keystrokes, running `toLowerCase()` on multiple properties of every list item during the filter phase causes massive unnecessary string allocations per keystroke. The parallel cache pattern (`item`, `lowerName`, etc.) effectively eliminates these allocations while preserving object identity.
**Action:** When implementing search filters over arrays of objects in Svelte, use a `$derived` parallel cache to store lowercased strings derived from the data, then filter against the cache and map back to `item`.

## 2025-02-12 - Side-Effect Free String Caching with WeakMap
**Learning:** Mutating input objects during search filtering to cache lowercased properties can cause `TypeError` crashes on frozen objects and corrupt state on object reference updates. Using a `WeakMap<object, string>` allows O(1) string caching keyed by object identity without mutating the objects or leaking memory.
**Action:** Use a `WeakMap<object, string>` in standalone utility functions when lowercasing object properties on repeated filter loops, keeping utility functions 100% side-effect free.
