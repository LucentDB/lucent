## 2025-02-12 - Prevent Repeated String Allocation in Loops
**Learning:** In a Svelte application with hierarchical trees mapping large underlying datasets (e.g., PostgreSQL databases, schemas, objects), rendering loops using `filter` / `map` can trigger massive numbers of `String.prototype.toLowerCase()` calls if the query normalization occurs inside the iteration block. This introduces a heavy toll (measurably up to ~5x overhead on large arrays).
**Action:** When filtering across large arrays (or loops iterating heavily nested components), always lift `query.toLowerCase()` (or `new RegExp`) outside the iteration loop. Using Svelte `$derived(searchQuery.toLowerCase())` is ideal.

## 2025-02-13 - Number.toLocaleString() instantiation overhead
**Learning:** In heavily repeated loops (e.g., rendering hundreds/thousands of grid cells), using `Number.toLocaleString()` introduces a massive performance bottleneck because it instantiates a new `Intl.NumberFormat` object on every invocation under the hood. Tests show an overhead of ~700ms for 10,000 calls vs ~10ms when reusing a formatter.
**Action:** Always extract and reuse `Intl.NumberFormat` instances outside of frequently executed loops or formatting functions instead of relying on inline `toLocaleString()` calls.
