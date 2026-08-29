/**
 * Appearance: the theme override, and dark mode's accent.
 *
 * The theme default follows the platform, which is what SPEC §15 asks for. The
 * override exists so both modes can be looked at without changing a system
 * setting — and because someone whose machine is light may still want the black
 * vault.
 *
 * The accent is dark mode's only: light mode has no hue at all, so there is
 * nothing there to choose. Nothing here names a colour — the well and the field
 * both show what the stylesheet currently resolves to, and "Default" clears the
 * choice rather than writing a value back.
 */

import { useState } from "react";

import { Button, Field, Panel } from "../../components";
import { isAccent, resolvedAccent } from "../../state/accent";
import { THEME_PREFERENCES, type ThemePreference } from "../../state/theme";

const LABELS: Record<ThemePreference, string> = {
  system: "System",
  light: "Light",
  dark: "Dark",
};

interface AppearanceProps {
  preference: ThemePreference;
  onChange: (preference: ThemePreference) => void;
  /** Null is the stylesheet's own accent — see `state/accent.ts`. */
  accent: string | null;
  onAccentChange: (accent: string | null) => void;
}

export function Appearance({ preference, onChange, accent, onAccentChange }: AppearanceProps) {
  // What is being typed, which is only sometimes a colour: a half-written hex
  // must stay on the screen rather than being rejected a character at a time.
  const [typed, setTyped] = useState<string | null>(null);

  const current = accent ?? resolvedAccent(document.documentElement);

  // A colour input's value is lowercase hex, so a value typed in capitals has
  // to arrive that way too or the two controls disagree about what is set.
  const choose = (value: string) => onAccentChange(value.toLowerCase());

  return (
    <div className="stack">
      <Panel
        title="Theme"
        description="Dark is pure black with accented links; light is the exact inverse, with black links."
      >
        <div className="choice" role="radiogroup" aria-label="Theme">
          {THEME_PREFERENCES.map((option) => (
            <label
              key={option}
              className={
                option === preference ? "choice__option choice__option--selected" : "choice__option"
              }
            >
              <input
                type="radio"
                name="theme"
                value={option}
                checked={option === preference}
                onChange={() => onChange(option)}
              />
              {LABELS[option]}
            </label>
          ))}
        </div>
      </Panel>

      <Panel
        title="Accent"
        description="Dark mode spends one colour on links, the note you have open, the active graph node, and mermaid diagrams. Light mode has no hue at all, so this does not change it."
      >
        <div className="row">
          <div className="field">
            <label className="field__label" htmlFor="accent-colour">
              Colour
            </label>
            <input
              id="accent-colour"
              className="swatch"
              type="color"
              value={current}
              onChange={(event) => {
                setTyped(null);
                choose(event.target.value);
              }}
            />
          </div>

          <Field
            id="accent-hex"
            label="Hex"
            value={typed ?? current.toUpperCase()}
            onChange={(value) => {
              setTyped(value);
              if (isAccent(value)) choose(value);
            }}
          />

          <Button
            onClick={() => {
              setTyped(null);
              onAccentChange(null);
            }}
            disabled={accent === null}
          >
            Default
          </Button>
        </div>
      </Panel>
    </div>
  );
}
