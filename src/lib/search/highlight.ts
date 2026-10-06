import { normalizeSearchText, searchWords, withinOneEdit } from "./matching";

export interface TextSegment {
  text: string;
  matched: boolean;
}

interface Range {
  start: number;
  end: number;
}

function literalRanges(source: string, query: string): Range[] {
  const normalized = normalizeSearchText(source);
  const matches: Range[] = [];
  for (
    let start = normalized.indexOf(query);
    start !== -1;
    start = normalized.indexOf(query, start + query.length)
  ) {
    matches.push({ start, end: start + query.length });
  }
  if (!matches.length) return [];

  // Normalization changes offsets. Map matches back to whole original characters,
  // including their combining marks, without changing the displayed transcription.
  const ranges: Range[] = [];
  let offset = 0,
    index = 0,
    start = -1;
  for (const cluster of source.matchAll(/\P{M}\p{M}*|\p{M}+/gu)) {
    const end = offset + normalizeSearchText(cluster[0]).length;
    while (index < matches.length && matches[index].start < end) {
      if (start === -1) start = cluster.index;
      if (matches[index].end > end) break;
      ranges.push({ start, end: cluster.index + cluster[0].length });
      index++;
      start = -1;
    }
    offset = end;
    if (index === matches.length) break;
  }
  return ranges;
}

export function highlightText(source: string, query: string, fuzzy = true): TextSegment[] {
  query = query.trim();
  if (!query || !source) return [{ text: source, matched: false }];
  const literal = normalizeSearchText(query);
  const terms = [...new Set(searchWords(query))].map((text) => ({ text, chars: Array.from(text) }));
  fuzzy &&= terms.length <= 8 && terms.every((term) => term.chars.length <= 64);
  const ranges = literal ? literalRanges(source, literal) : [];
  for (const token of source.matchAll(/[\p{L}\p{N}][\p{L}\p{N}\p{M}]*/gu)) {
    const word = normalizeSearchText(token[0]);
    if (
      terms.some(
        (term) =>
          word === term.text ||
          (fuzzy &&
            (word.startsWith(term.text) ||
              (term.chars.length >= 4 && withinOneEdit(term.chars, Array.from(word))))),
      )
    ) {
      ranges.push({ start: token.index, end: token.index + token[0].length });
    }
  }
  ranges.sort((left, right) => left.start - right.start || left.end - right.end);
  const merged: Range[] = [];
  for (const range of ranges) {
    const previous = merged.at(-1);
    if (previous && range.start <= previous.end) previous.end = Math.max(previous.end, range.end);
    else merged.push({ ...range });
  }
  const parts: TextSegment[] = [];
  let offset = 0;
  for (const range of merged) {
    if (range.start > offset)
      parts.push({ text: source.slice(offset, range.start), matched: false });
    parts.push({ text: source.slice(range.start, range.end), matched: true });
    offset = range.end;
  }
  if (offset < source.length) parts.push({ text: source.slice(offset), matched: false });
  return parts;
}
