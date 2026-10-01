#!/usr/bin/env node
// Build the signed, notarized macOS release: Heimdall.app inside a DMG (SPEC §16).
//
// Tauri's bundler signs the app and its `externalBin` sidecar with the hardened
// runtime, notarizes the app, and staples the ticket, all driven by the
// `APPLE_*` variables below. It does not notarize the disk image, so this
// script signs, notarizes, and staples the DMG itself, then checks every
// signature the way Gatekeeper will. Nothing secret lives in the repository:
// the identity is in the login keychain and the API key is a file on disk.

import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const desktopDir = resolve(here, "..");
const repoRoot = resolve(desktopDir, "..", "..");
const bundleDir = join(desktopDir, "src-tauri", "target", "release", "bundle");
const TRIPLE = "aarch64-apple-darwin";

const REQUIRED = {
  APPLE_SIGNING_IDENTITY: 'the keychain identity, e.g. "Developer ID Application: Name (TEAMID)"',
  APPLE_API_KEY: "the App Store Connect API key ID",
  APPLE_API_ISSUER: "the App Store Connect issuer ID",
  APPLE_API_KEY_PATH: "the path to AuthKey_<KEYID>.p8",
};

function fail(message) {
  console.error(`\nrelease failed: ${message}`);
  process.exit(1);
}

/**
 * Run a tool without a shell and return stdout and stderr together — codesign
 * and spctl give their verdict on stderr even when they succeed. Any non-zero
 * exit ends the release.
 */
function run(file, args, options = {}) {
  const result = spawnSync(file, args, { encoding: "utf8", ...options });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  if (result.error) fail(`${file} could not be run: ${result.error.message}`);
  if (result.status !== 0) fail(`${file} ${args.join(" ")}\n${output.trim()}`);
  return output;
}

function preflight() {
  const missing = Object.entries(REQUIRED).filter(([name]) => !process.env[name]);
  if (missing.length > 0) {
    fail(
      "set these environment variables first:\n" +
        missing.map(([name, what]) => `  ${name} — ${what}`).join("\n"),
    );
  }
  if (!existsSync(process.env.APPLE_API_KEY_PATH)) {
    fail(`APPLE_API_KEY_PATH does not exist: ${process.env.APPLE_API_KEY_PATH}`);
  }
  const identities = run("security", ["find-identity", "-v", "-p", "codesigning"]);
  if (!identities.includes(`"${process.env.APPLE_SIGNING_IDENTITY}"`)) {
    fail(
      `the identity "${process.env.APPLE_SIGNING_IDENTITY}" is not in the keychain. ` +
        `security find-identity -v -p codesigning lists:\n${identities}`,
    );
  }
  const host = run("rustc", ["-vV"]).split("\n").find((line) => line.startsWith("host:"));
  if (host?.slice("host:".length).trim() !== TRIPLE) {
    fail(`this release is built for ${TRIPLE}; rustc reports ${host}`);
  }
  if (run("git", ["status", "--porcelain"], { cwd: repoRoot }).trim() !== "") {
    console.warn("warning: the working tree has uncommitted changes; they will ship in this build");
  }
}

function build() {
  console.log("building, signing, and notarizing Heimdall.app (notarization takes a few minutes)");
  try {
    execFileSync("npm", ["run", "tauri:build"], { cwd: desktopDir, stdio: "inherit" });
  } catch {
    fail(
      "npm run tauri:build did not succeed (see above). If it failed creating Assets.car, " +
        "run the release again, after `killall ibtoold` if it fails twice: actool's daemon " +
        "intermittently crashes on Icon Composer icons.",
    );
  }
}

function findDmg(version) {
  const dir = join(bundleDir, "dmg");
  const name = `Heimdall_${version}_aarch64.dmg`;
  if (!existsSync(join(dir, name))) {
    const found = existsSync(dir) ? readdirSync(dir).join(", ") : "nothing";
    fail(`expected ${name} in ${dir}; found ${found}`);
  }
  return join(dir, name);
}

function notarizeDmg(dmg) {
  console.log("signing the disk image");
  run("codesign", ["--force", "--sign", process.env.APPLE_SIGNING_IDENTITY, "--timestamp", dmg]);

  console.log("notarizing the disk image (this waits for Apple)");
  const submitted = run("xcrun", [
    "notarytool", "submit", dmg,
    "--key", process.env.APPLE_API_KEY_PATH,
    "--key-id", process.env.APPLE_API_KEY,
    "--issuer", process.env.APPLE_API_ISSUER,
    "--wait",
  ]);
  console.log(submitted.trim());
  // notarytool exits 0 on a completed submission even when Apple rejects it.
  if (!/status:\s*Accepted/.test(submitted)) {
    fail(
      "Apple did not accept the disk image. Fetch the log with:\n" +
        "  xcrun notarytool log <submission id> --key … --key-id … --issuer …",
    );
  }
  run("xcrun", ["stapler", "staple", dmg]);
}

/** Every check Gatekeeper makes on a downloaded app, so a bad build never reaches a user. */
function verify(app, dmg, version) {
  console.log("verifying signatures");
  run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", app]);

  const macos = join(app, "Contents", "MacOS");
  for (const name of readdirSync(macos)) {
    const details = run("codesign", ["-dv", "--verbose=4", join(macos, name)]);
    if (!details.includes("Authority=Developer ID Application")) fail(`${name} is not signed with a Developer ID`);
    if (!/flags=0x[0-9a-f]+\([^)]*runtime/.test(details)) fail(`${name} lacks the hardened runtime`);
    if (!details.includes("Timestamp=")) fail(`${name} has no secure timestamp`);
  }

  const assessment = run("spctl", ["-a", "-vvv", "-t", "exec", app]);
  if (!assessment.includes("source=Notarized Developer ID")) {
    fail(`Gatekeeper does not see the app as notarized:\n${assessment}`);
  }
  run("spctl", ["-a", "-vvv", "-t", "open", "--context", "context:primary-signature", dmg]);
  run("xcrun", ["stapler", "validate", app]);
  run("xcrun", ["stapler", "validate", dmg]);

  const sidecar = join(macos, "heimdall");
  const reported = JSON.parse(execFileSync(sidecar, ["--version", "--json"], { encoding: "utf8" })).data;
  if (reported.cli_version !== version) {
    fail(`the bundled CLI reports ${reported.cli_version}, but the app is ${version}`);
  }
}

function main() {
  const version = JSON.parse(
    readFileSync(join(desktopDir, "src-tauri", "tauri.conf.json"), "utf8"),
  ).version;

  preflight();
  build();

  const app = join(bundleDir, "macos", "Heimdall.app");
  if (!existsSync(app)) fail(`expected ${app}`);
  const dmg = findDmg(version);

  notarizeDmg(dmg);
  verify(app, dmg, version);

  const sha256 = createHash("sha256").update(readFileSync(dmg)).digest("hex");
  console.log(`\nreleased Heimdall ${version}`);
  console.log(`  ${dmg}`);
  console.log(`  sha256 ${sha256}`);
}

main();
