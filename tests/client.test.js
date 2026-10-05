import { describe, expect, test } from "bun:test";
import { createPreviewClient } from "../src/lib/clients/preview";
import { validateSettings } from "../src/lib/settings/validation";

describe("preview client", () => {
  test("returned snapshots cannot modify saved settings", async () => {
    const client = createPreviewClient();
    const status = await client.status();
    status.settings.theme = "dark";
    expect((await client.status()).settings.theme).toBe("light");
    const saved = await client.save(status.settings);
    saved.settings.theme = "light";
    expect((await client.status()).settings.theme).toBe("dark");
    expect((await createPreviewClient().status()).settings.theme).toBe("light");
  });

  test("filters filenames and pages without changing stored text", async () => {
    const client = createPreviewClient();
    const result = await client.listFiles(1, "MEETING", 0);
    expect(result.total).toBe(1);
    expect(result.files[0].relativePath).toBe("notes/meeting.png");
    result.files[0].relativePath = "changed";
    expect((await client.listFiles(1, "meeting", 0)).total).toBe(1);
    expect((await client.listFiles(1, "", 50)).files).toHaveLength(0);
  });

  test("searches extracted text and supports optional word prefixes and typos", async () => {
    const client = createPreviewClient();
    expect((await client.listFiles(1, "  CROISSANT  ", 0)).files.map((file) => file.id)).toEqual([
      101,
    ]);
    expect((await client.listFiles(1, "cofee", 0)).total).toBe(0);
    for (const query of ["cofee", "cofffee", "xoffee", "cofefe", "reciept", "receipt coff"])
      expect((await client.listFiles(1, query, 0, true)).files.map((file) => file.id)).toEqual([
        101,
      ]);
    expect((await client.listFiles(1, "receipt Friday", 0, true)).total).toBe(0);
    expect((await client.listFiles(1, "unrelated", 0, true)).total).toBe(0);
    expect((await client.listFiles(1, "%", 0, true)).total).toBe(0);
    expect((await client.listFiles(1, "  ", 0, true)).total).toBe(3);
    expect((await client.listFiles(1, "new-imag", 0, true)).files[0].processor).toBeNull();
  });

  test("search results use current edited text and prefer exact filenames", async () => {
    const client = createPreviewClient();
    await client.editFile(await client.fileDetails(102), "Receipt with coffee on Friday");
    expect((await client.listFiles(1, "receipt", 0, true)).files.map((file) => file.id)).toEqual([
      101, 102,
    ]);
    await client.editFile(await client.fileDetails(101), "Café λογος");
    expect((await client.listFiles(1, "croissant", 0, true)).total).toBe(0);
    expect((await client.listFiles(1, "cafe", 0, true)).files.map((file) => file.id)).toEqual([
      101,
    ]);
    expect((await client.listFiles(1, "λογοσ", 0, true)).files.map((file) => file.id)).toEqual([
      101,
    ]);
    expect((await client.listFiles(1, "cat", 0, true)).total).toBe(0);
  });

  test("rejects stale edits and reprocessing preserves existing text", async () => {
    const client = createPreviewClient();
    const original = await client.fileDetails(101);
    const result = await client.editFile(original, "Corrected receipt");
    expect(result.file.processor).toBe("manual/v1");
    await expect(client.editFile(original, "Stale correction")).rejects.toThrow("text changed");
    await client.queueFile(result.file, true, true);
    expect((await client.fileDetails(101)).text).toBe("Corrected receipt");
    expect((await client.filePreview(101)).startsWith("data:image/svg+xml")).toBe(true);
  });
});

describe("settings validation", () => {
  test("identifies the correct screen and advanced field", async () => {
    const settings = (await createPreviewClient().status()).settings;
    settings.extraction.timeoutSeconds = 601;
    expect(validateSettings(settings)).toMatchObject({
      page: "extraction",
      field: "llm-timeout",
      advanced: true,
    });
    settings.extraction.timeoutSeconds = 120;
    settings.folders[1].path = "";
    expect(validateSettings(settings)).toMatchObject({ page: "folders", field: "folder-1" });
  });

  test("allows automatic API ports and rejects completion URLs", async () => {
    const settings = (await createPreviewClient().status()).settings;
    settings.api.port = 0;
    expect(validateSettings(settings)).toBeNull();
    settings.extraction.baseUrl = "http://localhost:1234/v1/chat/completions";
    expect(validateSettings(settings)?.field).toBe("llm-url");
  });
});
