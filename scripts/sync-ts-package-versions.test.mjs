import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  discoverPublicTypeScriptPackages,
  readWorkspaceVersion,
  syncTypeScriptPackageVersions,
} from "./sync-ts-package-versions.mjs";

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

test("readWorkspaceVersion reads the workspace package version", () => {
  const cargoToml = `[workspace]
members = []

[workspace.package]
version = "9.1.0"
`;

  assert.equal(readWorkspaceVersion(cargoToml), "9.1.0");
});

test("discoverPublicTypeScriptPackages only returns non-private packages", () => {
  const repoRoot = mkdtempSync(join(tmpdir(), "telex-ts-packages-"));
  const packagesDir = join(repoRoot, "typescript", "packages");
  mkdirSync(join(packagesDir, "public"), { recursive: true });
  mkdirSync(join(packagesDir, "private"), { recursive: true });

  writeJson(join(packagesDir, "public", "package.json"), {
    name: "@bearcove/public",
    version: "0.1.0",
  });
  writeJson(join(packagesDir, "private", "package.json"), {
    name: "@bearcove/private",
    private: true,
    version: "0.1.0",
  });

  const packages = discoverPublicTypeScriptPackages(repoRoot);
  assert.deepEqual(
    packages.map(({ manifest }) => manifest.name),
    ["@bearcove/public"],
  );
});

test("syncTypeScriptPackageVersions updates public packages to the Cargo workspace version", () => {
  const repoRoot = mkdtempSync(join(tmpdir(), "telex-ts-sync-"));
  const packagesDir = join(repoRoot, "typescript", "packages");
  mkdirSync(join(packagesDir, "telex-core"), { recursive: true });
  mkdirSync(join(packagesDir, "telex-private"), { recursive: true });

  writeFileSync(
    join(repoRoot, "Cargo.toml"),
    `[workspace]
members = []

[workspace.package]
version = "7.2.0"
`,
  );

  writeJson(join(packagesDir, "telex-core", "package.json"), {
    name: "@bearcove/telex-core",
    version: "7.1.0",
  });
  writeJson(join(packagesDir, "telex-private", "package.json"), {
    name: "@bearcove/telex-private",
    private: true,
    version: "1.0.0",
  });

  const result = syncTypeScriptPackageVersions(repoRoot);

  assert.equal(result.version, "7.2.0");
  assert.deepEqual(result.updatedPackages, [
    {
      name: "@bearcove/telex-core",
      previousVersion: "7.1.0",
      version: "7.2.0",
    },
  ]);

  assert.equal(
    JSON.parse(readFileSync(join(packagesDir, "telex-core", "package.json"), "utf8")).version,
    "7.2.0",
  );
  assert.equal(
    JSON.parse(readFileSync(join(packagesDir, "telex-private", "package.json"), "utf8")).version,
    "1.0.0",
  );
});
