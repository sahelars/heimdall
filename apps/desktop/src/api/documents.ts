/**
 * Whole documents, assembled from the CLI's bounded reads (SPEC §8, §15).
 *
 * Every read operation is capped at 1,000 lines and 256 KiB per call, because
 * those caps exist to bound what an AI client can pull in one request. An editor
 * needs the whole file, so it follows the continuation the contract provides
 * rather than asking for a bigger cap.
 *
 * Nothing outside this module calls `read-documents` or `write-document`.
 */

import { invokeCli } from "./cli";
import { baseName, sourceOf, type DocumentSource } from "./source";
import type {
  CreateFolderData,
  ReadResult,
  DeletePathData,
  DocumentEntry,
  DocumentRead,
  ListDocumentsData,
  MovePathData,
  ReadDocumentsData,
  WriteDocumentData,
} from "./types";
import type { DomainError } from "./types";

/** The CLI's own per-call ceilings (SPEC §8). */
const MAX_LINES = 1000;
const MAX_BYTES = 262_144;
/** Our own stop, at 64 chunks — 16 MiB, far past any real note. */
const MAX_CHUNKS = 64;
/** The CLI's maximum listing page. */
const PAGE = 200;
/** 8,000 entries before the tree admits it stopped rather than pretending. */
const MAX_PAGES = 40;
/** How many times a read restarts when the file changes underneath it. */
const MAX_REREADS = 2;

/** A domain failure carried as an exception, so callers can `try`/`catch`. */
export class CliFailure extends Error {
  readonly error: DomainError;
  readonly stderr?: string;

  constructor(error: DomainError, stderr?: string) {
    super(error.message);
    this.name = "CliFailure";
    this.error = error;
    this.stderr = stderr;
  }
}

function fail(code: string, message: string, details?: Record<string, unknown>): never {
  throw new CliFailure({ code, message, details });
}

export interface WholeDocument {
  path: string;
  content: string;
  /** The revision of the complete file, valid for a later write. */
  revision: string;
  sizeBytes: number;
  /** How many CLI calls it took, which Diagnostics can show. */
  chunks: number;
}

/**
 * Ask one command for one bounded range.
 *
 * Which command depends on where the note lives. `read-documents` excludes
 * `aios/` at every depth by design (SPEC §6), so a memory, an entry, or the
 * agent instructions has to be read by the operation that owns it — the editor
 * shows the whole vault, and reaching all of it through one command would mean
 * a general read tool, which is exactly what Heimdall does not have.
 */
async function readChunk(
  vault: string,
  path: string,
  source: DocumentSource,
  startLine: number,
): Promise<ReadResult> {
  const range = { "start-line": startLine, "max-lines": MAX_LINES };
  const id = baseName(path);

  const call = () => {
    switch (source) {
      case "agents":
        return invokeCli<ReadResult>("read-agents", { vault, ...range });
      case "memory-main":
        return invokeCli<ReadResult>("read-memory", { vault, ...range });
      case "memory-extended":
        return invokeCli<ReadResult>("read-memory", { vault, extended: id, ...range });
      case "entry-conversation":
        return invokeCli<ReadResult>("read-entry", { vault, kind: "conversation", id, ...range });
      case "entry-notification":
        return invokeCli<ReadResult>("read-entry", { vault, kind: "notification", id, ...range });
      case "document":
        return invokeCli<ReadDocumentsData>("read-documents", {
          vault,
          doc: [path],
          ...range,
          "max-total-bytes": MAX_BYTES,
        });
    }
  };

  const response = await call();
  if (!response.ok || !response.data) {
    throw new CliFailure(
      response.error ?? { code: "INTERNAL_ERROR", message: "the read returned nothing" },
      response.stderr,
    );
  }

  if (source !== "document") return response.data as ReadResult;

  const document: DocumentRead | undefined = (response.data as ReadDocumentsData).documents[0];
  if (!document) fail("INTERNAL_ERROR", `"${path}" was not in the read response`);
  if (!document.returned || document.revision === undefined) {
    fail("LIMIT_EXCEEDED", `"${path}" was not returned: ${document.reason ?? "unknown reason"}`);
  }
  return document as ReadResult;
}

/**
 * Read one complete note, whichever part of the vault it lives in.
 *
 * The revision each chunk reports covers the **whole file** at that moment, not
 * the returned range — `read_range` reads and hashes every byte before slicing
 * out the requested lines. So a file that changes mid-read is always visible
 * here as a revision that stopped matching, and the honest response is to start
 * again rather than stitch two versions together.
 */
export async function readWholeDocument(
  vault: string,
  path: string,
  attempt = 0,
): Promise<WholeDocument> {
  const source = sourceOf(path);
  let content = "";
  let line = 1;
  let revision: string | undefined;
  let sizeBytes = 0;
  let chunks = 0;

  while (chunks < MAX_CHUNKS) {
    const result = await readChunk(vault, path, source, line);

    if (revision === undefined) {
      revision = result.revision;
      sizeBytes = result.size_bytes ?? 0;
    } else if (result.revision !== revision) {
      if (attempt >= MAX_REREADS) {
        fail("IO_ERROR", `"${path}" kept changing while it was being read; try again`);
      }
      return readWholeDocument(vault, path, attempt + 1);
    }

    content += result.content ?? "";
    chunks += 1;
    if (result.complete) break;

    if (result.next_line == null) {
      fail("INTERNAL_ERROR", `"${path}" reported an incomplete read with no continuation point`);
    }
    line = result.next_line;
  }

  if (revision === undefined) fail("INTERNAL_ERROR", `"${path}" produced no content`);

  // `content.length` counts UTF-16 code units and `size_bytes` counts bytes, so
  // the comparison has to encode first. Skipping that would make every note
  // containing an emoji or an accent look truncated.
  const assembled = new TextEncoder().encode(content).length;
  if (assembled !== sizeBytes) {
    fail(
      "INTERNAL_ERROR",
      `"${path}" assembled to ${assembled} bytes but the vault reports ${sizeBytes}`,
      { path, assembled, expected: sizeBytes },
    );
  }

  return { path, content, revision, sizeBytes, chunks };
}

export interface SaveResult {
  revision: string;
  created: boolean;
}

/**
 * Save one note, wherever it lives.
 *
 * `expectedRevision` is the revision the buffer was read at; passing `null`
 * creates a note that does not exist yet. There is no third option — omitting it
 * is what the CLI treats as a caller mistake, precisely so a stale editor cannot
 * silently overwrite someone else's change.
 *
 * Entries and agent instructions have no create form: both always exist in an
 * initialized vault, so `null` there is a caller error rather than a shorthand.
 */
export async function saveDocument(
  vault: string,
  path: string,
  content: string,
  expectedRevision: string | null,
): Promise<SaveResult> {
  const source = sourceOf(path);
  const id = baseName(path);
  const revisionArgs =
    expectedRevision === null ? { create: true } : { "expected-revision": expectedRevision };

  const call = () => {
    switch (source) {
      case "agents":
        if (expectedRevision === null) {
          fail("INVALID_INPUT", "the agent instructions already exist; pass their revision");
        }
        return invokeCli<WriteDocumentData>(
          "write-agents",
          { vault, "expected-revision": expectedRevision },
          content,
        );
      case "memory-main":
        return invokeCli<WriteDocumentData>("write-memory", { vault, ...revisionArgs }, content);
      case "memory-extended":
        return invokeCli<WriteDocumentData>(
          "write-memory",
          { vault, extended: id, ...revisionArgs },
          content,
        );
      case "entry-conversation":
      case "entry-notification":
        if (expectedRevision === null) {
          fail("INVALID_INPUT", "entries are created by an agent, not by the editor");
        }
        return invokeCli<WriteDocumentData>(
          "write-entry",
          {
            vault,
            kind: source === "entry-conversation" ? "conversation" : "notification",
            id,
            "expected-revision": expectedRevision,
          },
          content,
        );
      case "document":
        return invokeCli<WriteDocumentData>(
          "write-document",
          { vault, path, ...revisionArgs },
          // Markdown always travels on stdin, never as an argument (SPEC §11).
          content,
        );
    }
  };

  const response = await call();
  if (!response.ok || !response.data) {
    throw new CliFailure(
      response.error ?? { code: "INTERNAL_ERROR", message: "the write returned nothing" },
      response.stderr,
    );
  }
  return {
    revision: response.data.new_revision,
    created: response.data.created ?? false,
  };
}

export async function createFolder(vault: string, path: string): Promise<string[]> {
  const response = await invokeCli<CreateFolderData>("create-folder", { vault, path });
  if (!response.ok || !response.data) {
    throw new CliFailure(
      response.error ?? { code: "INTERNAL_ERROR", message: "the folder was not created" },
    );
  }
  return response.data.created;
}

export async function movePath(vault: string, from: string, to: string): Promise<MovePathData> {
  const response = await invokeCli<MovePathData>("move-path", { vault, from, to });
  if (!response.ok || !response.data) {
    throw new CliFailure(response.error ?? { code: "INTERNAL_ERROR", message: "the move failed" });
  }
  return response.data;
}

export async function deletePath(
  vault: string,
  path: string,
  expectedRevision?: string,
): Promise<DeletePathData> {
  const response = await invokeCli<DeletePathData>("delete-path", {
    vault,
    path,
    ...(expectedRevision ? { "expected-revision": expectedRevision } : {}),
  });
  if (!response.ok || !response.data) {
    throw new CliFailure(response.error ?? { code: "INTERNAL_ERROR", message: "the delete failed" });
  }
  return response.data;
}

export interface Listing {
  entries: DocumentEntry[];
  /** True when the listing stopped early, so the tree can say so. */
  truncated: boolean;
  scanGuardHit: boolean;
}

/**
 * Every ordinary note and folder in the vault.
 *
 * Eager rather than lazy-on-expand: the quick switcher, wikilink resolution, and
 * the graph's orphan nodes all need the complete path list immediately, and each
 * call is a process spawn, so one paged burst is cheaper than one call per
 * folder the user opens.
 *
 * This never sees `aios/` — `list-documents` excludes it at every depth by
 * design. The protected tree reaches the sidebar through the link index.
 */
export async function listAllDocuments(vault: string): Promise<Listing> {
  const entries: DocumentEntry[] = [];
  let cursor: string | null = null;
  let pages = 0;
  let scanGuardHit = false;

  do {
    const response: Awaited<ReturnType<typeof invokeCli<ListDocumentsData>>> =
      await invokeCli<ListDocumentsData>("list-documents", {
        vault,
        recursive: true,
        "max-depth": 16,
        limit: PAGE,
        ...(cursor ? { cursor } : {}),
      });
    if (!response.ok || !response.data) {
      throw new CliFailure(
        response.error ?? { code: "INTERNAL_ERROR", message: "the listing returned nothing" },
        response.stderr,
      );
    }

    entries.push(...response.data.entries);
    scanGuardHit = scanGuardHit || response.data.scan_guard_hit;
    cursor = response.data.next_cursor;
    pages += 1;
  } while (cursor && pages < MAX_PAGES);

  return { entries, truncated: Boolean(cursor), scanGuardHit };
}
