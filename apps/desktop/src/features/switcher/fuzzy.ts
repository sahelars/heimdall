/**
 * Fuzzy matching for the quick switcher.
 *
 * A subsequence scorer rather than a dependency: the whole thing is smaller than
 * any package that would do it, and being pure makes its ranking testable, which
 * is the part that decides whether the switcher feels right.
 */

export interface Match {
  path: string;
  score: number;
  /** Indices into `path` that matched, for highlighting. */
  positions: number[];
}

/**
 * A run of letters typed in sequence is the strongest signal, so it outweighs a
 * word boundary — otherwise `n_o_t_e_x` beats `note` for "note", because every
 * letter of the first sits after an underscore.
 */
const CONSECUTIVE_BONUS = 12;
const BOUNDARY_BONUS = 8;
/** What someone types is nearly always the note's name, not its folder. */
const BASENAME_WEIGHT = 2;

/**
 * Score one candidate, or null when the query is not a subsequence of it.
 *
 * Matching is case-insensitive and greedy left to right, which is what makes
 * "hlw" find "how_lens_works".
 */
export function score(query: string, path: string): Match | null {
  const needle = query.toLowerCase().replace(/\s+/g, "");
  if (needle === "") return { path, score: 0, positions: [] };

  const haystack = path.toLowerCase();
  const basenameStart = path.lastIndexOf("/") + 1;

  const positions: number[] = [];
  let total = 0;
  let cursor = 0;
  let previous = -2;

  for (const character of needle) {
    const earliest = haystack.indexOf(character, cursor);
    if (earliest === -1) return null;

    // Purely greedy matching takes the first occurrence, which misses the
    // better one: "w" against `how_works` would land on the `w` of `how` and
    // never see the one starting `works`. Continuing a run still wins, since
    // that is the stronger signal.
    const consecutive = earliest === previous + 1;
    const found = consecutive ? earliest : (boundaryAt(haystack, path, character, cursor) ?? earliest);

    let points = 1;
    if (found === previous + 1) points += CONSECUTIVE_BONUS;
    else if (isBoundary(path, found)) points += BOUNDARY_BONUS;
    if (found >= basenameStart) points *= BASENAME_WEIGHT;

    total += points;
    positions.push(found);
    previous = found;
    cursor = found + 1;
  }

  // A shorter path matching the same query is the more likely intent.
  return { path, score: total - path.length * 0.1, positions };
}

/** Whether the character at `at` starts a word. */
function isBoundary(path: string, at: number): boolean {
  return at === 0 || "/_- .".includes(path[at - 1] ?? "");
}

/** The first occurrence at or after `from` that starts a word. */
function boundaryAt(
  haystack: string,
  path: string,
  character: string,
  from: number,
): number | null {
  for (let at = haystack.indexOf(character, from); at !== -1; at = haystack.indexOf(character, at + 1)) {
    if (isBoundary(path, at)) return at;
  }
  return null;
}

/** The best matches, highest first. */
export function search(query: string, paths: readonly string[], limit = 50): Match[] {
  const matches = paths.flatMap((path) => {
    const match = score(query, path);
    return match ? [match] : [];
  });

  matches.sort((a, b) => b.score - a.score || a.path.localeCompare(b.path));
  return matches.slice(0, limit);
}
