#!/usr/bin/env node

import { readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(join(__dirname, ".."));

const publishOrder = [
  "typescript/packages/roam-postcard/package.json",
  "typescript/packages/roam-wire/package.json",
  "typescript/packages/roam-core/package.json",
  "typescript/packages/roam-tcp/package.json",
  "typescript/packages/roam-ws/package.json",
];

const manifestPaths = publishOrder.map((relativePath) => join(repoRoot, relativePath));
const originalManifests = new Map();

function parseArgs(argv) {
  let tag = "latest";
  let dryRun = false;

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--dry-run") {
      dryRun = true;
      continue;
    }
    if (arg === "--tag") {
      tag = argv[i + 1];
      i += 1;
      continue;
    }
    throw new Error(`Unknown argument: ${arg}`);
  }

  return { tag, dryRun };
}

function readWorkspaceVersion() {
  const cargoToml = readFileSync(join(repoRoot, "Cargo.toml"), "utf8");
  const match = cargoToml.match(/\[workspace\.package\][\s\S]*?\nversion = "([^"]+)"/);
  if (!match) {
    throw new Error("Could not determine workspace version from Cargo.toml");
  }
  return match[1];
}

function updateManifest(path, version, packageNames) {
  const source = readFileSync(path, "utf8");
  originalManifests.set(path, source);
  const manifest = JSON.parse(source);

  manifest.version = version;

  for (const section of [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
  ]) {
    const deps = manifest[section];
    if (!deps) {
      continue;
    }

    for (const [name, current] of Object.entries(deps)) {
      if (packageNames.has(name) && typeof current === "string" && current.startsWith("workspace:")) {
        deps[name] = version;
      }
    }
  }

  writeFileSync(path, `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest.name;
}

function restoreManifests() {
  for (const [path, source] of originalManifests) {
    writeFileSync(path, source);
  }
}

function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    stdio: "inherit",
    env: process.env,
  });
  if (result.status !== 0) {
    const commandLine = [command, ...args].join(" ");
    throw new Error(`${commandLine} failed with exit code ${result.status ?? 1}`);
  }
}

function dryRunPublish(packageDir, tag) {
  const manifest = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8"));
  const tarballName = manifest.name.replace(/^@/, "").replace(/\//g, "-");
  run("pnpm", ["pack"], packageDir);
  unlinkSync(join(packageDir, `${tarballName}-${manifest.version}.tgz`));
  const relativeDir = packageDir.slice(repoRoot.length + 1);
  console.log(`dry-run: pnpm publish --access public --no-git-checks --tag ${tag} (${relativeDir})`);
}

const { tag, dryRun } = parseArgs(process.argv.slice(2));
const version = readWorkspaceVersion();

try {
  const packageNames = new Set();
  for (const manifestPath of manifestPaths) {
    const name = updateManifest(manifestPath, version, packageNames);
    packageNames.add(name);
  }

  for (const manifestPath of manifestPaths) {
    const packageDir = dirname(manifestPath);
    if (dryRun) {
      dryRunPublish(packageDir, tag);
      continue;
    }
    run("pnpm", ["publish", "--access", "public", "--no-git-checks", "--tag", tag], packageDir);
  }
} finally {
  restoreManifests();
}
