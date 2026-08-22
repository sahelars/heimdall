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

const { readWholeDocument, saveDocument, listAllDocuments, CliFailure } = await import(
  "./documents"
);

/** One chunk of a `read-documents` response. */
function chunk(options: {
  content: string;
  complete: boolean;
  nextLine?: number | null;
  revision?: string;
  sizeBytes: number;
}) {
  return {
    ok: true,
    data: {
      documents: [
        {
          path: "ideas/note.md",
          returned: true,
          content: options.content,
          start_line: 1,
          end_line: 1,
          next_line: options.complete ? null : (options.nextLine ?? 2),
          complete: options.complete,
          size_bytes: options.sizeBytes,
          revision: options.revision ?? "blake3:aa",
        },
      ],
      total_bytes: options.content.length,
      truncated: false,
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
    expect(invoke).toHaveBeenCalledTimes(1);
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

  it("surfaces a skipped document rather than returning an empty file", async () => {
    invoke.mockResolvedValue({
      ok: true,
      data: {
        documents: [{ path: "ideas/note.md", returned: false, reason: "byte_budget_exhausted" }],
        total_bytes: 0,
        truncated: true,
      },
    });

    await expect(readWholeDocument("/v", "ideas/note.md")).rejects.toThrow(/was not returned/);
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
      command: "write-document",
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
    expect(invoke.mock.calls[0]![1]).toMatchObject({ request: { create: true } });
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
});

describe("listing the vault", () => {
  it("follows the cursor to the end and returns one flat list", async () => {
    invoke
      .mockResolvedValueOnce({
        ok: true,
        data: {
          entries: [{ path: "a.md", kind: "document", modified_at: "" }],
          next_cursor: "a.md",
          scan_guard_hit: false,
        },
      })
      .mockResolvedValueOnce({
        ok: true,
        data: {
          entries: [{ path: "b.md", kind: "document", modified_at: "" }],
          next_cursor: null,
          scan_guard_hit: false,
        },
      });

    const listing = await listAllDocuments("/v");

    expect(listing.entries.map((entry) => entry.path)).toEqual(["a.md", "b.md"]);
    expect(listing.truncated).toBe(false);
    expect(invoke.mock.calls[1]![1]).toMatchObject({ request: { cursor: "a.md" } });
  });

  it("reports a listing it had to stop rather than implying it is complete", async () => {
    invoke.mockResolvedValue({
      ok: true,
      data: {
        entries: [{ path: "a.md", kind: "document", modified_at: "" }],
        next_cursor: "a.md",
        scan_guard_hit: true,
      },
    });

    const listing = await listAllDocuments("/v");

    expect(listing.truncated).toBe(true);
    expect(listing.scanGuardHit).toBe(true);
  });
});

describe("reaching the protected tree", () => {
  /** A complete single-chunk read, as the protected commands return it. */
  function whole(path: string, content: string) {
    return {
      ok: true,
      data: {
        path,
        content,
        start_line: 1,
        end_line: 1,
        next_line: null,
        complete: true,
        size_bytes: bytes(content),
        revision: "blake3:aa",
      },
    };
  }

  const cases: [string, string, string, Record<string, unknown>][] = [
    ["the agent instructions", "aios/AGENTS.md", "read-agents", {}],
    ["the main memory", "aios/memories/memory.md", "read-memory", {}],
    [
      "an extended memory",
      "aios/memories/extended/memory_1.md",
      "read-memory",
      { extended: "memory_1.md" },
    ],
    [
      "a conversation",
      "aios/conversations/2026-08-16_10-30-00.md",
      "read-entry",
      { kind: "conversation", id: "2026-08-16_10-30-00.md" },
    ],
    [
      "a notification",
      "aios/notifications/2026-08-16_10-30-00.md",
      "read-entry",
      { kind: "notification", id: "2026-08-16_10-30-00.md" },
    ],
  ];

  it.each(cases)("reads %s with the command that owns it", async (_name, path, command, extra) => {
    // `read-documents` excludes aios/ at every depth, so routing everything
    // through it would mean the editor could not open half the tree — and
    // making it able to would mean a general read tool, which is the one thing
    // Heimdall does not have.
    invoke.mockResolvedValue(whole(path, "body\n"));

    const document = await readWholeDocument("/v", path);

    expect(document.content).toBe("body\n");
    expect(invoke).toHaveBeenCalledWith("invoke_cli", {
      command,
      request: { vault: "/v", "start-line": 1, "max-lines": 1000, ...extra },
      stdin: undefined,
    });
  });

  it("still reads an ordinary note through read-documents", async () => {
    const content = "ordinary\n";
    invoke.mockResolvedValue(chunk({ content, complete: true, sizeBytes: bytes(content) }));

    await readWholeDocument("/v", "ideas/note.md");
    expect(invoke.mock.calls[0]![1]).toMatchObject({ command: "read-documents" });
  });

  it("saves a memory with write-memory and an entry with write-entry", async () => {
    invoke.mockResolvedValue({
      ok: true,
      data: { path: "x", new_revision: "blake3:bb", size_bytes: 1, created: false },
    });

    await saveDocument("/v", "aios/memories/extended/memory_1.md", "body\n", "blake3:aa");
    expect(invoke.mock.calls[0]![1]).toMatchObject({
      command: "write-memory",
      request: { extended: "memory_1.md", "expected-revision": "blake3:aa" },
      stdin: "body\n",
    });

    invoke.mockClear();
    await saveDocument("/v", "aios/conversations/2026-08-16_10-30-00.md", "body\n", "blake3:aa");
    expect(invoke.mock.calls[0]![1]).toMatchObject({
      command: "write-entry",
      request: { kind: "conversation", id: "2026-08-16_10-30-00.md" },
    });
  });

  it("refuses to invent a create form for content that always exists", async () => {
    // Both are present in any initialized vault, so a null revision there is a
    // caller mistake rather than a shorthand for "create it".
    await expect(saveDocument("/v", "aios/AGENTS.md", "x", null)).rejects.toThrow(
      /already exist/,
    );
    await expect(
      saveDocument("/v", "aios/notifications/2026-08-16_10-30-00.md", "x", null),
    ).rejects.toThrow(/created by an agent/);
  });
});
