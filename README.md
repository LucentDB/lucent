# Lucent

A fast, native desktop database GUI for PostgreSQL with an integrated AI copilot.

Lucent pairs a polished SQL workspace — connection profiles with keychain-backed
credentials, a schema browser, a CodeMirror SQL editor, a paged results grid with
filtering/sorting, and CSV/JSON/INSERT export — with an LLM agent that answers
questions about your data using schema retrieval, read-only safety guardrails, and
a DML approval flow.

## Architecture

Lucent runs database drivers as **separate supervised processes** that speak a typed
`bincode` protocol over `0700` Unix domain sockets. A crashed driver can't take down
the UI. Crates layer bottom-up:

```
lucent-protocol        # shared IPC message types + length-delimited framing
   ↑
lucent-worker-host     # Connector trait + generic serve loop
   ↑
lucent-driver-postgres # Postgres Connector impl + standalone worker binary
   ↑
src-tauri              # Tauri commands, process supervisor, AI subsystem,
                       # export, query history  (the desktop app backend)
   ↑
src/                   # Svelte 5 (runes) frontend
```

The AI subsystem (`src-tauri/src/ai/`) adds M-Schema tiering, FK-graph clustering,
preflight literal probing, and a join linter, plus a headless eval harness for
grading retrieval accuracy.

## Prerequisites

- **Rust** (stable) + the [Tauri 2 system dependencies](https://v2.tauri.app/start/prerequisites/) for your OS
- **Node.js** ≥ 18 and npm
- **PostgreSQL** to connect to (local or remote)

## Getting started

```bash
npm install          # install frontend + Tauri CLI deps
npm run tauri dev    # launch the desktop app in dev mode
```

Other scripts:

```bash
npm run build        # build the frontend bundle
npm run check        # svelte-check + TypeScript type-check
npm run format       # format src/ with Prettier
npm test             # frontend unit tests (Vitest)
```

## Testing

Tests are tiered so the default run needs no external services:

| Tier | Command | Needs |
|------|---------|-------|
| Frontend unit | `npm test` | nothing |
| Rust unit | `cd src-tauri && cargo test` | nothing |
| Postgres integration | `cd src-tauri && cargo test --features integration-tests` | Docker/Colima (spun up via testcontainers) |
| LLM eval | `cd src-tauri && cargo test --features evals` (some are `#[ignore]`d) | LLM provider credentials |

CI (`.github/workflows/ci.yml`) runs the frontend unit tier, `svelte-check`,
Prettier check, `cargo fmt --check`, `cargo clippy -D warnings`, and the Rust unit tier.

## AI configuration

The AI copilot reads its API key with a fallback chain: `~/.lucent/ai-key.txt` →
environment variable → OS keychain. Provider/model/limits are configured in-app via
**AI Settings** and persisted to `~/.lucent/ai-config.json`.

