import { describe, expect, test } from "bun:test";
import { createPreviewClient } from "../src/lib/clients/preview";
import {
  providerOptions,
  providerPresets,
  selectProvider,
  switchConnection,
  rememberConnection,
} from "../src/lib/settings/providers";
import { validateSettings } from "../src/lib/settings/validation";

describe("provider settings", () => {
  test("switching provider clears connection credentials while preserving extraction options", async () => {
    const original = (await createPreviewClient().status()).settings.extraction;
    original.apiKey = "old-provider-secret";
    for (const provider of providerOptions.filter((entry) => entry.id !== original.provider)) {
      const selected = selectProvider(original, provider.id);
      expect(selected).toMatchObject({
        provider: provider.id,
        baseUrl: provider.baseUrl,
        model: "",
        apiKey: "",
        enabled: original.enabled,
        prompt: original.prompt,
        maxTokens: original.maxTokens,
        timeoutSeconds: original.timeoutSeconds,
      });
      expect(original.apiKey).toBe("old-provider-secret");
    }
    expect(selectProvider(original, original.provider)).toBe(original);
  });

  test("cloud providers require a key, while custom endpoints and Ollama can run without one", async () => {
    for (const provider of providerOptions) {
      const settings = (await createPreviewClient().status()).settings;
      settings.extraction = {
        ...selectProvider(settings.extraction, provider.id),
        model: "fixture-vision",
      };
      expect(validateSettings(settings)?.field ?? null).toBe(
        provider.requiresKey ? "llm-key" : null,
      );
      settings.extraction.apiKey = "fixture-key";
      expect(validateSettings(settings)).toBeNull();
      settings.extraction.enabled = false;
      settings.extraction.apiKey = "";
      expect(validateSettings(settings)).toBeNull();
    }
  });

  test("native completion URLs and unsafe Gemini model paths cannot be saved", async () => {
    const settings = (await createPreviewClient().status()).settings;
    settings.extraction.provider = "gemini";
    settings.extraction.apiKey = "fixture-key";
    settings.extraction.baseUrl = providerPresets.gemini.baseUrl;
    for (const model of [
      "../other",
      "model?key=secret",
      "models/",
      "models/..",
      "gemini::fixture",
    ]) {
      settings.extraction.model = model;
      expect(validateSettings(settings)?.field).toBe("llm-model");
    }
    settings.extraction.model = "models/fixture-vision";
    expect(validateSettings(settings)).toBeNull();
    for (const suffix of ["/messages", "/api/chat", "/models/fixture:generateContent"]) {
      settings.extraction.baseUrl = "https://example.com" + suffix;
      expect(validateSettings(settings)?.field).toBe("llm-url");
    }
  });
});

test("each provider restores its own connection after saving and loading", async () => {
  const client = createPreviewClient();
  let settings = (await client.status()).settings;
  switchConnection(settings, "custom");
  Object.assign(settings.extraction, {
    baseUrl: "http://localhost:12345/v1",
    model: "local-vision",
    apiKey: "custom-key",
  });
  switchConnection(settings, "anthropic");
  Object.assign(settings.extraction, { model: "claude-fixture", apiKey: "anthropic-key" });
  rememberConnection(settings);
  rememberConnection(settings);
  settings = (await client.save(settings)).settings;
  switchConnection(settings, "custom");
  expect(settings.extraction).toMatchObject({
    baseUrl: "http://localhost:12345/v1",
    model: "local-vision",
    apiKey: "custom-key",
  });
  switchConnection(settings, "anthropic");
  expect(settings.extraction).toMatchObject({
    baseUrl: providerPresets.anthropic.baseUrl,
    model: "claude-fixture",
    apiKey: "anthropic-key",
  });
  expect(settings.connections.filter((entry) => entry.provider === "anthropic")).toHaveLength(1);
});

test("an invalid abandoned connection draft does not prevent another provider from saving", async () => {
  const settings = (await createPreviewClient().status()).settings;
  switchConnection(settings, "custom");
  settings.extraction.model = "saved-model";
  rememberConnection(settings);
  for (const invalid of [
    { baseUrl: "unfinished" },
    { apiKey: "key\ninvalid" },
    { model: "invalid\nmodel" },
  ]) {
    Object.assign(settings.extraction, invalid);
    switchConnection(settings, "ollama");
    settings.extraction.model = "vision:latest";
    expect(validateSettings(settings)).toBeNull();
    switchConnection(settings, "custom");
    expect(settings.extraction.model).toBe("saved-model");
    expect(settings.extraction.baseUrl).toBe(providerPresets.custom.baseUrl);
  }
});

test("processing limits are validated and routed to advanced extraction settings", async () => {
  const settings = (await createPreviewClient().status()).settings;
  for (const concurrency of [0, 9, 1.5, NaN]) {
    settings.extraction.concurrency = concurrency;
    expect(validateSettings(settings)).toMatchObject({ field: "llm-concurrency", advanced: true });
  }
  settings.extraction.concurrency = 8;
  for (const requestsPerMinute of [-1, 601, 0.5, NaN]) {
    settings.extraction.requestsPerMinute = requestsPerMinute;
    expect(validateSettings(settings)).toMatchObject({ field: "llm-rate", advanced: true });
  }
  settings.extraction.requestsPerMinute = 600;
  expect(validateSettings(settings)).toBeNull();
});
