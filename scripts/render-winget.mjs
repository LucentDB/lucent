#!/usr/bin/env node
// Renders the winget manifests for one release.
//
//   node scripts/render-winget.mjs <version> <installer-sha256> [release-date] [outdir]
//
// The manifests cannot be committed as static files: `InstallerSha256` and
// `ReleaseDate` change with every release, and a stale hash is a guaranteed
// `Error-Hash-Mismatch` rejection from the winget validation pipeline. So the
// committed artifacts are templates, and this renders them from the bytes that
// were actually published.
//
// Winget also requires exactly one package version per pull request, so the
// renderer writes a flat directory ready to be dropped into
// `manifests/l/LucentDB/Lucent/<version>/`.
import { readdirSync, readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [version, sha256, releaseDate = new Date().toISOString().slice(0, 10), outDir = 'packaging/winget/out'] =
  process.argv.slice(2);

if (!/^\d+\.\d+\.\d+$/.test(version ?? '')) {
  console.error('usage: node scripts/render-winget.mjs <version> <sha256> [release-date] [outdir]');
  process.exit(2);
}

// A wrong hash is not caught locally — it surfaces as a failed PR days later —
// so reject anything that is not a SHA256 before writing it into a manifest.
if (!/^[a-f0-9]{64}$/i.test(sha256 ?? '')) {
  console.error(`sha256 must be 64 hex characters, got: ${sha256 ?? '(missing)'}`);
  process.exit(2);
}

if (!/^\d{4}-\d{2}-\d{2}$/.test(releaseDate)) {
  console.error(`release date must be YYYY-MM-DD, got: ${releaseDate}`);
  process.exit(2);
}

const tag = `v${version}`;
const templateDir = 'packaging/winget';
const substitutions = {
  __VERSION__: version,
  __SHA256__: sha256.toLowerCase(),
  __TAG__: tag,
  __RELEASE_DATE__: releaseDate,
};

mkdirSync(outDir, { recursive: true });

const templates = readdirSync(templateDir).filter((name) => name.endsWith('.template'));
if (templates.length === 0) {
  console.error(`no *.template files found in ${templateDir}`);
  process.exit(1);
}

for (const name of templates) {
  let text = readFileSync(join(templateDir, name), 'utf8');
  for (const [placeholder, value] of Object.entries(substitutions)) {
    text = text.replaceAll(placeholder, value);
  }

  // A placeholder left behind means a manifest would ship with literal `__X__`
  // in a field winget validates. Fail here rather than in a review queue.
  const leftover = text.match(/__[A-Z_]+__/);
  if (leftover) {
    console.error(`${name}: unsubstituted placeholder ${leftover[0]}`);
    process.exit(1);
  }

  const outPath = join(outDir, name.replace(/\.template$/, ''));
  writeFileSync(outPath, text);
  console.log(`rendered ${outPath}`);
}

console.log(`\nSubmit these for LucentDB.Lucent ${version}:`);
console.log(`  manifests/l/LucentDB/Lucent/${version}/`);
