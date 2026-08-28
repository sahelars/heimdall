/**
 * The frontmatter table, as a property editor shows it.
 *
 * Edits are applied to the parsed YAML document and handed back, so everything
 * the user did not touch keeps its formatting.
 *
 * The table is only shown for a note that actually has a `---` block. A note
 * without one gets the "Add property" affordance and nothing else — a heading
 * and an empty grid over a note with no frontmatter is furniture, not
 * information.
 */

import { useState } from "react";

import { IconClose } from "../../components/icons";
import {
  addProperty,
  isListKey,
  parseFrontmatter,
  readProperties,
  removeProperty,
  removeValue,
  serializeFrontmatter,
} from "../../markdown/frontmatter";
import { parseWikilink } from "../../markdown/wikilinks";

interface PropertiesProps {
  /** The raw block, or null when the note has no `---` fences. */
  frontmatter: string | null;
  /** Null while the note cannot be edited, which hides every control. */
  onChange: ((frontmatter: string) => void) | null;
  onOpenLink: (target: string) => void;
  /** Whether the "add a property" form is open, driven from the note header. */
  adding: boolean;
  onAddingChange: (adding: boolean) => void;
}

export function Properties({
  frontmatter,
  onChange,
  onOpenLink,
  adding,
  onAddingChange,
}: PropertiesProps) {
  const [name, setName] = useState("");
  const [value, setValue] = useState("");

  const document = parseFrontmatter(frontmatter);
  const properties = readProperties(document);
  const readOnly = onChange === null;
  const hasBlock = frontmatter !== null && properties.length > 0;

  // A note with no properties shows no properties. An empty grid under a
  // heading is furniture; the way to add the first one is the note's own menu.
  if (!hasBlock && !adding) return null;

  const edit = (mutate: () => void) => {
    if (!onChange) return;
    mutate();
    onChange(serializeFrontmatter(document));
  };

  const submit = () => {
    const key = name.trim();
    if (key) edit(() => addProperty(document, key, value.trim()));
    setName("");
    setValue("");
    onAddingChange(false);
  };

  return (
    <section className="properties" aria-label="Properties">
      {hasBlock ? (
        <>
          <h2 className="properties__title">Properties</h2>

          <dl className="properties__grid">
            {properties.map((property) => (
              <div key={property.key} className="properties__row">
                <dt className="properties__key">
                  <span>{property.key}</span>
                  {readOnly ? null : (
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={`Remove ${property.key}`}
                      onClick={() => edit(() => removeProperty(document, property.key))}
                    >
                      <IconClose size={12} />
                    </button>
                  )}
                </dt>
                <dd className="properties__values">
                  {property.values.length === 0 ? (
                    <span className="muted">empty</span>
                  ) : (
                    property.values.map((entry, index) => (
                      <Value
                        key={`${entry}-${index}`}
                        value={entry}
                        readOnly={readOnly}
                        onOpen={onOpenLink}
                        onRemove={() => edit(() => removeValue(document, property.key, entry))}
                      />
                    ))
                  )}
                </dd>
              </div>
            ))}
          </dl>
        </>
      ) : null}

      {readOnly || !adding ? null : (
        <form
          className="properties__add"
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              onAddingChange(false);
            }
          }}
        >
          <div className="field">
            <label className="field__label" htmlFor="property-name">
              Property
            </label>
            <input
              id="property-name"
              className="input"
              autoFocus
              placeholder="links"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </div>

          <div className="field field--grow">
            <label className="field__label" htmlFor="property-value">
              Value
            </label>
            <input
              id="property-value"
              className="input"
              placeholder={isListKey(name) ? "[[a note]]" : "a value"}
              value={value}
              onChange={(event) => setValue(event.target.value)}
            />
          </div>

          <button type="submit" className="button" disabled={name.trim() === ""}>
            Add
          </button>
          <button type="button" className="button" onClick={() => onAddingChange(false)}>
            Cancel
          </button>
        </form>
      )}
    </section>
  );
}

/** One value: a wikilink becomes a link, anything else stays text. */
function Value({
  value,
  readOnly,
  onOpen,
  onRemove,
}: {
  value: string;
  readOnly: boolean;
  onOpen: (target: string) => void;
  onRemove: () => void;
}) {
  const link =
    value.startsWith("[[") && value.endsWith("]]") ? parseWikilink(value.slice(2, -2)) : null;

  return (
    <span className="properties__value">
      {link ? (
        <button type="button" className="property__value" onClick={() => onOpen(link.target)}>
          {link.label}
        </button>
      ) : (
        <span>{value}</span>
      )}
      {readOnly ? null : (
        <button
          type="button"
          className="icon-button"
          aria-label={`Remove ${link?.label ?? value}`}
          onClick={onRemove}
        >
          <IconClose size={11} />
        </button>
      )}
    </span>
  );
}
