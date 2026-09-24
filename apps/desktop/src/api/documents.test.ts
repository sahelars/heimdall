/**
 * Assembling whole documents out of bounded reads (SPEC §8, §17).
 *
 * The CLI is stubbed at the IPC boundary, so these exercise the real loop
 * against the real shape of a response — including the shapes that mean
 * "something moved underneath you", which must never assemble into a file that
 * never existed.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

const {
  readWholeDocument,
  readLockState,
  saveDocument,
  listAllDocuments,
  lockPath,
  unlockPath,
  CliFailure,
} = await import("./documents");

/** One chunk of a `read` response for a note. */
function chunk(options: {
  content: string;
  complete: boolean;
  nextLine?: number | null;
  revision?: string;
  sizeBytes: number;
  locked?: boolean;
  lockedAt?: string;
}) {
  return {
    ok: true,
    data: {
      path: "ideas/note.md",
      kind: "document",
      locked: options.locked ?? false,
      ...(options.locked ? { locked_at: options.lockedAt ?? "ideas/note.md" } : {}),
      document: {
        path: "ideas/note.md",
        content: options.content,
        start_line: 1,
        end_line: 1,
        next_line: options.complete ? null : (options.nextLine ?? 2),
        complete: options.complete,
        size_bytes: options.sizeBytes,
        revision: options.revision ?? "blake3:aa",
      },
    },
  };
}

/** One page of a `read` response for a folder. */
function page(
  entries: { path: string; kind: "document" | "directory"; locked?: boolean }[],
  nextCursor: string | null,
  options: { scanGuardHit?: boolean; locked?: boolean } = {},
) {
  return {
    ok: true,
    data: {
      path: "",
      kind: "directory",
      locked: options.locked ?? false,
      listing: {
        entries: entries.map((entry) => ({ modified_at: "", locked: false, ...entry })),
        next_cursor: nextCursor,
        scan_guard_hit: options.scanGuardHit ?? false,
      },
    },
  };
}

const bytes = (text: string) => new TextEncoder().encode(text).length;

beforeEach(() => {
  invoke.mockReset();
});

describe("reading a whole document", () => {
  it("returns a single-chunk file as it stands", async () => {
    const content = "# Note\n\nBody.\n";
    invoke.mockResolvedValue(chunk({ content, complete: true, sizeBytes: bytes(content) }));

    const document = await readWholeDocument("/v", "ideas/note.md");

    expect(document.content).toBe(content);
    expect(document.revision).toBe("blake3:aa");
    expect(document.chunks).toBe(1);
    expect(document.locked).toBe(false);
    expect(document.lockedAt).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("invoke_cli", {
      command: "read",
      request: {
        vault: "/v",
        path: "ideas/note.md",
        "start-line": 1,
        "max-lines": 1000,
        "max-total-bytes": 262_144,
      },
      stdin: undefined,
    });
  });

  it("reports a locked note, and the rule that locks it", async () => {
    const content = "# Note\n";
    invoke.mockResolvedValue(
      chunk({ content, complete: true, sizeBytes: bytes(content), locked: true, lockedAt: "ideas" }),
    );

    const document = await readWholeDocument("/v", "ideas/note.md");

    expect(document.content).toBe(content);
    expect(document.locked).toBe(true);
    expect(document.lockedAt).toBe("ideas");
  });

  it("follows the continuation and reassembles the chunks in order", async () => {
    const parts = ["one\n", "two\n", "three\n"];
    const total = bytes(parts.join(""));
    invoke
      .mockResolvedValueOnce(chunk({ content: parts[0]!, complete: false, nextLine: 2, sizeBytes: total }))
      .mockResolvedValueOnce(chunk({ content: parts[1]!, complete: false, nextLine: 3, sizeBytes: total }))
      .mockResolvedValueOnce(chunk({ content: parts[2]!, complete: true, sizeBytes: total }));

    const document = await readWholeDocument("/v", "ideas/note.md");

    expect(document.content).toBe("one\ntwo\nthree\n");
    expect(document.chunks).toBe(3);
    // Each call resumes exactly where the last one stopped.
    expect(invoke.mock.calls[1]![1]).toMatchObject({ request: { "start-line": 2 } });
    expect(invoke.mock.calls[2]![1]).toMatchObject({ request: { "start-line": 3 } });
  });

  it("starts again when the file changes between chunks", async () => {
    // Each chunk's revision covers the whole file, so a change is always visible
    // here — and stitching the two halves together would produce a document that
    // never existed on disk.
    const content = "one\ntwo\n";
    const total = bytes(content);
    invoke
      .mockResolvedValueOnce(
        chunk({ content: "one\n", complete: false, nextLine: 2, revision: "blake3:aa", sizeBytes: total }),
      )
      .mockResolvedValueOnce(
        chunk({ content: "CHANGED\n", complete: true, revision: "blake3:bb", sizeBytes: total }),
      )
      .mockResolvedValue(chunk({ content, complete: true, revision: "blake3:bb", sizeBytes: total }));

    const document = await readWholeDocument("/v", "ideas/note.md");

    expect(document.content).toBe(content);
    expect(document.revision).toBe("blake3:bb");
  });

  it("gives up rather than looping when the file never settles", async () => {
    let revision = 0;
    invoke.mockImplementation(() => {
      revision += 1;
      return Promise.resolve(
        chunk({
          content: "x\n",
          complete: false,
          nextLine: 2,
          revision: `blake3:${revision}`,
          sizeBytes: 4,
        }),
      );
    });

    await expect(readWholeDocument("/v", "ideas/note.md")).rejects.toThrow(/kept changing/);
  });

  it("counts bytes rather than characters when checking the assembly", async () => {
    // The test that catches a UTF-16 length comparison: this content is 8 code
    // units but 14 bytes.
    const content = "a 🌍 café\n";
    invoke.mockResolvedValue(chunk({ content, complete: true, sizeBytes: bytes(content) }));

    const document = await readWholeDocument("/v", "ideas/note.md");
    expect(document.content).toBe(content);
    expect(document.sizeBytes).toBe(bytes(content));
    expect(content.length).not.toBe(bytes(content));
  });

  it("refuses an assembly that does not match the size the vault reports", async () => {
    invoke.mockResolvedValue(chunk({ content: "short\n", complete: true, sizeBytes: 9_999 }));

    await expect(readWholeDocument("/v", "ideas/note.md")).rejects.toThrow(/assembled to/);
  });

  it("refuses a folder rather than returning an empty file", async () => {
    invoke.mockResolvedValue(page([], null));

    await expect(readWholeDocument("/v", "ideas")).rejects.toThrow(/is a folder/);
  });

  it("passes a domain failure through with its code intact", async () => {
    invoke.mockResolvedValue({
      ok: false,
      error: { code: "NOT_FOUND", message: "no such note" },
    });

    await expect(readWholeDocument("/v", "ideas/gone.md")).rejects.toMatchObject({
      error: { code: "NOT_FOUND" },
    });
  });
});

describe("saving", () => {
  it("sends the revision it read and the content on stdin", async () => {
    invoke.mockResolvedValue({
      ok: true,
      data: { path: "ideas/note.md", new_revision: "blake3:bb", size_bytes: 5, created: false },
    });

    const result = await saveDocument("/v", "ideas/note.md", "body\n", "blake3:aa");

    expect(result).toEqual({ revision: "blake3:bb", created: false });
    expect(invoke).toHaveBeenCalledWith("invoke_cli", {
      command: "write",
      request: { vault: "/v", path: "ideas/note.md", "expected-revision": "blake3:aa" },
      stdin: "body\n",
    });
  });

  it("spells a new note as an explicit create rather than a missing revision", async () => {
    invoke.mockResolvedValue({
      ok: true,
      data: { path: "ideas/new.md", new_revision: "blake3:cc", size_bytes: 1, created: true },
    });

    await saveDocument("/v", "ideas/new.md", "x", null);

    // Omitting the revision is what the CLI treats as a caller mistake, which is
    // exactly the protection this relies on.
    expect(invoke.mock.calls[0]![1]).toMatchObject({
      command: "write",
      request: { vault: "/v", path: "ideas/new.md", create: true },
    });
  });

  it("raises a revision conflict instead of reporting success", async () => {
    invoke.mockResolvedValue({
      ok: false,
      error: {
        code: "REVISION_CONFLICT",
        message: "changed since it was read",
        details: { current_revision: "blake3:zz" },
      },
    });

    await expect(saveDocument("/v", "ideas/note.md", "x", "blake3:aa")).rejects.toBeInstanceOf(
      CliFailure,
    );
  });

  it("carries a lock refusal through with the rule that caused it", async () => {
    invoke.mockResolvedValue({
      ok: false,
      error: {
        code: "LOCKED",
        message: '"ideas/note.md" is locked',
        details: { path: "ideas/note.md", locked_at: "ideas" },
      },
    });

    await expect(saveDocument("/v", "ideas/note.md", "x", "blake3:aa")).rejects.toMatchObject({
      error: { code: "LOCKED", details: { locked_at: "ideas" } },
    });
  });
});

describe("locking", () => {
  it("locks and unlocks one path", async () => {
    invoke.mockResolvedValue({
      ok: true,
      data: { path: "ideas", kind: "directory", locked: true, changed: true },
    });

    const locked = await lockPath("/v", "ideas");
    expect(locked).toEqual({ path: "ideas", kind: "directory", locked: true, changed: true });
    expect(invoke).toHaveBeenLastCalledWith("invoke_cli", {
      command: "lock",
      request: { vault: "/v", path: "ideas" },
      stdin: undefined,
    });

    invoke.mockResolvedValue({
      ok: true,
      data: { path: "ideas/note.md", kind: "document", locked: false, changed: true },
    });
    await unlockPath("/v", "ideas/note.md");
    expect(invoke).toHaveBeenLastCalledWith("invoke_cli", {
      command: "unlock",
      request: { vault: "/v", path: "ideas/note.md" },
      stdin: undefined,
    });
  });

  it("locks the whole vault by naming no path at all", async () => {
    invoke.mockResolvedValue({
      ok: true,
      data: { path: "", kind: "directory", locked: true, changed: true },
    });

    await lockPath("/v", null);
    expect(invoke).toHaveBeenCalledWith("invoke_cli", {
      command: "lock",
      request: { vault: "/v" },
      stdin: undefined,
    });
  });

  it("passes a failure through", async () => {
    invoke.mockResolvedValue({ ok: false, error: { code: "NOT_FOUND", message: "no such path" } });

    await expect(unlockPath("/v", "gone.md")).rejects.toMatchObject({
      error: { code: "NOT_FOUND" },
    });
  });

  it("reads one path's lock state with a one-line read", async () => {
    invoke.mockResolvedValue(
      chunk({ content: "x\n", complete: false, sizeBytes: 9, locked: true, lockedAt: "" }),
    );

    expect(await readLockState("/v", "ideas/note.md")).toEqual({ locked: true, lockedAt: "" });
    expect(invoke.mock.calls[0]![1]).toMatchObject({
      command: "read",
      request: { path: "ideas/note.md", "max-lines": 1 },
    });
  });
});

describe("listing the vault", () => {
  it("follows the cursor to the end and returns one flat list", async () => {
    invoke
      .mockResolvedValueOnce(page([{ path: "a.md", kind: "document" }], "a.md"))
      .mockResolvedValueOnce(page([{ path: "b.md", kind: "document", locked: true }], null));

    const listing = await listAllDocuments("/v");

    expect(listing.entries.map((entry) => [entry.path, entry.locked])).toEqual([
      ["a.md", false],
      ["b.md", true],
    ]);
    expect(listing.truncated).toBe(false);
    expect(listing.rootLocked).toBe(false);
    expect(invoke.mock.calls[0]![1]).toEqual({
      command: "read",
      request: { vault: "/v", recursive: true, "max-depth": 16, limit: 200 },
      stdin: undefined,
    });
    expect(invoke.mock.calls[1]![1]).toMatchObject({ command: "read", request: { cursor: "a.md" } });
  });

  it("says when the vault root itself is locked", async () => {
    invoke.mockResolvedValue(page([{ path: "a.md", kind: "document", locked: true }], null, { locked: true }));

    const listing = await listAllDocuments("/v");

    expect(listing.rootLocked).toBe(true);
  });

  it("reports a listing it had to stop rather than implying it is complete", async () => {
    invoke.mockResolvedValue(
      page([{ path: "a.md", kind: "document" }], "a.md", { scanGuardHit: true }),
    );

    const listing = await listAllDocuments("/v");

    expect(listing.truncated).toBe(true);
    expect(listing.scanGuardHit).toBe(true);
  });
});
