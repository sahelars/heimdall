/**
 * Colour lives in styles.css, where the visual rules can be checked.
 *
 * The editor theme and the graph renderer are JavaScript, so a colour literal
 * in either would be one `design.test.ts` could never see — it reads the
 * stylesheet, not the bundle. Both read CSS custom properties instead, and this
 * is what keeps it that way.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const SRC = dirname(fileURLToPath(import.meta.url));

/** Every application source file, tests excluded. */
function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    if (!/\.tsx?$/.test(name) || /\.test\.tsx?$/.test(name)) return [];
    return [path];
  });
}

describe("colour lives in css", () => {
  it("has no colour literal anywhere in the application source", () => {
    const files = sources(SRC);
    expect(files.length).toBeGreaterThan(0);

    for (const file of files) {
      const code = readFileSync(file, "utf8")
        .replace(/\/\*[\s\S]*?\*\//g, "")
        .replace(/\/\/.*/g, "");
      const name = relative(SRC, file);
      expect(code.match(/#[0-9a-fA-F]{6}\b|#[0-9a-fA-F]{3}\b(?![0-9a-fA-F])/g) ?? [], name).toEqual(
        [],
      );
      expect(code, name).not.toMatch(/\b(?:rgb|rgba|hsl|hsla|oklch)\(/);
    }
  });
});
