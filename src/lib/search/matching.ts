export function normalizeSearchText(value: string): string {
  return value
    .toLowerCase()
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .replace(/ς/g, "σ")
    .replace(/_/g, " ");
}

export function searchWords(value: string): string[] {
  return normalizeSearchText(value).match(/[\p{L}\p{N}]+/gu) ?? [];
}

export function withinOneEdit(left: string[], right: string[]): boolean {
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
