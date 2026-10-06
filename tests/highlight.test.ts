import { describe, expect, test } from "bun:test";
import { render } from "svelte/server";
import SearchHighlight from "../src/lib/components/SearchHighlight.svelte";
import { highlightText } from "../src/lib/search/highlight";

const matched = (source: string, query: string, fuzzy = true) =>
  highlightText(source, query, fuzzy)
    .filter((part) => part.matched)
    .map((part) => part.text);

describe("extracted text highlighting", () => {
  test("highlights case-insensitive literals, punctuation, and all occurrences", () => {
    expect(matched("Coffee COFFEE", "off")).toEqual(["off", "OFF"]);
    expect(matched("Save 10% today. Another 10% tomorrow.", "10%")).toEqual(["10%", "10%"]);
    expect(matched("receipt+coffee", "+")).toEqual(["+"]);
  });

  test("uses fuzzy words and prefixes from the submitted search", () => {
    for (const query of ["cofee", "cofffee", "xoffee", "cofefe", "coff"])
      expect(matched("Coffee with a croissant", query)).toEqual(["Coffee"]);
    expect(matched("Receipt: Coffee and CROISSANT", "reciept coff croiss")).toEqual([
      "Receipt",
      "Coffee",
      "CROISSANT",
    ]);
    expect(matched("Coffee", "cxxfee")).toEqual([]);
    expect(matched("Café", "cat")).toEqual([]);
  });

  test("preserves Unicode, accents, combining marks, emoji, and whitespace", () => {
    const source = "🧾 Café\n  CAFE\u0301\tλόγος\r\nTotal €4.50";
    expect(matched(source, "cafe λογοσ")).toEqual(["Café", "CAFE\u0301", "λόγος"]);
    const parts = highlightText(source, "cafe");
    expect(parts.map((part) => part.text).join("")).toBe(source);
    expect(matched("İstanbul", "istanbul")).toEqual(["İstanbul"]);
    expect(matched("under_score", "under score")).toEqual(["under_score"]);
  });

  test("empty and unmatched searches keep the original text", () => {
    const source = "  Coffee\n<script>alert(1)</script>";
    for (const query of ["", "  ", "unrelated"])
      expect(highlightText(source, query)).toEqual([{ text: source, matched: false }]);
  });

  test("exact fallback suppresses typo and prefix matches", () => {
    expect(matched("Coffee", "cofee", false)).toEqual([]);
    expect(matched("Croissant", "croiss extra", false)).toEqual([]);
    expect(matched("Coffee receipt", "coffee extra", false)).toEqual(["Coffee"]);
    expect(
      matched(
        "one two three four five six seven eight nine Coffee",
        "one two three four five six seven eight nine cofee",
      ),
    ).not.toContain("Coffee");
  });

  test("the rendered component preserves text spacing and escapes extracted markup", () => {
    const source = "First\n  Coffee\tsecond\n";
    const body = render(SearchHighlight, { props: { text: source, query: "cofee" } }).body;
    expect(body).toContain(">Coffee</mark>");
    expect(body.replace(/<!--[\s\S]*?-->/g, "").replace(/<\/?mark\b[^>]*>/g, "")).toBe(source);
    const unsafe = render(SearchHighlight, {
      props: { text: '<img src=x onerror="alert(1)"> Coffee', query: "coffee" },
    }).body;
    expect(unsafe).not.toContain("<img");
    expect(unsafe).toContain("&lt;img");
  });
});
