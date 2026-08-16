# DuckDB binary size

Measured on Fri Aug 14 2026, `arm64` (Apple Silicon), `cargo build --release`, `duckdb = "1.10505.0"`,
features = ["bundled"], no others. Final numbers from Task 11 (release binary measured 2026-08-15).

| Artifact                                | Size                                                                                                                                                                                                                                    |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lucent-driver-postgres` (baseline)     | 4,628,464 B (4.4 MiB)                                                                                                                                                                                                                   |
| `lucent-driver-duckdb` (release binary) | 42,847,136 B (40.9 MiB) — the real worker binary, measured after Task 11. Well under the 69.0 MiB `libduckdb_sys` rlib it was predicted from: debug symbols and release optimization close most of the gap.                             |
| `.app` bundle before DuckDB             | 56 MB (58,417,152 B)                                                                                                                                                                                                                    |
| `.app` bundle with DuckDB               | 56 MB (58,417,152 B)                                                                                                                                                                                                                    |
| **Installer delta**                     | **0 for the current bundle** — see note below. The real delta lands when worker binaries are bundled: **+40.9 MiB** per `lucent-driver-duckdb` binary (≈9.3× the 4.4 MiB `lucent-driver-postgres` worker; ≈0.7× the current 56 MB app). |

Build intermediates (`target/release/build/libduckdb-sys-*/out`): 183 MB of C++ objects; the
statically linked archive ships inside the worker binary, so the out-dir does not ship.

Note on the `.app` delta: Lucent currently bundles **no** worker binaries into the app (no Tauri
bundle resources/sidecars configured; packaging was removed on `main`). The DuckDB library is not
linked into the app binary either — that is the point of the worker-process seam. The before/after
`.app` numbers are therefore identical by construction; the cost of the second driver shows up in
the worker binary, which is **40.9 MiB** against the 4.4 MiB Postgres worker.

## Decision

**Accepted — 2026-08-14, by user (plan gate).** The second-driver cost is approved; proceed with the DuckDB driver. Rationale: the final release binary came in at **40.9 MiB** — ~40% under the 69 MiB rlib estimate and ~60% under the 107–112 MB prediction that motivated the original defer decision. The delta is deferred until worker bundling returns, and the value of a second driver proving the seam outweighs the size cost. This record is the precedent for the third driver's cost.

Lucent's positioning is "small, fast, quiet" (spec §10). This number is the
cost of the second driver. Record it so the third driver's cost is judged
against a real precedent rather than a guess.

## What was deliberately excluded

`parquet`, `json`, `httpfs`, `extensions-full`. Each adds a substantial C++
artifact and none is needed to query a `.duckdb` file. If DuckDB ever becomes a
compute engine (spec §9, explicitly out of scope), do Parquet I/O in `arrow-rs`
and hand Arrow across the C Data Interface rather than enabling these.
