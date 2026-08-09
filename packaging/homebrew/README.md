# Homebrew cask for Lucent

`lucent.rb` is a **template** — the live cask lives in the
[`banu-teja/homebrew-lucent`](https://github.com/banu-teja/homebrew-lucent)
tap repository and is maintained there after every release.

## Release flow

1. Tag `v<version>` and push (`.github/workflows/release.yml` builds, signs,
   notarizes, and uploads the runner-arch `.dmg` — `Lucent_<version>_aarch64.dmg`
   on current `macos-latest` — to a draft GitHub Release).
2. Publish the draft release.
3. Update the tap cask: replace `sha256` with `shasum -a 256 <dmg>` and bump
   `version` if it changed. Push to the tap repo.

## Verification (after first real release, on a clean Mac)

```sh
brew tap banu-teja/homebrew-lucent
brew install --cask lucent
```

Pass criteria: the app launches from `/Applications/Lucent.app`, and Gatekeeper
reports it signed + notarized (`spctl --assess --type execute --verbose
/Applications/Lucent.app`).

## Artifact naming note

tauri-bundler names artifacts `{productName}_{version}_{arch}.dmg` (underscores,
not hyphens) — e.g. `Lucent_0.1.0_aarch64.dmg` on Apple Silicon,
`Lucent_0.1.0_x64.dmg` on Intel. The cask's `url` must match the arch the
workflow actually produced (aarch64 today, single-arch build).
