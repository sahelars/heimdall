/**
 * What to do when a note changed underneath the editor.
 *
 * Never a silent merge and never a silent overwrite. In this vault the other
 * writer is often an agent, which is exactly the case the revision contract
 * exists for — so the choice is put to the person, with the buffer untouched
 * until they make it.
 */

import { Button } from "../../components";

export interface Conflict {
  /** The revision now on disk, from the failed write. */
  theirRevision: string;
}

interface ConflictBarProps {
  conflict: Conflict;
  onResolve: (choice: "mine" | "theirs") => void;
}

export function ConflictBar({ onResolve }: ConflictBarProps) {
  return (
    <div className="notice notice--failure note__conflict" role="alert">
      <span className="notice__code">Revision conflict</span>
      <p>
        This note changed on disk after it was opened. Nothing has been overwritten, and your
        edits are still here.
      </p>
      <div className="row">
        <Button onClick={() => onResolve("theirs")}>Use the version on disk</Button>
        <Button onClick={() => onResolve("mine")}>Keep my version</Button>
      </div>
    </div>
  );
}
