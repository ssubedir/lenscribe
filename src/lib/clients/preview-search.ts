// Mirror the inspector's local search behavior for development fixtures.
import { searchWords as words, withinOneEdit as oneEdit } from "../search/matching";

export function previewSearchRank(
  path: string,
  text: string | null,
  query: string,
  fuzzy: boolean,
  exactWords = false,
): number | null {
  const literal = query.toLowerCase();
  if (path.toLowerCase().includes(literal)) return 0;
  if ((text ?? "").toLowerCase().includes(literal)) return 1;
  if (!fuzzy && !exactWords) return null;
  const terms = words(query);
  if (!terms.length) return null;
  const indexed = [...new Set(words(path + "\n" + (text ?? "")))];
  return terms.every((term) =>
    indexed.some(
      (word) =>
        (exactWords ? word === term : word.startsWith(term)) ||
        (fuzzy && Array.from(term).length >= 4 && oneEdit(Array.from(term), Array.from(word))),
    ),
  )
    ? 2
    : null;
}
