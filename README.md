<div align="center">

<img src="src-tauri/icons/lucent.png" width="96" alt="Lucent logo" />

# Lucent

**A fast, native desktop database GUI for PostgreSQL with an AI copilot that answers questions about your data — safely.**

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/LucentDB/lucent/actions/workflows/ci.yml/badge.svg)](https://github.com/LucentDB/lucent/actions/workflows/ci.yml)
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
- **Or bring your own agent.** No API key? Install a coding agent (opencode, Claude Code, …) from the ACP registry in AI Settings, and Lucent's four database tools reach it through a bundled MCP bridge — the same read-only guardrails, row caps, and DML approval keep running in Lucent.

### SQL Notebooks

Turn exploration into a living document: mix markdown and SQL cells, run cells in any
order, restart sessions, and save/load notebooks as files. Great for onboarding
documentation, incident writeups, and reproducible analysis.

## Getting started

### Install

**macOS** (Apple Silicon or Intel):

```bash
brew install --cask lucentdb/lucent/lucent
```

Use the fully qualified name. Homebrew 6 and later refuse to load a cask from a
tap they have not been told to trust, and naming the cask explicitly is what
grants that trust — `brew tap lucentdb/lucent && brew install --cask lucent`
fails with `Refusing to load cask ... from untrusted tap` unless you run
`brew trust lucentdb/lucent` first.

Prefer a disk image? Download `Lucent_<version>_aarch64.dmg` (Apple Silicon) or
`Lucent_<version>_x64.dmg` (Intel) from the
[latest release](https://github.com/LucentDB/lucent/releases/latest).

**Windows**: download `Lucent_<version>_x64-setup.exe` from the
[latest release](https://github.com/LucentDB/lucent/releases/latest). A winget
manifest exists in `packaging/winget/` but is not published to the community
repository yet, so `winget install` is not available.

**Linux** (x86_64): download from the
[latest release](https://github.com/LucentDB/lucent/releases/latest) and pick
one.

```bash
# .deb — Debian 12+, Ubuntu 22.04+. Use `apt install ./file.deb`, not `dpkg -i`:
# only apt resolves the dependencies from your own distro.
sudo apt install ./Lucent_<version>_amd64.deb

# AppImage — anything with WebKitGTK 4.1 available
chmod +x Lucent_<version>_amd64.AppImage && ./Lucent_<version>_amd64.AppImage
```

The Linux binaries are built on Ubuntu 22.04 for the widest glibc floor, which
means they need WebKitGTK 4.1: Ubuntu 22.04+/Debian 12+. Ubuntu 20.04 and
Debian 11 have no WebKitGTK 4.1 package and cannot run them.

Lucent checks for new versions when it starts. macOS and Windows installs, and
the AppImage, can update themselves in place; `.deb` users upgrade through their
package manager.

#### First launch

Lucent is an independent project without a paid Apple or Microsoft signing
certificate, so both systems ask for confirmation the first time. This is
expected, not a sign that anything is wrong.

- **macOS** — the first launch is refused. Go to **System Settings → Privacy
  & Security**, scroll down, and click **Open Anyway**.
- **Windows** — SmartScreen shows "Windows protected your PC". Click **More
  info**, then **Run anyway**.

#### First connect

The AI copilot's schema search downloads a ~33MB embedding model
(bge-small-en-v1.5) the first time you connect. Everything else works
offline; if the download fails, semantic search stays unavailable until it
succeeds and the rest of the app is unaffected.

### Run from source

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

No API key? The **Agents (ACP)** panel in AI Settings installs coding agents
(opencode, Claude Code, …) straight from the ACP registry — a snapshot ships with the
app so the list works offline — and the installed agent shows up in the provider
picker. ACP needs no key: the agent owns its own auth and model choice.

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

### Releases

Tagged releases are built by `.github/workflows/release.yml`, which builds all
four targets and attaches them to a single **draft** GitHub Release. macOS is
ad-hoc signed and never notarized; Windows and Linux are unsigned.

The database driver workers (`lucent-driver-postgres`, `lucent-driver-duckdb`)
and the ACP bridge binary (`lucent-db-tools-mcp`) ship inside the bundle as Tauri
sidecars. `scripts/stage-sidecars.mjs` builds and stages them under the **target**
triple — not the host triple, which would ship an arm64 bridge in an Intel bundle
— and `tauri build` runs it automatically via `beforeBuildCommand`. Fresh-checkout
dev and test runs (`cargo test`, `cargo check`) bypass the sidecar check in
`build.rs` when the binaries have not been staged yet, keeping development fast
and clean.

To build a release bundle locally:

```bash
npm run build:app                              # host target
npx tauri build --target x86_64-apple-darwin   # cross-target, same staging flow
```

Building an updater-enabled target (`app`, `nsis`, `appimage`) also requires
`TAURI_SIGNING_PRIVATE_KEY` — the minisign key the app verifies updates against.
**That key is the release identity: lose it and no existing install can ever
auto-update again.** `--bundles dmg` alone needs no key, because a DMG is not
updater-enabled.

Publishing a release also updates the install channels, each on the `released`
trigger (a full release — never a prerelease):

| Workflow | Channel | One-time setup |
| --- | --- | --- |
| `tap.yml` | Homebrew cask in `LucentDB/homebrew-lucent` | create that repository, add a `HOMEBREW_TAP_TOKEN` PAT |
| `winget.yml` | manifest PR to `microsoft/winget-pkgs` | fork it under `LucentDB`, add `WINGET_TOKEN`, sign Microsoft's CLA, set the variable `WINGET_ENABLED=true` |
| `apt.yml` | signed apt repository on GitHub Pages | add the `APT_GPG_*` secrets, enable Pages, set the variable `APT_ENABLED=true` |

`tap.yml` is unconditional and fails loudly when its token is missing; the other
two are opt-in through their repository variable, so a channel that has not been
set up yet cannot turn a release red. Chocolatey is deliberately not included:
`choco install` runs elevated, which would install Tauri's per-user NSIS package
into the administrator's profile rather than the user who asked for it.

## Roadmap

- [x] Core SQL workspace + AI copilot
- [x] SQL notebooks
- [x] Release pipeline: draft releases for macOS, Windows and Linux, plus in-app updates
- [x] Homebrew cask, tap layout and audit-gated tap workflow
- [ ] Publish the first tagged release, and create `LucentDB/homebrew-lucent`
- [ ] winget manifest in the community repository
- [ ] Scoop bucket (needs a portable `.zip` artifact)
- [ ] Signed apt repository on GitHub Pages
- [ ] Notarized macOS build (needs a paid Apple Developer ID)

## Contributing

Bug reports, feature requests, and PRs are welcome — open an issue or a pull request.
This project is small and friendly; if you're unsure where to start, ask.

## License

[MIT](LICENSE)
