/**
 * The command and configuration text the Server screen shows.
 *
 * These are pure functions on purpose: what a user copies into a client config
 * has to be exactly right, and that should be checkable without a window.
 */

import { describe, expect, it } from "vitest";

import {
  chatgptConfigSnippet,
  clientConfigSnippet,
  mcpCommand,
  protectedLocation,
  vaultName,
} from "./cli";

const BUNDLED = "/Applications/Heimdall.app/Contents/MacOS/heimdall";

describe("mcpCommand", () => {
  it("shows the exact command, pointing at the bundled tool, with no vault in it", () => {
    expect(mcpCommand(BUNDLED)).toBe(`${BUNDLED} mcp`);
  });

  it("quotes a tool path a shell would otherwise split", () => {
    expect(mcpCommand("/Applications/My Apps/heimdall")).toBe('"/Applications/My Apps/heimdall" mcp');
  });

  it("escapes a path that would otherwise break out of its quotes", () => {
    const command = mcpCommand('/x/"; rm -rf ~; echo "/heimdall');
    // Every embedded quote is escaped, so the value cannot end the argument.
    expect(command).not.toMatch(/[^\\]"; rm/);
    expect(command).toContain('\\"');
  });
});

describe("clientConfigSnippet", () => {
  it("produces one mcpServers entry for every shared vault", () => {
    const parsed = JSON.parse(clientConfigSnippet(BUNDLED));
    expect(parsed.mcpServers).toEqual({ heimdall: { command: BUNDLED, args: ["mcp"] } });
  });

  it("never names a vault: clients pick one by name on each call", () => {
    const snippet = clientConfigSnippet(BUNDLED);
    expect(snippet).not.toContain("--vault");
    expect(Object.keys(JSON.parse(snippet).mcpServers.heimdall)).toEqual(["command", "args"]);
  });

  it("stays valid JSON for a tool path containing quotes and backslashes", () => {
    const awkward = '/Users/n/He said "hi"\\heimdall';
    expect(JSON.parse(clientConfigSnippet(awkward)).mcpServers.heimdall.command).toBe(awkward);
  });
});

describe("chatgptConfigSnippet", () => {
  it("produces the mcp_servers table ChatGPT's config.toml reads", () => {
    // Byte for byte what "Add entry" writes into a file that has none yet.
    expect(chatgptConfigSnippet(BUNDLED)).toBe(
      `[mcp_servers.heimdall]\ncommand = "${BUNDLED}"\nargs = ["mcp"]\n`,
    );
  });

  it("escapes a tool path containing spaces, quotes and backslashes", () => {
    const awkward = '/Users/n/He said "hi"\\heimdall';
    expect(chatgptConfigSnippet(awkward)).toContain('"/Users/n/He said \\"hi\\"\\\\heimdall"');
  });
});

describe("protectedLocation", () => {
  it("names the folders macOS keeps other apps out of", () => {
    expect(protectedLocation("/Users/n/Documents/Heimdall-Vault")).toBe("Documents");
    expect(protectedLocation("/Users/n/Desktop/notes")).toBe("Desktop");
    expect(protectedLocation("/Users/n/Downloads")).toBe("Downloads");
    expect(protectedLocation("/Users/n/Library/Mobile Documents/com~apple~CloudDocs/v")).toBe("iCloud Drive");
    expect(protectedLocation("/Users/n/Library/CloudStorage/Dropbox/v")).toBe("cloud storage");
    expect(protectedLocation("/Volumes/USB/v")).toBe("an external or network volume");
  });

  it("leaves everywhere else alone", () => {
    expect(protectedLocation("/Users/n/Notes")).toBeNull();
    expect(protectedLocation("/Users/n/DocumentsArchive/v")).toBeNull();
    expect(protectedLocation("/Users/n/Heimdall/Work")).toBeNull();
  });
});

describe("vaultName", () => {
  it("labels a vault by its folder", () => {
    expect(vaultName("/Users/n/Documents/My Vault")).toBe("My Vault");
    expect(vaultName("/Users/n/Documents/My Vault/")).toBe("My Vault");
    expect(vaultName("C:\\Users\\n\\Vault")).toBe("Vault");
  });
});
