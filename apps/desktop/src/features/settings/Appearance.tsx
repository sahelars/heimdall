/**
 * Appearance: the theme override.
 *
 * The default follows the platform, which is what SPEC §15 asks for. The
 * override exists so both modes can be looked at without changing a system
 * setting — and because someone whose machine is light may still want the black
 * vault.
 */

import { Panel } from "../../components";
import { THEME_PREFERENCES, type ThemePreference } from "../../state/theme";

const LABELS: Record<ThemePreference, string> = {
  system: "System",
  light: "Light",
  dark: "Dark",
};

interface AppearanceProps {
  preference: ThemePreference;
  onChange: (preference: ThemePreference) => void;
}

export function Appearance({ preference, onChange }: AppearanceProps) {
  return (
    <Panel
      title="Theme"
      description="Dark is pure black with green links; light is the exact inverse, with black links."
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
  );
}
