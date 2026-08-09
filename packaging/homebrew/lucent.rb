# Lucent Homebrew cask template.
#
# The live cask lives in the `banu-teja/homebrew-lucent` tap repository.
# This file is the template: on the first real release, replace the sha256
# placeholder with the actual digest of the uploaded .dmg and push the cask
# to the tap.
#
# Artifact naming: tauri-bundler produces `{productName}_{version}_{arch}.dmg`
# (underscores). Product name "Lucent" comes from src-tauri/tauri.conf.json.
# The release workflow (macos-latest) builds ONE arch: Apple Silicon
# (Lucent_<version>_aarch64.dmg) unless a build matrix is added. Point this
# at the arch the workflow actually produces; on_arch conditionals can cover
# both once a matrix exists.

cask "lucent" do
  version "0.1.0"
  sha256 "<sha256 of Lucent_0.1.0_aarch64.dmg>"  # replace with shasum -a 256 output before brew audit
  url "https://github.com/banu-teja/lucent/releases/download/v#{version}/Lucent_#{version}_aarch64.dmg"
  name "Lucent"
  desc "Native Postgres GUI with an AI copilot"
  homepage "https://github.com/banu-teja/lucent"
  app "Lucent.app"
end
