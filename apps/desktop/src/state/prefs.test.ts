import { beforeEach, describe, expect, it } from "vitest";

import { readPref, writePref } from "./prefs";

const isString = (value: unknown): value is string => typeof value === "string";

beforeEach(() => window.localStorage.clear());

describe("preferences", () => {
  it("round-trips a value", () => {
    writePref("vault", "/Users/n/vault");
    expect(readPref("vault", "", isString)).toBe("/Users/n/vault");
  });

  it("falls back when nothing is stored", () => {
    expect(readPref("vault", "none", isString)).toBe("none");
  });

  it("falls back when the stored value is the wrong shape", () => {
    window.localStorage.setItem("heimdall.vault", "42");
    expect(readPref("vault", "none", isString)).toBe("none");
  });

  it("still reads a bare string written by an earlier build", () => {
    // The previous version stored the vault path unquoted; forgetting which
    // vault someone was in is a poor way to greet them after an upgrade.
    window.localStorage.setItem("heimdall.vault", "/Users/n/vault");
    expect(readPref("vault", "", isString)).toBe("/Users/n/vault");
  });
});
