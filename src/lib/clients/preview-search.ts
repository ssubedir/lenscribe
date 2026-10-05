// Mirror the inspector's local search behavior for development fixtures.
function words(value: string): string[] {
  return (
    value
      .toLowerCase()
      .normalize("NFD")
      .replace(/\p{M}/gu, "")
      .match(/[\p{L}\p{N}]+/gu) ?? []
  );
}

function oneEdit(left: string[], right: string[]): boolean {
  if (Math.abs(left.length - right.length) > 1) return false;
  let index = 0;
  while (index < Math.min(left.length, right.length) && left[index] === right[index]) index++;
  if (index === Math.min(left.length, right.length)) return true;
  if (left.length < right.length)
    return left.slice(index).join("") === right.slice(index + 1).join("");
  if (left.length > right.length)
    return left.slice(index + 1).join("") === right.slice(index).join("");
  return (
    left.slice(index + 1).join("") === right.slice(index + 1).join("") ||
    (index + 1 < left.length &&
      left[index] === right[index + 1] &&
      left[index + 1] === right[index] &&
      left.slice(index + 2).join("") === right.slice(index + 2).join(""))
  );
}

export function previewSearchRank(
  path: string,
  text: string | null,
  query: string,
  fuzzy: boolean,
): number | null {
  const literal = query.toLowerCase();
  if (path.toLowerCase().includes(literal)) return 0;
  if ((text ?? "").toLowerCase().includes(literal)) return 1;
  if (!fuzzy) return null;
  const terms = words(query);
  if (!terms.length) return null;
  const indexed = [...new Set(words(path + "\n" + (text ?? "")))];
  return terms.every((term) =>
    indexed.some(
      (word) =>
        word.startsWith(term) ||
        (Array.from(term).length >= 4 && oneEdit(Array.from(term), Array.from(word))),
    ),
  )
    ? 2
    : null;
}
