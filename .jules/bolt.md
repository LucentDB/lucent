## 2025-02-12 - Prevent Repeated String Allocation in Loops
**Learning:** In a Svelte application with hierarchical trees mapping large underlying datasets (e.g., PostgreSQL databases, schemas, objects), rendering loops using `filter` / `map` can trigger massive numbers of `String.prototype.toLowerCase()` calls if the query normalization occurs inside the iteration block. This introduces a heavy toll (measurably up to ~5x overhead on large arrays).
**Action:** When filtering across large arrays (or loops iterating heavily nested components), always lift `query.toLowerCase()` (or `new RegExp`) outside the iteration loop. Using Svelte `$derived(searchQuery.toLowerCase())` is ideal.

## 2025-02-12 - Number Formatting in Large Grids
**Learning:** Using `toLocaleString()` with options (e.g., `{ minimumFractionDigits: 2 }`) is notoriously slow because it re-instantiates an internal formatter and parses the options every time. In a large data grid rendering hundreds of cells with floating-point numbers, this causes massive blocking overhead on the main thread (up to 50x slower than reusing an `Intl.NumberFormat` instance).
**Action:** Always cache and reuse `Intl.NumberFormat` instances when formatting numbers inside loops or large repeating component trees like grids.
