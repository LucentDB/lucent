<div align="center">

<img src="src-tauri/icons/128x128@2x.png" width="96" alt="Lucent logo" />

# Lucent

**A fast, native desktop database GUI for PostgreSQL with an AI copilot that answers questions about your data — safely.**

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/banu-teja/lucent/actions/workflows/ci.yml/badge.svg)](https://github.com/banu-teja/lucent/actions/workflows/ci.yml)
![Platform](https://img.shields.io/badge/platform-macOS%20%E2%80%A2%20Windows%20%E2%80%A2%20Linux-lightgrey.svg)

</div>

---

## Why Lucent?

Lucent is a Postgres client built for people who are tired of slow, cluttered database
UIs. It combines the polish of a native desktop app with an AI copilot that is useful
**because it is safe**: read-only by default, every write gated behind an approval flow,
and its query answers are graded by an automated eval harness — not vibes.

- **Native, not Electron.** Rust + Tauri 2 + Svelte 5. Small, fast, and quiet.
- **AI that respects your data.** Read-only guardrails, DML approval, blast-radius checks
  and per-query timeouts. Your schema is indexed locally with embeddings; the LLM never
  sees rows you haven't opted to share.
- **Works with your own model.** OpenAI, Anthropic, or a **local Ollama** model — no API
  key, no data leaving your machine.
- **Focused on Postgres.** One database, done properly: SSH tunnels, keychain-stored
  credentials, deep schema introspection, and query history you can favorite.

## Features

### The SQL workspace

| | |
| --- | --- |
| **Connections** | Connection profiles with keychain-backed credentials and SSH tunneling |
| **Schema browser** | Schemas, tables, views, sequences, functions — with source views |
| **Editor** | CodeMirror SQL editor with syntax highlighting and multi-query execution |
| **Results grid** | Paged results with in-grid filtering and sorting, even on huge result sets |
| **Export** | CSV, JSON, and ready-to-run INSERT statements |
| **History** | Every query saved, searchable, and favoritable (`Cmd/Ctrl+K` to jump around) |

**Known limitations:** single statements only per query — multi-statement scripts (`SELECT 1; SELECT 2`) are rejected with an explicit error.

### The AI copilot (`Cmd/Ctrl+Shift+A`)

Ask questions in plain English — *"which customers churned last quarter and what did they have in common?"* — and the copilot turns them into SQL:

- **Schema-aware retrieval.** Your schema is indexed locally with embeddings (bge-small-en-v1.5), tiered with M-Schema, and clustered along foreign-key relationships so the agent finds the *right* tables.
- **Safe by default.** The agent runs read-only queries only, with a per-query timeout. Any INSERT/UPDATE/DELETE requires explicit approval.
- **Grounded answers.** Preflight literal probing, a join linter, and a blast-radius check catch wrong-column and cross-table mistakes before they reach you.
- **Verified, not vibes.** A headless eval harness grades retrieval accuracy and query correctness against a real Postgres — every change ships with measured results, not anecdotes.
- **Your keys, your models.** OpenAI, Anthropic, or a local Ollama endpoint. Configure provider, model, and limits in-app.

### SQL Notebooks

Turn exploration into a living document: mix markdown and SQL cells, run cells in any
order, restart sessions, and save/load notebooks as files. Great for onboarding
documentation, incident writeups, and reproducible analysis.

## Getting started

> 📦 **Installers coming soon** — Lucent is pre-release. Until the first signed build
> ships (macOS first, per our roadmap), run from source:

**Prerequisites:** Rust (stable) + the [Tauri 2 system dependencies](https://v2.tauri.app/start/prerequisites/) for your OS, Node.js ≥ 18, npm, and a Postgres instance to connect to.

```bash
npm install          # install frontend + Tauri CLI deps
npm run tauri dev    # launch the desktop app in dev mode
```

Then: **Connect** → add your Postgres connection (SSH tunnels supported) → open
**AI Settings** (`~/.lucent/ai-config.json`) and pick a provider.

### AI configuration

The API key uses a fallback chain: `~/.lucent/ai-key.txt` → environment variable →
OS keychain. Provider, model, and safety limits are configured in-app and persisted
to `~/.lucent/ai-config.json`.

| Setting | Default | What it does |
| --- | --- | --- |
| `rowLimit` | 500 | Max rows the agent may read per query |
| `aiQueryTimeoutSecs` | 60 | Statement timeout for AI read-only queries |
| `enableBlastRadiusCheck` | true | Warns before touching tables related to the ones you're writing |
| `enableSemanticIndex` | true | Local embedding index for schema retrieval |

## Architecture

Postgres drivers run as **separate supervised processes**, speaking a typed `bincode`
protocol over `0700`-permissioned Unix domain sockets. A crashed driver can't take down
the UI, query cancellation uses Postgres' native cancel protocol, and backpressure
keeps unread rows on the server instead of in memory.

```
lucent-protocol        # shared IPC message types + length-delimited framing
   ↑
lucent-worker-host     # Connector trait + generic serve loop
   ↑
lucent-driver-postgres # Postgres Connector impl + standalone worker binary
   ↑
src-tauri              # Tauri backend: commands, supervisor, AI, export, history
   ↑
src/                   # Svelte 5 frontend
```

## Development

| Task | Command |
| --- | --- |
| Frontend unit tests | `npm test` |
| Rust unit tests | `cd src-tauri && cargo test` |
| Integration tests (real Postgres via Docker) | `cd src-tauri && cargo test --features integration-tests` |
| LLM eval harness | `cd src-tauri && cargo test --features evals` |
| Type-check | `npm run check` |

CI (`.github/workflows/ci.yml`) runs the frontend and Rust unit tiers, `svelte-check`,
Prettier, and `cargo clippy -D warnings` on every push.

## Roadmap

- [x] Core SQL workspace + AI copilot
- [x] SQL notebooks
- [ ] Signed macOS installer + Homebrew cask (in progress)
- [ ] Windows and Linux installers
- [ ] First stable release (v0.1.0)

## Contributing

Bug reports, feature requests, and PRs are welcome — open an issue or a pull request.
This project is small and friendly; if you're unsure where to start, ask.

## License

[MIT](LICENSE)
