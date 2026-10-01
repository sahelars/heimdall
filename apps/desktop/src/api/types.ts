/**
 * The contract the bundled CLI publishes, as React consumes it.
 *
 * These mirror the CLI's versioned envelope rather than any internal type: the
 * desktop is a client of the published contract, not of `heimdall-core`.
 */

/** One of the fixed domain codes from SPEC §11. */
export type DomainCode =
  | "INVALID_INPUT"
  | "LIMIT_EXCEEDED"
  | "PATH_OUTSIDE_VAULT"
  | "NOT_FOUND"
  | "ALREADY_EXISTS"
  | "NOT_INITIALIZED"
  | "REVISION_CONFLICT"
  | "LOCKED"
  | "NOT_CONFIRMED"
  | "IO_ERROR"
  | "INTERNAL_ERROR";

export interface DomainError {
  code: DomainCode | string;
  message: string;
  details?: Record<string, unknown>;
}

export interface CliResponse<T = unknown> {
  ok: boolean;
  data?: T;
  error?: DomainError;
  schemaVersion?: number;
  exitCode?: number;
  stderr?: string;
}

export interface CliStatus {
  /** Absolute path of the bundled sidecar this app runs. */
  path: string;
  available: boolean;
  /** Whether that path is a build artifact rather than a shipped app's. */
  developmentBuild: boolean;
  cliVersion?: string;
  coreVersion?: string;
  mcpProtocolVersion?: string;
  outputSchemaVersion?: number;
  error?: DomainError;
}

export interface HealthCheck {
  ok: boolean;
  serverName?: string;
  serverVersion?: string;
  protocolVersion?: string;
  instructions?: string;
  toolCount?: number;
  error?: DomainError;
  stderr?: string;
}

/** An older Heimdall entry that served one vault, which installing replaces. */
export interface LegacyEntry {
  key: string;
  vault: string;
}

export interface KnownClient {
  id: string;
  name: string;
  path: string;
  present: boolean;
  /** Whether it has the one `heimdall` entry, serving the shared vaults. */
  installed: boolean;
  /** Whether the entry already there names a command that has gone. */
  stale: boolean;
  /** Per-vault entries from older builds; their vaults are shared first. */
  legacy: LegacyEntry[];
  /** Set when `heimdall` there runs something else, which is never overwritten. */
  conflict?: string;
}

export interface InstallOutcome {
  path: string;
  serverKey: string;
  replaced: boolean;
  /** Older per-vault entries removed in favour of the one entry. */
  removed: string[];
  backupPath?: string;
}

/** One vault Heimdall knows, from `heimdall vaults`. */
export interface KnownVault {
  path: string;
  folder: string;
  exists: boolean;
  /** Whether AI clients can reach it. */
  shared: boolean;
  /** The name tool calls use for it, when shared. */
  name: string | null;
}

export interface VaultsData {
  vaults: KnownVault[];
}

/**
 * `heimdall create` (SPEC §7).
 *
 * `scaffolded`: an empty or missing folder was given the template.
 * `registered`: an existing folder of notes was adopted as a vault, and nothing
 * was written into it.
 */
export interface CreateVaultData {
  path: string;
  mode: "scaffolded" | "registered";
  created: string[];
}

/**
 * One bounded range of a note (SPEC §8).
 *
 * The CLI emits snake_case, so these are the wire names. `next_line` and
 * `complete` together are what let a caller assemble a whole file without
 * guessing whether it has all of it.
 */
export interface ReadResult {
  path: string;
  content: string;
  start_line: number;
  end_line: number;
  next_line: number | null;
  complete: boolean;
  size_bytes: number;
  revision: string;
}

export type DocumentKind = "directory" | "document";

export interface DocumentEntry {
  path: string;
  kind: DocumentKind;
  size_bytes?: number;
  modified_at: string;
  /** Whether the entry is read-only, by its own rule or an enclosing one. */
  locked: boolean;
}

/** One page of a folder's contents. */
export interface Listing {
  entries: DocumentEntry[];
  next_cursor: string | null;
  scan_guard_hit: boolean;
}

/**
 * `read`: a folder's listing or one range of a note.
 *
 * `locked_at` is present only when `locked` is — the folder or note whose rule
 * applies, with `""` meaning the vault root.
 */
export interface ReadData {
  /** The vault's name: its shared name, or its folder when it is not shared. */
  vault?: string;
  path: string;
  kind: DocumentKind;
  locked: boolean;
  locked_at?: string;
  listing?: Listing;
  document?: ReadResult;
}

/** `lock` / `unlock`. `changed` is false when the path was already that way. */
export interface LockData {
  path: string;
  kind: DocumentKind;
  locked: boolean;
  changed: boolean;
}

/** The result of any client-side write (SPEC §15). */
export interface WriteData {
  path: string;
  new_revision: string;
  size_bytes: number;
  created: boolean;
}

export interface CreateFolderData {
  path: string;
  created: string[];
}

export interface MovePathData {
  from: string;
  to: string;
  kind: DocumentKind;
}

export interface RelinkUpdate {
  path: string;
  /** How many links in this file were retargeted. */
  links: number;
  new_revision: string;
}

export interface RelinkSkip {
  path: string;
  /** The link's target, exactly as it still stands in the file. */
  target: string;
  reason: "unresolvable";
}

export interface RelinkTruncation {
  file_cap_hit: boolean;
  node_cap_hit: boolean;
  total_bytes_cap_hit: boolean;
  nodes_omitted: number;
  files_unscanned: number;
  scanned_bytes: number;
}

export interface RelinkData {
  from: string;
  to: string;
  dry_run: boolean;
  updated: RelinkUpdate[];
  skipped: RelinkSkip[];
  /**
   * Notes that link to the moved path but were left unchanged because they are
   * locked. Their links still point at the old name.
   */
  locked: string[];
  truncated: RelinkTruncation;
}

export interface DeletePathData {
  path: string;
  trashed_to: string;
  kind: DocumentKind;
}

/* The link graph (SPEC §8) ------------------------------------------------ */

export interface GraphNode {
  path: string;
  /** The filename stem — what the graph shows under each dot. */
  title: string;
  /** Whether the note is read-only. */
  locked: boolean;
  size_bytes: number;
  modified_at: string;
  /**
   * False when the note was too large, unreadable, or past the scan budget, so
   * its outgoing links are absent rather than known to be empty.
   */
  scanned: boolean;
}

/** Endpoints are indices into `nodes`, which is what keeps the payload small. */
export interface GraphEdge {
  from: number;
  to: number;
  count: number;
}

export interface UnresolvedLink {
  from: number;
  target: string;
  count: number;
}

export interface GraphTruncation {
  node_cap_hit: boolean;
  total_bytes_cap_hit: boolean;
  nodes_omitted: number;
  files_unscanned: number;
  scanned_bytes: number;
}

export interface LinkGraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
  unresolved: UnresolvedLink[];
  truncated: GraphTruncation;
}
