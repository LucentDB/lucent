# Homebrew tap for Lucent

## Install

```bash
brew install --cask lucentdb/lucent/lucent
```

Or tap first, then install:

```bash
brew tap lucentdb/lucent
brew install --cask lucent
```

Upgrade with `brew upgrade --cask lucent`, remove with
`brew uninstall --cask lucent` (add `--zap` to remove Lucent's data too).

## First launch

Lucent ships without an Apple Developer ID, so macOS blocks its first launch.
This is expected for an independent app without a paid signing certificate —
not a sign that anything is wrong.

To allow it once: try to open Lucent, then go to **System Settings → Privacy &
Security**, scroll down, and click **Open Anyway**. Later launches, and every
`brew upgrade`, need no further action.

## How this tap is maintained

`Casks/lucent.rb` is generated — do not edit it by hand. When a release is
published in [`LucentDB/lucent`](https://github.com/LucentDB/lucent), the
`Update Homebrew tap` workflow there downloads both DMGs, hashes them, renders
the cask, audits it with `brew audit --cask --strict --online`, and pushes the
result here. Commits are made by `github-actions[bot]`.

The cask template and the workflow both live in the main repository:

- `.github/tap/lucent.rb.template`
- `.github/workflows/tap.yml`
