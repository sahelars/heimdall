/**
 * Appearance: the theme override, and an accent for each theme.
 *
 * The theme default follows the platform, which is what SPEC §15 asks for. The
 * override exists so both modes can be looked at without changing a system
 * setting — and because someone whose machine is light may still want the black
 * vault.
 *
 * There is an accent per theme rather than one shared between them, because no
 * single colour works on both grounds: white is the right accent on black and
 * invisible on white, and black is the reverse. Both wells are shown at once,
 * including the one for the theme that is not currently on screen, so a choice
 * for the other theme does not mean switching to it first.
 *
 * Nothing here names a colour — each well and field shows the choice, or the
 * default the stylesheet declares for its theme, and "Default" clears that
 * choice rather than writing a value back, which is what leaves that theme
 * monochrome. Each well is drawn on its own theme's ground, so a default that
 * matches the page — white for dark mode, on a light page — still reads as a
 * colour rather than as an empty box.
 */

import { useState } from "react";

import { Button, Field, Panel } from "../../components";
import { defaultAccent, isAccent, type AccentTheme, type Accents } from "../../state/accent";
import { THEME_PREFERENCES, type ThemePreference } from "../../state/theme";

const LABELS: Record<ThemePreference, string> = {
  system: "System",
  light: "Light",
  dark: "Dark",
};

const ACCENT_THEMES: { theme: AccentTheme; label: string }[] = [
  { theme: "light", label: "Light" },
  { theme: "dark", label: "Dark" },
];

interface AppearanceProps {
  preference: ThemePreference;
  onChange: (preference: ThemePreference) => void;
  /** One per theme; null in either is that theme's stylesheet default. */
  accents: Accents;
  onAccentsChange: (accents: Accents) => void;
}

interface AccentWellProps {
  theme: AccentTheme;
  label: string;
  accent: string | null;
  onChange: (accent: string | null) => void;
}

function AccentWell({ theme, label, accent, onChange }: AccentWellProps) {
  // What is being typed, which is only sometimes a colour: a half-written hex
  // must stay on the screen rather than being rejected a character at a time.
  // Per well, so typing in one does not disturb the other.
  const [typed, setTyped] = useState<string | null>(null);
  // The choice comes from props, never from the document: the document is
  // updated in an effect after this render, so it would still hold a choice
  // that has just been cleared.
  const current = accent ?? defaultAccent(document.documentElement, theme);

  // A colour input's value is lowercase hex, so a value typed in capitals has
  // to arrive that way too or the two controls disagree about what is set.
  const choose = (value: string) => onChange(value.toLowerCase());

  return (
    <div className="row">
      <div className="field">
        <label className="field__label" htmlFor={`accent-colour-${theme}`}>
          {label}
        </label>
        <input
          id={`accent-colour-${theme}`}
          className={`swatch swatch--${theme}`}
          type="color"
          value={current}
          onChange={(event) => {
            setTyped(null);
            choose(event.target.value);
          }}
        />
      </div>

      <Field
        id={`accent-hex-${theme}`}
        label={`${label} hex`}
        value={typed ?? current.toUpperCase()}
        onChange={(value) => {
          setTyped(value);
          if (isAccent(value)) choose(value);
        }}
      />

      <Button
        onClick={() => {
          setTyped(null);
          onChange(null);
        }}
        disabled={accent === null}
      >
        Default
      </Button>
    </div>
  );
}

export function Appearance({ preference, onChange, accents, onAccentsChange }: AppearanceProps) {
  return (
    <div className="stack">
      <Panel
        title="Theme"
        description="Dark is pure black behind white; light is the exact inverse."
      >
        <div className="choice" role="radiogroup" aria-label="Theme">
          {THEME_PREFERENCES.map((option) => (
            <label
              key={option}
              className={`choice__option${preference === option ? " choice__option--on" : ""}`}
            >
              <input
                type="radio"
                name="theme"
                value={option}
                checked={preference === option}
                onChange={() => onChange(option)}
              />
              {LABELS[option]}
            </label>
          ))}
        </div>
      </Panel>

      <Panel
        title="Accent"
        description="One colour per theme, spent on links, the note you have open, the active graph node, and mermaid diagrams. Each is used as picked, so choose one that reads against its own background. Default leaves that theme monochrome."
      >
        <div className="stack">
          {ACCENT_THEMES.map(({ theme, label }) => (
            <AccentWell
              key={theme}
              theme={theme}
              label={label}
              accent={accents[theme]}
              onChange={(accent) => onAccentsChange({ ...accents, [theme]: accent })}
            />
          ))}
        </div>
      </Panel>
    </div>
  );
}
