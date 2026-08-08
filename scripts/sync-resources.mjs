#!/usr/bin/env node
/**
 * sync-resources.mjs
 *
 * Tauri's `bundle.resources` is static — it cannot express per-platform
 * includes. llama-server ships separate binaries for linux / macos-aarch64 /
 * macos-x86_64 / windows-x86_64, and bundling ALL of them inflates every
 * installer with three irrelevant copies.
 *
 * This script runs as the first step of `beforeBuildCommand` (`npm run build`)
 * and rewrites `src-tauri/tauri.conf.json` so that `bundle.resources` only
 * references the llama-server directory for the platform actually being built.
 *
 * It is idempotent: re-running it for the same target is a no-op.
 *
 * Resolution order for the target platform:
 *   1. TAURI_ENV_PLATFORM_TARGET / TAURI_PLATFORM_TARGET / CARGO_BUILD_TARGET
 *      (a full Rust target triple — most reliable for cross-compiles)
 *   2. TAURI_ENV_PLATFORM + TAURI_ENV_ARCH / TAURI_PLATFORM + TAURI_ARCH
 *   3. Node's process.platform + process.arch (host = target for local builds)
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '..');
const confPath = resolve(projectRoot, 'src-tauri', 'tauri.conf.json');

// Map a Rust target triple to our binaries/llama-server/<dir> folder name.
const TRIPLE_TO_DIR = {
  'x86_64-apple-darwin': 'macos-x86_64',
  'aarch64-apple-darwin': 'macos-aarch64',
  'x86_64-pc-windows-msvc': 'windows-x86_64',
  'x86_64-pc-windows-gnu': 'windows-x86_64',
  'x86_64-unknown-linux-gnu': 'linux-x86_64',
  'aarch64-unknown-linux-gnu': 'linux-aarch64',
};

function getPlatformDir() {
  // 1) Full target triple — handles macOS x86_64 cross-compile on Apple Silicon.
  const triple =
    process.env.TAURI_ENV_PLATFORM_TARGET ||
    process.env.TAURI_PLATFORM_TARGET ||
    process.env.CARGO_BUILD_TARGET ||
    '';
  if (TRIPLE_TO_DIR[triple]) return TRIPLE_TO_DIR[triple];

  // 2) Tauri platform + arch env vars.
  const tp = process.env.TAURI_ENV_PLATFORM || process.env.TAURI_PLATFORM || '';
  const ta = process.env.TAURI_ENV_ARCH || process.env.TAURI_ARCH || '';
  if (tp === 'macos') return ta === 'aarch64' ? 'macos-aarch64' : 'macos-x86_64';
  if (tp === 'windows') return 'windows-x86_64';
  if (tp === 'linux') return ta === 'aarch64' ? 'linux-aarch64' : 'linux-x86_64';

  // 3) Host platform (local non-cross builds).
  const plat = process.platform;
  const arch = process.arch;
  if (plat === 'darwin') return arch === 'arm64' ? 'macos-aarch64' : 'macos-x86_64';
  if (plat === 'win32') return 'windows-x86_64';
  if (plat === 'linux') return arch === 'arm64' ? 'linux-aarch64' : 'linux-x86_64';

  throw new Error(
    `[sync-resources] Cannot determine target platform ` +
      `(platform=${plat}, arch=${arch}, triple=${triple || '<none>'})`
  );
}

const platformDir = getPlatformDir();
console.log(`[sync-resources] Target llama-server platform dir: ${platformDir}`);

// ── Rewrite bundle.resources in tauri.conf.json ──────────────────────────
const confRaw = readFileSync(confPath, 'utf-8');
const conf = JSON.parse(confRaw);

const resources = conf.bundle?.resources ?? {};
// Drop any prior llama-server and acp entries, keep the rest (nodejs/, etc.).
// This makes the script idempotent across re-runs.
for (const key of [...Object.keys(resources)]) {
  if (key.startsWith('./binaries/') || key.startsWith('./acp/')) {
    delete resources[key];
  }
}
// Only bundle the llama-server directory for the platform being built.
const platformResourceKey = `./binaries/llama-server/${platformDir}/`;
resources[platformResourceKey] = platformResourceKey;
// Bundle the pre-installed Claude ACP package (single-platform, slimmed by
// preinstall-claude-acp.mjs which runs right after this script).
resources['./acp/'] = './acp/';

conf.bundle.resources = resources;

// Preserve 2-space indentation + trailing newline (matches the repo style).
writeFileSync(confPath, JSON.stringify(conf, null, 2) + '\n', 'utf-8');
console.log(`[sync-resources] Updated bundle.resources in tauri.conf.json`);
console.log(
  `[sync-resources]   ${JSON.stringify({ [platformResourceKey]: platformResourceKey, './acp/': './acp/' })}`
);
