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

export interface KnownClient {
  id: string;
  name: string;
  path: string;
  present: boolean;
  installed: boolean;
  /** Whether the entry already there names a command that has gone. */
  stale: boolean;
  serverKey: string;
}

export interface InstallOutcome {
  path: string;
  serverKey: string;
  replaced: boolean;
  backupPath?: string;
}

/** `heimdall create` (SPEC §7). */
export interface CreateVaultData {
  path: string;
  mode: "scaffolded" | "initialized";
  created: string[];
}

/**
 * Any bounded read (SPEC §8).
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

export interface DocumentRead extends Partial<ReadResult> {
  path: string;
  returned: boolean;
  reason?: string;
}

export interface ReadDocumentsData {
  documents: DocumentRead[];
  total_bytes: number;
  truncated: boolean;
}

export type DocumentKind = "directory" | "document";

export interface DocumentEntry {
  path: string;
  kind: DocumentKind;
  size_bytes?: number;
  modified_at: string;
}

export interface ListDocumentsData {
  entries: DocumentEntry[];
  next_cursor: string | null;
  scan_guard_hit: boolean;
}

/** The result of any client-side write (SPEC §15). */
export interface WriteDocumentData {
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

export interface DeletePathData {
  path: string;
  trashed_to: string;
  kind: DocumentKind;
}

export type EntryKind = "conversation" | "notification";

export interface EntryMeta {
  id: string;
  kind: EntryKind;
  created_at: string;
  size_bytes: number;
}

export interface ListEntriesData {
  entries: EntryMeta[];
  truncated: boolean;
}

/* The link graph (SPEC §8) ------------------------------------------------ */

export interface GraphNode {
  path: string;
  /** The filename stem — what the graph shows under each dot. */
  title: string;
  in_aios: boolean;
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

export interface MemoryEntry {
  name: string;
  kind: "main" | "extended";
  path: string;
  size_bytes: number;
  modified_at: string;
}

export interface ListMemoriesData {
  memories: MemoryEntry[];
  truncated: boolean;
}
