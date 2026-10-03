import type {
  ExtractionSettings,
  LlmProvider,
  SavedConnection,
  Settings,
} from "$lib/generated/core";

interface ProviderPreset {
  label: string;
  baseUrl: string;
  description: string;
  urlHint: string;
  requiresKey: boolean;
  editableBaseUrl: boolean;
}

export const providerPresets: Record<LlmProvider, ProviderPreset> = {
  openai: {
    label: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    description: "Connect directly to an OpenAI vision model.",
    urlHint: "Use the API base URL, without /chat/completions.",
    requiresKey: true,
    editableBaseUrl: false,
  },
  anthropic: {
    label: "Anthropic",
    baseUrl: "https://api.anthropic.com/v1",
    description: "Use Claude through Anthropic’s native Messages API.",
    urlHint: "Use the API base URL, without /messages.",
    requiresKey: true,
    editableBaseUrl: false,
  },
  gemini: {
    label: "Google Gemini",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta",
    description: "Use a Gemini vision model with your Google AI Studio API key.",
    urlHint: "Use the API base URL, without /models or :generateContent.",
    requiresKey: true,
    editableBaseUrl: false,
  },
  openRouter: {
    label: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    description: "Choose an image-capable model from OpenRouter’s catalog.",
    urlHint: "Use the API base URL, without /chat/completions.",
    requiresKey: true,
    editableBaseUrl: false,
  },
  ollama: {
    label: "Ollama",
    baseUrl: "http://localhost:11434",
    description: "Run an installed vision model locally or on your own Ollama server.",
    urlHint: "Use the server URL, without /api/chat or /v1.",
    requiresKey: false,
    editableBaseUrl: true,
  },
  groq: {
    label: "Groq",
    baseUrl: "https://api.groq.com/openai/v1",
    description: "Use an image-capable model hosted by Groq.",
    urlHint: "Use the API base URL, without /chat/completions.",
    requiresKey: true,
    editableBaseUrl: false,
  },
  xai: {
    label: "xAI",
    baseUrl: "https://api.x.ai/v1",
    description: "Connect to an image-capable Grok model.",
    urlHint: "Use the API base URL, without /chat/completions.",
    requiresKey: true,
    editableBaseUrl: false,
  },
  custom: {
    label: "Custom / OpenAI Compatible",
    baseUrl: "http://localhost:1234/v1",
    description: "Connect to any OpenAI-compatible vision endpoint, including LM Studio.",
    urlHint: "Usually ends in /v1. Use the base URL, without /chat/completions.",
    requiresKey: false,
    editableBaseUrl: true,
  },
};

export const providerOptions = Object.entries(providerPresets).map(([id, preset]) => ({
  id: id as LlmProvider,
  ...preset,
}));

export function selectProvider(
  settings: ExtractionSettings,
  provider: LlmProvider,
): ExtractionSettings {
  if (settings.provider === provider) return settings;
  return {
    ...settings,
    provider,
    baseUrl: providerPresets[provider].baseUrl,
    model: "",
    apiKey: "",
  };
}

export function rememberConnection(settings: Settings) {
  // An unfinished draft must not overwrite a valid saved connection or block another provider.
  if (connectionIssue({ ...settings.extraction, enabled: false })) return;
  const { provider, baseUrl, model, apiKey } = settings.extraction;
  const connection: SavedConnection = { provider, baseUrl, model, apiKey };
  settings.connections = settings.connections.filter((entry) => entry.provider !== provider);
  settings.connections.push(connection);
  settings.connections.sort((a, b) => a.provider.localeCompare(b.provider));
}

export function connectionIssue(
  extraction: ExtractionSettings,
): { field: string; message: string } | null {
  if (!Object.hasOwn(providerPresets, extraction.provider))
    return { field: "llm-provider", message: "Choose a supported LLM provider." };
  if (
    (extraction.enabled && !extraction.model.trim()) ||
    new TextEncoder().encode(extraction.model).length > 256 ||
    /[\x00-\x1f\x7f-\x9f]/.test(extraction.model)
  )
    return { field: "llm-model", message: "Enter a vision model ID of at most 256 bytes." };
  if (
    extraction.provider === "gemini" &&
    extraction.model.trim() &&
    (!/^(?:models\/)?[A-Za-z0-9_.-]+$/.test(extraction.model.trim()) ||
      [".", ".."].includes(extraction.model.trim().replace(/^models\//, "")))
  )
    return {
      field: "llm-model",
      message: "Enter a valid Gemini model ID, with an optional models/ prefix.",
    };
  try {
    const url = new URL(extraction.baseUrl.trim());
    if (
      !["http:", "https:"].includes(url.protocol) ||
      url.username ||
      url.password ||
      url.href.includes("?") ||
      url.href.includes("#") ||
      [
        "/chat/completions",
        "/messages",
        "/api/chat",
        ":generateContent",
        ":streamGenerateContent",
      ].some((suffix) => url.pathname.replace(/\/+$/, "").endsWith(suffix))
    )
      throw new Error();
  } catch {
    return {
      field: "llm-url",
      message:
        "Enter an HTTP or HTTPS base URL, without credentials or a chat/generation endpoint.",
    };
  }
  if (
    new TextEncoder().encode(extraction.apiKey).length > 4096 ||
    /[\x00-\x08\x0a-\x1f\x7f]/.test(extraction.apiKey.trim())
  )
    return {
      field: "llm-key",
      message: "Enter an API key of at most 4096 bytes, without invalid header characters.",
    };
  if (
    extraction.enabled &&
    providerPresets[extraction.provider].requiresKey &&
    !extraction.apiKey.trim()
  )
    return { field: "llm-key", message: "Enter an API key for the selected provider." };
  return null;
}

export function switchConnection(settings: Settings, provider: LlmProvider) {
  if (settings.extraction.provider === provider) return;
  rememberConnection(settings);
  const saved = settings.connections.find((entry) => entry.provider === provider);
  settings.extraction = { ...selectProvider(settings.extraction, provider), ...saved };
  if (!providerPresets[provider].editableBaseUrl)
    settings.extraction.baseUrl = providerPresets[provider].baseUrl;
}
