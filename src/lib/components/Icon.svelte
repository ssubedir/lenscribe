<script lang="ts">
  let { name, size = 20 }: { name: string; size?: number } = $props();
  const paths: Record<string, string> = {
    overview: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
    folder: "M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z",
    spark: "m12 3 2.6 6.4L21 12l-6.4 2.6L12 21l-2.6-6.4L3 12l6.4-2.6L12 3z",
    terminal: "m5 7 5 5-5 5 M13 17h6",
    settings: "M4 7h16 M4 17h16 M8 4v6 M16 14v6",
    arrow: "M5 12h14 m-5-5 5 5-5 5",
    back: "M19 12H5 m5-5-5 5 5 5",
    plus: "M12 5v14 M5 12h14",
    check: "m5 12 4 4L19 6",
    warning: "m12 3 10 18H2L12 3z M12 9v5 M12 17h.01",
    copy: "M9 9h11v12H9z M15 9V3H3v12h6",
    trash: "M3 6h18 M9 6V3h6v3 M5 6l1 15h12l1-15 M10 10v7 M14 10v7",
    image: "M3 3h18v18H3z m0 14 5-5 5 5 4-4 4 4 M8 7h.01",
    clock: "M12 8v5l3 2 M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0",
    eye: "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0",
    close: "m6 6 12 12 M6 18 18 6",
    minimize: "M5 12h14",
    maximize: "M5 5h14v14H5z",
    restore: "M8 4h12v12 M4 8h12v12H4z",
    sun: "M12 2v2 M12 20v2 M2 12h2 M20 12h2 M5 5l1.5 1.5 M17.5 17.5 19 19 M5 19l1.5-1.5 M17.5 6.5 19 5 M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0",
    moon: "M21 12.7A9 9 0 1 1 11.3 3a7 7 0 0 0 9.7 9.7z",
    search: "M11 3a8 8 0 1 1 0 16 8 8 0 0 1 0-16 M17 17l4 4",
    retry: "M20 7v5h-5 M4 17v-5h5 M5 7a8 8 0 0 1 14-1l1 6 M4 12l1 6a8 8 0 0 0 14-1",
    download: "M12 3v12 m-5-5 5 5 5-5 M4 16v5h16v-5",
  };
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.65"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"><path d={paths[name] ?? paths.spark} /></svg
>
