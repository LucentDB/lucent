#!/usr/bin/env node

import { execSync } from 'node:child_process';
import { copyFileSync, chmodSync, existsSync, mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = fileURLToPath(new URL('.', import.meta.url));
const rootDir = resolve(__dirname, '..');
const tauriDir = join(rootDir, 'src-tauri');
const binariesDir = join(tauriDir, 'binaries');

function getTargetTriple() {
  if (process.env.TARGET) {
    return process.env.TARGET.trim();
  }
  if (process.env.TAURI_ENV_TARGET_TRIPLE) {
    return process.env.TAURI_ENV_TARGET_TRIPLE.trim();
  }
  const output = execSync('rustc -vV', { encoding: 'utf8' });
  const match = output.match(/^host:\s*(.+)$/m);
  if (!match) {
    throw new Error('Failed to parse host target triple from rustc -vV');
  }
  return match[1].trim();
}

const triple = getTargetTriple();
const isWindows = triple.includes('windows') || process.platform === 'win32';
const ext = isWindows ? '.exe' : '';

const explicitTarget = process.env.TARGET || process.env.TAURI_ENV_TARGET_TRIPLE;
const targetFlag = explicitTarget ? ` --target ${triple}` : '';
const targetReleaseDir = explicitTarget ? join(rootDir, 'target', triple, 'release') : join(rootDir, 'target', 'release');

console.log(`[stage-sidecars] Building release worker binaries for target: ${triple}`);

// Build the release binaries
execSync(
  `cargo build --release${targetFlag} -p lucent-driver-postgres -p lucent-driver-duckdb -p lucent --bin lucent-db-tools-mcp`,
  {
    cwd: rootDir,
    stdio: 'inherit',
  }
);

if (!existsSync(binariesDir)) {
  mkdirSync(binariesDir, { recursive: true });
}

const sidecars = [
  'lucent-driver-postgres',
  'lucent-driver-duckdb',
  'lucent-db-tools-mcp',
];

for (const name of sidecars) {
  const src = join(targetReleaseDir, `${name}${ext}`);
  const dest = join(binariesDir, `${name}-${triple}${ext}`);

  if (!existsSync(src)) {
    throw new Error(`Expected build artifact not found: ${src}`);
  }

  console.log(`[stage-sidecars] Staging ${src} -> ${dest}`);
  copyFileSync(src, dest);
  if (!isWindows) {
    chmodSync(dest, 0o755);
  }
}

console.log('[stage-sidecars] All sidecars successfully staged.');
