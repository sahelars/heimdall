/**
 * Whole documents, assembled from the CLI's bounded reads (SPEC §8, §15).
 *
 * Every read is capped at 1,000 lines and 256 KiB per call, because those caps
 * exist to bound what an AI client can pull in one request. An editor needs the
 * whole file, so it follows the continuation the contract provides rather than
 * asking for a bigger cap.
 *
 * Nothing outside this module calls `read`, `write`, `lock` or `unlock`.
 */

import { invokeCli } from "./cli";
import type {
  CreateFolderData,
  DeletePathData,
  DocumentEntry,
  LockData,
  MovePathData,
  ReadData,
  ReadResult,
  RelinkData,
  WriteData,
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
  /** Whether the note is read-only, by its own rule or an enclosing one. */
  locked: boolean;
  /** Where the rule that locks it lives (`""` is the vault root); null when unlocked. */
  lockedAt: string | null;
  /** How many CLI calls it took, which Diagnostics can show. */
  chunks: number;
}

interface Chunk {
  document: ReadResult;
  locked: boolean;
  lockedAt: string | null;
}

/** Ask `read` for one bounded range of one note. */
async function readChunk(
  vault: string,
  path: string,
  startLine: number,
  maxLines = MAX_LINES,
): Promise<Chunk> {
  const response = await invokeCli<ReadData>("read", {
    vault,
    path,
    "start-line": startLine,
    "max-lines": maxLines,
    "max-total-bytes": MAX_BYTES,
  });
  if (!response.ok || !response.data) {
    throw new CliFailure(
      response.error ?? { code: "INTERNAL_ERROR", message: "the read returned nothing" },
      response.stderr,
    );
  }

  const { document, locked, locked_at: lockedAt } = response.data;
  if (!document) fail("INVALID_INPUT", `"${path}" is a folder, not a note`);
  return { document, locked: Boolean(locked), lockedAt: locked ? (lockedAt ?? null) : null };
}

/**
 * Read one complete note.
 *
 * The revision each chunk reports covers the **whole file** at that moment, not
 * the returned range — the CLI reads and hashes every byte before slicing out
 * the requested lines. So a file that changes mid-read is always visible here as
 * a revision that stopped matching, and the honest response is to start again
 * rather than stitch two versions together.
 *
 * The lock state is the one the first chunk reported. A lock does not change
 * the bytes, so it cannot tear a read; whatever changes it afterwards is caught
 * by the write, which refuses a locked note whatever the editor believed.
 */
export async function readWholeDocument(
  vault: string,
  path: string,
  attempt = 0,
): Promise<WholeDocument> {
  let content = "";
  let line = 1;
  let revision: string | undefined;
  let sizeBytes = 0;
  let locked = false;
  let lockedAt: string | null = null;
  let chunks = 0;

  while (chunks < MAX_CHUNKS) {
    const { document: result, ...state } = await readChunk(vault, path, line);

    if (revision === undefined) {
      revision = result.revision;
      sizeBytes = result.size_bytes ?? 0;
      locked = state.locked;
      lockedAt = state.lockedAt;
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

  return { path, content, revision, sizeBytes, locked, lockedAt, chunks };
}

/**
 * Whether one path is locked right now, and by which rule.
 *
 * A one-line read: the lock state is on every `read` response, and asking for
 * as little of the note as the contract allows keeps this cheap enough to run
 * after every lock or unlock.
 */
export async function readLockState(
  vault: string,
  path: string,
): Promise<{ locked: boolean; lockedAt: string | null }> {
  const { locked, lockedAt } = await readChunk(vault, path, 1, 1);
  return { locked, lockedAt };
}

export interface SaveResult {
  revision: string;
  created: boolean;
}

/**
 * Save one note.
 *
 * `expectedRevision` is the revision the buffer was read at; passing `null`
 * creates a note that does not exist yet, and is refused with
 * `REVISION_CONFLICT` if one does. There is no third option, precisely so a
 * stale editor cannot silently overwrite someone else's change.
 *
 * A locked note, or a new one inside a locked folder, is refused with `LOCKED`.
 */
export async function saveDocument(
  vault: string,
  path: string,
  content: string,
  expectedRevision: string | null,
): Promise<SaveResult> {
  const revisionArgs =
    expectedRevision === null ? { create: true } : { "expected-revision": expectedRevision };

  const response = await invokeCli<WriteData>(
    "write",
    { vault, path, ...revisionArgs },
    // Markdown always travels on stdin, never as an argument (SPEC §11).
    content,
  );
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

/**
 * Make a note or folder read-only, or writable again. `null` is the whole vault.
 *
 * A folder's rule covers everything inside it and clears any exception nested
 * beneath it, so one note can then be set the other way on its own.
 */
export function lockPath(vault: string, path: string | null): Promise<LockData> {
  return setLock("lock", vault, path);
}

export function unlockPath(vault: string, path: string | null): Promise<LockData> {
  return setLock("unlock", vault, path);
}

async function setLock(
  command: "lock" | "unlock",
  vault: string,
  path: string | null,
): Promise<LockData> {
  const response = await invokeCli<LockData>(command, {
    vault,
    ...(path ? { path } : {}),
  });
  if (!response.ok || !response.data) {
    throw new CliFailure(
      response.error ?? { code: "INTERNAL_ERROR", message: `the ${command} returned nothing` },
      response.stderr,
    );
  }
  return response.data;
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

/**
 * Carry the links that pointed at `from` over to `to`.
 *
 * Called straight after `movePath`, never instead of it. Only the notes holding
 * a link to the moved path are written, each one retargeted by the same
 * resolver the graph is drawn from — so a rewritten link goes exactly where the
 * reader could already see it going.
 *
 * Whole-vault work, so the bridge gives it the long timeout rather than the
 * ordinary one.
 */
export async function relinkPaths(vault: string, from: string, to: string): Promise<RelinkData> {
  const response = await invokeCli<RelinkData>("relink", { vault, from, to });
  if (!response.ok || !response.data) {
    throw new CliFailure(
      response.error ?? { code: "INTERNAL_ERROR", message: "the links were not updated" },
      response.stderr,
    );
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

export interface VaultListing {
  entries: DocumentEntry[];
  /** Whether the vault root itself is locked — every path in it then is too. */
  rootLocked: boolean;
  /** True when the listing stopped early, so the tree can say so. */
  truncated: boolean;
  scanGuardHit: boolean;
}

/**
 * Every note and folder in the vault.
 *
 * Eager rather than lazy-on-expand: the quick switcher, wikilink resolution, and
 * the graph's orphan nodes all need the complete path list immediately, and each
 * call is a process spawn, so one paged burst is cheaper than one call per
 * folder the user opens.
 */
export async function listAllDocuments(vault: string): Promise<VaultListing> {
  const entries: DocumentEntry[] = [];
  let cursor: string | null = null;
  let pages = 0;
  let scanGuardHit = false;
  let rootLocked = false;

  do {
    const response: Awaited<ReturnType<typeof invokeCli<ReadData>>> = await invokeCli<ReadData>(
      "read",
      {
        vault,
        recursive: true,
        "max-depth": 16,
        limit: PAGE,
        ...(cursor ? { cursor } : {}),
      },
    );
    if (!response.ok || !response.data) {
      throw new CliFailure(
        response.error ?? { code: "INTERNAL_ERROR", message: "the listing returned nothing" },
        response.stderr,
      );
    }
    const listing = response.data.listing;
    if (!listing) fail("INTERNAL_ERROR", "the vault root read did not return a listing");

    if (pages === 0) rootLocked = Boolean(response.data.locked);
    entries.push(...listing.entries);
    scanGuardHit = scanGuardHit || listing.scan_guard_hit;
    cursor = listing.next_cursor;
    pages += 1;
  } while (cursor && pages < MAX_PAGES);

  return { entries, rootLocked, truncated: Boolean(cursor), scanGuardHit };
}
