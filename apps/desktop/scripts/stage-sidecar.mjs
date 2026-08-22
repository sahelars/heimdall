#!/usr/bin/env node
// Build the CLI and stage it where Tauri expects an `externalBin` (SPEC §16).
//
// Tauri resolves `binaries/heimdall` to `binaries/heimdall-<target-triple>`, so
// the artifact has to carry that suffix. The desktop always ships its own
// version-matched copy; this script is what makes that true, and it refuses to
// stage a CLI whose version does not match the app's.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, chmodSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const desktopDir = resolve(here, "..");
const repoRoot = resolve(desktopDir, "..", "..");
const release = process.argv.includes("--release");
const profile = release ? "release" : "debug";

/** The host triple, as rustc reports it and as Tauri expects it. */
function targetTriple() {
  const explicit = process.env.TAURI_ENV_TARGET_TRIPLE;
  if (explicit) return explicit;
  const verbose = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const host = verbose.split("\n").find((line) => line.startsWith("host:"));
  if (!host) throw new Error("could not determine the host target triple from rustc -vV");
  return host.slice("host:".length).trim();
}

function cargoVersion(manifestPath) {
  const manifest = readFileSync(manifestPath, "utf8");
  const workspaceVersion = manifest.match(/^\s*version\s*=\s*"([^"]+)"/m);
  return workspaceVersion?.[1];
}

function main() {
  const triple = targetTriple();

  console.log(`staging the ${profile} heimdall sidecar for ${triple}`);
  execFileSync(
    "cargo",
    ["build", "-p", "heimdall-cli", ...(release ? ["--release"] : [])],
    { cwd: repoRoot, stdio: "inherit" },
  );

  const built = join(repoRoot, "target", profile, "heimdall");
  const binaries = join(desktopDir, "src-tauri", "binaries");
  mkdirSync(binaries, { recursive: true });
  const staged = join(binaries, `heimdall-${triple}`);

  copyFileSync(built, staged);
  chmodSync(staged, 0o755);

  // A desktop release must bundle a CLI of the same version, so a mismatch is
  // a packaging bug rather than something to discover after shipping.
  const appVersion = JSON.parse(
    readFileSync(join(desktopDir, "src-tauri", "tauri.conf.json"), "utf8"),
  ).version;
  const cliVersion = cargoVersion(join(repoRoot, "Cargo.toml"));
  if (cliVersion && appVersion !== cliVersion) {
    throw new Error(
      `version mismatch: the app is ${appVersion} but the CLI is ${cliVersion}. ` +
        `The desktop must bundle a version-matched CLI (SPEC §16).`,
    );
  }

  const reported = execFileSync(staged, ["--version", "--json"], { encoding: "utf8" });
  const { cli_version, mcp_protocol_version } = JSON.parse(reported).data;
  console.log(`staged ${staged}`);
  console.log(`  cli ${cli_version}, MCP protocol ${mcp_protocol_version}`);
}

main();
