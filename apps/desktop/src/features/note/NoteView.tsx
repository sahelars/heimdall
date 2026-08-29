/**
 * The middle pane: one note, in source or preview.
 */

import { useMemo, useState } from "react";

import { titleOf } from "../../api/source";
import type { DomainError } from "../../api/types";
import { IconBack, IconEdit, IconForward, IconMore, IconRead } from "../../components/icons";
import { Failure } from "../../components";
import { ContextMenu } from "../../components/ContextMenu";
import { joinNote, splitNote } from "../../markdown/frontmatter";
import { parseMarkdown, renderTokens, type RenderContext } from "../../markdown/render";
import { Backlinks } from "./Backlinks";
import { NoteTitle } from "./NoteTitle";
import { ConflictBar, type Conflict } from "./ConflictBar";
import { Properties } from "./Properties";
import { SourceEditor } from "./SourceEditor";

export type ViewMode = "source" | "preview";

export interface OpenNote {
  path: string;
  buffer: string;
  dirty: boolean;
  /** Null when the note cannot be saved from here — an entry, for instance. */
  editable: boolean;
  saving: boolean;
  conflict: Conflict | null;
  error: DomainError | null;
}

interface NoteViewProps {
  note: OpenNote | null;
  loading: boolean;
  /** What is being opened; differs from `note.path` during a slow read. */
  openingPath?: string | null;
  mode: ViewMode;
  /** Paths of the notes that link to this one. */
  mentions: string[];
  canBack: boolean;
  canForward: boolean;
  renderMermaid?: RenderContext["renderMermaid"];
  onModeChange: (mode: ViewMode) => void;
  onBack: () => void;
  onForward: () => void;
  onChange: (text: string) => void;
  onSave: () => void;
  onOpen: (path: string) => void;
  onOpenLink: (target: string, from: string) => void;
  resolves: (target: string, from: string) => boolean;
  onResolveConflict: (choice: "mine" | "theirs") => void;
  /** Null when the note cannot be renamed — the protected tree, for instance. */
  onRename: ((name: string) => void) | null;
}

export function NoteView(props: NoteViewProps) {
  const { note, mode } = props;
  const [addingProperty, setAddingProperty] = useState(false);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);

  const split = useMemo(() => (note ? splitNote(note.buffer) : null), [note?.buffer, note?.path]);
  const tokens = useMemo(
    () => (split && mode === "preview" ? parseMarkdown(split.body) : null),
    [split?.body, mode],
  );

  if (!note) {
    return (
      <p className="empty">
        {props.loading ? "Opening…" : "Select a note from the file tree to read or edit it."}
      </p>
    );
  }

  const context: RenderContext = {
    from: note.path,
    onOpenLink: props.onOpenLink,
    resolves: props.resolves,
    renderMermaid: props.renderMermaid,
  };

  return (
    <div className="note">
      {props.loading && props.openingPath !== note.path ? (
        <div className="note__loading" role="status">
          Opening {titleOf(props.openingPath ?? "")}…
        </div>
      ) : null}

      {/* Draggable for the same reason the file toolbar is: with the title
          bar overlaid, this bar is the top of the window. */}
      <header className="note__header" data-tauri-drag-region="deep">
        <div className="note__nav">
          <button
            type="button"
            className="icon-button"
            aria-label="Back"
            disabled={!props.canBack}
            onClick={props.onBack}
          >
            <IconBack />
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label="Forward"
            disabled={!props.canForward}
            onClick={props.onForward}
          >
            <IconForward />
          </button>
        </div>

        <Breadcrumb path={note.path} onOpen={props.onOpen} />

        <div className="note__actions">
          {note.dirty ? <span className="note__dirty" title="Unsaved changes">●</span> : null}
          <button
            type="button"
            className="icon-button"
            aria-label={mode === "source" ? "Read" : "Edit"}
            title={mode === "source" ? "Read" : "Edit"}
            onClick={() => props.onModeChange(mode === "source" ? "preview" : "source")}
          >
            {mode === "source" ? <IconRead /> : <IconEdit />}
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label="More"
            title="More"
            onClick={(event) => {
              const box = event.currentTarget.getBoundingClientRect();
              setMenuAt({ x: box.right - 170, y: box.bottom + 4 });
            }}
          >
            <IconMore />
          </button>
        </div>
      </header>

      {note.conflict ? (
        <ConflictBar conflict={note.conflict} onResolve={props.onResolveConflict} />
      ) : null}
      {note.error ? (
        <div className="note__error">
          <Failure error={note.error} />
        </div>
      ) : null}

      <div className="note__body">
        {mode === "source" ? (
          <SourceEditor
            path={note.path}
            value={note.buffer}
            editable={note.editable}
            onChange={props.onChange}
            onSave={props.onSave}
          />
        ) : (
          <article className="preview">
            <NoteTitle title={titleOf(note.path)} onRename={props.onRename} />
            <Properties
              frontmatter={split?.frontmatter ?? null}
              onChange={
                note.editable
                  ? (frontmatter) => props.onChange(joinNote(frontmatter, split?.body ?? ""))
                  : null
              }
              onOpenLink={(target) => props.onOpenLink(target, note.path)}
              adding={addingProperty}
              onAddingChange={setAddingProperty}
            />
            <hr className="preview__rule" />
            <div className="preview__content">{tokens ? renderTokens(tokens, context) : null}</div>
            <Backlinks mentions={props.mentions} onOpen={props.onOpen} />
          </article>
        )}
      </div>

      {menuAt ? (
        <ContextMenu
          x={menuAt.x}
          y={menuAt.y}
          items={[
            {
              label: mode === "source" ? "Read" : "Edit",
              onSelect: () => props.onModeChange(mode === "source" ? "preview" : "source"),
            },
            ...(note.editable
              ? [
                  {
                    label: "Add property",
                    onSelect: () => {
                      // Properties live in the rendered view, so adding one
                      // from the source view has to take you there.
                      props.onModeChange("preview");
                      setAddingProperty(true);
                    },
                  },
                ]
              : []),
          ]}
          onClose={() => setMenuAt(null)}
        />
      ) : null}
    </div>
  );
}

/** `projects / lens / how_lens_works`, with the folders clickable. */
function Breadcrumb({ path, onOpen }: { path: string; onOpen: (path: string) => void }) {
  const parts = path.split("/");
  const name = parts.pop() ?? path;

  return (
    <nav className="breadcrumb" aria-label="Location">
      {/* The folders give way first; the note's own name is the part worth
          keeping when there is not room for all of it. */}
      <span className="breadcrumb__folders">
        {parts.map((part, index) => (
          <span key={index}>
            <span className="breadcrumb__part">{part}</span>
            <span className="breadcrumb__separator"> / </span>
          </span>
        ))}
      </span>
      <button type="button" className="breadcrumb__current" onClick={() => onOpen(path)}>
        {titleOf(name)}
      </button>
    </nav>
  );
}
