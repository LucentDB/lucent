## 2024-06-25 - Avoid String Allocations in Reactive Loops
**Learning:** Calling methods like `toLowerCase()` or `trim()` repeatedly inside Svelte `$derived` blocks that loop over data (e.g., `.filter` or `.map`) causes unnecessary memory allocations and performance overhead, especially on large datasets.
**Action:** Always cache the computed string result outside of the loop using another `$derived` variable or immediately before the loop execution to prevent massive performance overhead.
