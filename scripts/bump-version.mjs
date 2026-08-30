#!/usr/bin/env node
// Single entry point for a release bump: `node scripts/bump-version.mjs 0.2.0`
import { readFileSync, writeFileSync } from 'node:fs';

const next = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(next ?? '')) {
  console.error('usage: node scripts/bump-version.mjs <major.minor.patch>');
  process.exit(2);
}

// package.json and tauri.conf.json are JSON, but rewriting them through
// JSON.stringify would reflow the whole file. Patch the one line instead so
// the diff shows exactly what changed.
for (const file of ['package.json', 'src-tauri/tauri.conf.json']) {
  const text = readFileSync(file, 'utf8');
  const patched = text.replace(/("version"\s*:\s*")[^"]+(")/, `$1${next}$2`);
  if (patched === text) {
    console.error(`${file}: no "version" field to patch`);
    process.exit(1);
  }
  writeFileSync(file, patched);
}

const cargo = readFileSync('Cargo.toml', 'utf8');
const patchedCargo = cargo.replace(
  /(\[workspace\.package\][\s\S]*?version\s*=\s*")[^"]+(")/,
  `$1${next}$2`,
);
if (patchedCargo === cargo) {
  console.error('Cargo.toml: no [workspace.package] version to patch');
  process.exit(1);
}
writeFileSync('Cargo.toml', patchedCargo);

console.log(`bumped to ${next}`);
