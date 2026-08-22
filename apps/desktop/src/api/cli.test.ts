/**
 * The command and configuration text the Server screen shows.
 *
 * These are pure functions on purpose: what a user copies into a client config
 * has to be exactly right, and that should be checkable without a window.
 */

import { describe, expect, it } from "vitest";

import { clientConfigSnippet, mcpCommand, vaultName } from "./cli";

const BUNDLED = "/Applications/Heimdall.app/Contents/MacOS/heimdall";

describe("mcpCommand", () => {
  it("shows the exact command, pointing at the bundled tool", () => {
    expect(mcpCommand(BUNDLED, "/Users/n/Notes")).toBe(
      `${BUNDLED} mcp --vault /Users/n/Notes`,
    );
  });

  it("quotes a vault path a shell would otherwise split", () => {
    expect(mcpCommand(BUNDLED, "/Users/n/My Vault")).toBe(
      `${BUNDLED} mcp --vault "/Users/n/My Vault"`,
    );
  });

  it("escapes a path that would otherwise break out of its quotes", () => {
    const command = mcpCommand(BUNDLED, '/Users/n/"; rm -rf ~; echo "');
    // Every embedded quote is escaped, so the value cannot end the argument.
    expect(command).not.toMatch(/[^\\]"; rm/);
    expect(command).toContain('\\"');
  });
});

describe("clientConfigSnippet", () => {
  it("produces a valid mcpServers entry for the vault", () => {
    const snippet = clientConfigSnippet(BUNDLED, "/Users/n/Notes");
    const parsed = JSON.parse(snippet);

    expect(parsed.mcpServers.heimdall).toEqual({
      command: BUNDLED,
      args: ["mcp", "--vault", "/Users/n/Notes"],
    });
  });

  it("never puts a vault path anywhere a tool call could read it", () => {
    // The vault appears once, as a server argument — the one place §7 allows.
    const parsed = JSON.parse(clientConfigSnippet(BUNDLED, "/v"));
    expect(parsed.mcpServers.heimdall.args).toEqual(["mcp", "--vault", "/v"]);
    expect(Object.keys(parsed.mcpServers.heimdall)).toEqual(["command", "args"]);
  });

  it("uses the entry name a second vault would need", () => {
    const parsed = JSON.parse(clientConfigSnippet(BUNDLED, "/Users/n/Work", "heimdall-work"));
    expect(Object.keys(parsed.mcpServers)).toEqual(["heimdall-work"]);
  });

  it("stays valid JSON for a path containing quotes and backslashes", () => {
    const awkward = '/Users/n/He said "hi"\\notes';
    const parsed = JSON.parse(clientConfigSnippet(BUNDLED, awkward));
    expect(parsed.mcpServers.heimdall.args[2]).toBe(awkward);
  });
});

describe("vaultName", () => {
  it("labels a vault by its folder", () => {
    expect(vaultName("/Users/n/Documents/My Vault")).toBe("My Vault");
    expect(vaultName("/Users/n/Documents/My Vault/")).toBe("My Vault");
    expect(vaultName("C:\\Users\\n\\Vault")).toBe("Vault");
  });
});
