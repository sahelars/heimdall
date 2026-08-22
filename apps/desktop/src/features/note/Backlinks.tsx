/**
 * Linked mentions: the notes that link to this one.
 *
 * Incoming links are `edges` reversed, which is why the index carries only one
 * direction and this panel is computed rather than fetched.
 *
 * Outgoing links are deliberately absent. They are already in the note's own
 * text a few lines above, and a second copy of them under the note is one more
 * list to read past rather than something new.
 *
 * A row is the linking note's name beside its path, both leading from the left
 * edge: the name is what you click, so nothing sits in front of it.
 */

import { titleOf } from "../../api/source";

interface BacklinksProps {
  /** Paths of the notes linking here, in the index's own order. */
  mentions: string[];
  onOpen: (path: string) => void;
}

export function Backlinks({ mentions, onOpen }: BacklinksProps) {
  if (mentions.length === 0) return null;

  return (
    <section className="backlinks" aria-label="Linked mentions">
      <h2 className="backlinks__title">Linked mentions ({mentions.length})</h2>
      <ul className="backlinks__list">
        {mentions.map((path) => (
          <li key={path} className="backlinks__item">
            <button type="button" className="backlink" onClick={() => onOpen(path)}>
              {titleOf(path)}
            </button>
            <span className="backlinks__path">{path}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
