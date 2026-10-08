## 2025-02-12 - Prevent Repeated String Allocation in Loops
**Learning:** In a Svelte application with hierarchical trees mapping large underlying datasets (e.g., PostgreSQL databases, schemas, objects), rendering loops using `filter` / `map` can trigger massive numbers of `String.prototype.toLowerCase()` calls if the query normalization occurs inside the iteration block. This introduces a heavy toll (measurably up to ~5x overhead on large arrays).
**Action:** When filtering across large arrays (or loops iterating heavily nested components), always lift `query.toLowerCase()` (or `new RegExp`) outside the iteration loop. Using Svelte `$derived(searchQuery.toLowerCase())` is ideal.

## 2025-02-12 - Prevent Repeated String Allocation in UI Filters
**Learning:** In Svelte components with reactive list filters driven by keystrokes, running `toLowerCase()` on multiple properties of every list item during the filter phase causes massive unnecessary string allocations per keystroke. The parallel cache pattern (`item`, `lowerName`, etc.) effectively eliminates these allocations while preserving object identity.
**Action:** When implementing search filters over arrays of objects in Svelte, use a `$derived` parallel cache to store lowercased strings derived from the data, then filter against the cache and map back to `item`.

## 2025-05-20 - Avoid Eager Computation across Collapsed Tree Nodes
**Learning:** Lifting nested tree filters into top-level derived state across all nodes (e.g. filtering all schemas in all databases upfront) causes eager evaluation for collapsed branches. This leads to performance regressions and unnecessary array allocations compared to lazy inline filtering evaluated only when nodes are expanded.
**Action:** For hierarchical tree views, keep filtering lazy inside expanded branch conditional blocks, or derive filtered subsets lazily per expanded branch.
