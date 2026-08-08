#!/usr/bin/env node
/**
 * preinstall-claude-acp.mjs
 *
 * Pre-installs @agentclientprotocol/claude-agent-acp into
 * src-tauri/acp/claude-agent-acp/ so it ships inside the app bundle and
 * users don't have to manually install the Claude agent.
 *
 * The bulk of the package is @anthropic-ai/claude-agent-sdk-{os}-{arch}
 * (~235 MB per platform), distributed via optionalDependencies. We install
 * only the target platform's binary and delete the rest — same idea as the
 * llama-server per-platform slimming.
 *
 * Idempotent: skips when already installed for the same platform.
 * Runs as part of beforeBuildCommand (npm run build) before tauri bundles.
 */
import { execSync } from 'node:child_process';
import {
  existsSync, mkdirSync, readFileSync, readdirSync,
  rmSync, writeFileSync,
} from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '..');
const acpDir = resolve(projectRoot, 'src-tauri', 'acp', 'claude-agent-acp');
const entryPoint = resolve(
  acpDir,
  'node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js'
);
const platformMarker = resolve(acpDir, '.preinstalled-platform');
const pkgJsonPath = resolve(acpDir, 'package.json');

// ── Resolve target platform in npm os/arch naming ────────────────────────
function getNpmTarget() {
  const triple =
    process.env.TAURI_ENV_PLATFORM_TARGET ||
    process.env.TAURI_PLATFORM_TARGET ||
    process.env.CARGO_BUILD_TARGET ||
    '';
  const tripleMap = {
    'x86_64-apple-darwin': { os: 'darwin', arch: 'x64' },
    'aarch64-apple-darwin': { os: 'darwin', arch: 'arm64' },
    'x86_64-pc-windows-msvc': { os: 'win32', arch: 'x64' },
    'x86_64-pc-windows-gnu': { os: 'win32', arch: 'x64' },
    'x86_64-unknown-linux-gnu': { os: 'linux', arch: 'x64' },
    'aarch64-unknown-linux-gnu': { os: 'linux', arch: 'arm64' },
  };
  if (tripleMap[triple]) return tripleMap[triple];

  const tp = process.env.TAURI_ENV_PLATFORM || process.env.TAURI_PLATFORM || '';
  const ta = process.env.TAURI_ENV_ARCH || process.env.TAURI_ARCH || '';
  const osMap = { macos: 'darwin', windows: 'win32', linux: 'linux' };
  const archMap = { x86_64: 'x64', aarch64: 'arm64', x64: 'x64', arm64: 'arm64' };
  if (osMap[tp]) return { os: osMap[tp], arch: archMap[ta] || 'x64' };

  // Last resort: host platform.
  return { os: process.platform, arch: process.arch };
}

const { os, arch } = getNpmTarget();
const platformSuffix = `${os}-${arch}`; // e.g. "darwin-x64"
const targetSdkPkg = `@anthropic-ai/claude-agent-sdk-${platformSuffix}`;
const targetSdkDir = resolve(acpDir, `node_modules/@anthropic-ai/claude-agent-sdk-${platformSuffix}`);
console.log(`[preinstall-claude-acp] Target platform: ${platformSuffix}`);

// ── Skip if already installed for this platform ──────────────────────────
if (existsSync(entryPoint) && existsSync(platformMarker)) {
  const installed = readFileSync(platformMarker, 'utf-8').trim();
  if (installed === platformSuffix && existsSync(targetSdkDir)) {
    console.log(`[preinstall-claude-acp] Already installed for ${platformSuffix}, skipping.`);
    process.exit(0);
  }
  console.log(
    `[preinstall-claude-acp] Platform changed or SDK missing (${installed} → ${platformSuffix}), reinstalling...`
  );
}

// ── Ensure package.json exists ───────────────────────────────────────────
mkdirSync(acpDir, { recursive: true });
if (!existsSync(pkgJsonPath)) {
  writeFileSync(
    pkgJsonPath,
    JSON.stringify(
      { name: 'runjam-claude-acp', version: '1.0.0', private: true },
      null,
      2
    ) + '\n',
    'utf-8'
  );
}

const npmBin = process.platform === 'win32' ? 'npm.cmd' : 'npm';
// Hint npm to fetch optionalDependencies for the target platform.
const npmEnv = {
  ...process.env,
  npm_config_target_os: os,
  npm_config_target_arch: arch,
};

// ── Install claude-agent-acp (pulls the right platform SDK via optionalDeps) ─
console.log(`[preinstall-claude-acp] Installing @agentclientprotocol/claude-agent-acp...`);
execSync(`${npmBin} install @agentclientprotocol/claude-agent-acp --save`, {
  cwd: acpDir,
  env: npmEnv,
  stdio: 'inherit',
});

// ── Ensure the target platform SDK binary exists (cross-compile safety net) ─
if (!existsSync(targetSdkDir)) {
  const sdkPkgJsonPath = resolve(
    acpDir,
    'node_modules/@anthropic-ai/claude-agent-sdk/package.json'
  );
  if (existsSync(sdkPkgJsonPath)) {
    const sdkPkg = JSON.parse(readFileSync(sdkPkgJsonPath, 'utf-8'));
    const version = sdkPkg.optionalDependencies?.[targetSdkPkg];
    if (version) {
      console.log(`[preinstall-claude-acp] Cross-compile: installing ${targetSdkPkg}@${version}...`);
      execSync(`${npmBin} install ${targetSdkPkg}@${version} --no-save`, {
        cwd: acpDir,
        env: npmEnv,
        stdio: 'inherit',
      });
    }
  }
}

// ── Remove non-target platform SDK binaries (~235 MB each) ───────────────
const anthropicDir = resolve(acpDir, 'node_modules/@anthropic-ai');
let removed = 0;
if (existsSync(anthropicDir)) {
  for (const entry of readdirSync(anthropicDir)) {
    // Keep "claude-agent-sdk" (main) + "sdk"; drop "claude-agent-sdk-{other-platform}".
    if (entry.startsWith('claude-agent-sdk-') && entry !== `claude-agent-sdk-${platformSuffix}`) {
      rmSync(resolve(anthropicDir, entry), { recursive: true, force: true });
      console.log(`[preinstall-claude-acp] Removed non-target platform package: ${entry}`);
      removed++;
    }
  }
}
console.log(`[preinstall-claude-acp] Removed ${removed} non-target platform package(s).`);

// ── Write platform marker so future builds can skip ──────────────────────
writeFileSync(platformMarker, platformSuffix, 'utf-8');
console.log(`[preinstall-claude-acp] Done. Preinstalled for ${platformSuffix}.`);
