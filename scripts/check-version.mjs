#!/usr/bin/env node
// Asserts the three version sources agree. They drift silently otherwise:
// tauri-action names the tag and every artifact from tauri.conf.json, so a
// stale Cargo.toml ships a binary whose --version disagrees with its filename.
import { readFileSync } from 'node:fs';

const pkg = JSON.parse(readFileSync('package.json', 'utf8')).version;

const tauri = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).version;

const workspace = readFileSync('Cargo.toml', 'utf8').match(
  /\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/,
)?.[1];

const sources = {
  'package.json': pkg,
  'tauri.conf.json': tauri,
  'Cargo.toml': workspace,
};
const distinct = [...new Set(Object.values(sources))];

if (distinct.length !== 1 || !distinct[0]) {
  console.error('version sources disagree:');
  for (const [file, v] of Object.entries(sources)) {
    console.error(`  ${file}: ${v ?? '(not found)'}`);
  }
  process.exit(1);
}

console.log(`version ${distinct[0]} consistent across ${Object.keys(sources).length} sources`);
