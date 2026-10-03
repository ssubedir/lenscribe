import type { Settings } from "$lib/generated/core";
import type { Page } from "./navigation";
import { connectionIssue } from "./providers";

export interface ValidationIssue {
  page: Page;
  field: string;
  message: string;
  advanced?: boolean;
}

export function validateSettings(settings: Settings): ValidationIssue | null {
  const issue = (page: Page, field: string, message: string, advanced = false) => ({
    page,
    field,
    message,
    advanced,
  });
  for (const [index, folder] of settings.folders.entries()) {
    if (!folder.path.trim())
      return issue(
        "folders",
        "folder-" + index,
        "Enter a path for each folder, or remove the empty row.",
      );
    if (
      !Number.isInteger(folder.maxImageMib) ||
      folder.maxImageMib < 0 ||
      folder.maxImageMib > 131072
    )
      return issue("folders", "size-" + index, "Enter a size limit from 0 to 131072 MiB.");
  }
  if (!Number.isInteger(settings.api.port) || settings.api.port < 0 || settings.api.port > 65535)
    return issue("api", "port", "Enter a port from 0 to 65535.");
  const extraction = settings.extraction;
  if (
    !Number.isInteger(extraction.concurrency) ||
    extraction.concurrency < 1 ||
    extraction.concurrency > 8
  )
    return issue("extraction", "llm-concurrency", "Enter 1 to 8 concurrent images.", true);
  if (
    !Number.isInteger(extraction.requestsPerMinute) ||
    extraction.requestsPerMinute < 0 ||
    extraction.requestsPerMinute > 600
  )
    return issue("extraction", "llm-rate", "Enter 0 to 600 requests per minute.", true);
  const connection = connectionIssue(extraction);
  if (connection) return issue("extraction", connection.field, connection.message);
  if (!extraction.prompt.trim())
    return issue("extraction", "llm-prompt", "Enter transcription instructions.", true);
  if (
    !Number.isInteger(extraction.maxTokens) ||
    extraction.maxTokens < 1 ||
    extraction.maxTokens > 32768
  )
    return issue("extraction", "llm-tokens", "Enter an output token limit from 1 to 32768.", true);
  if (
    !Number.isInteger(extraction.timeoutSeconds) ||
    extraction.timeoutSeconds < 1 ||
    extraction.timeoutSeconds > 600
  )
    return issue("extraction", "llm-timeout", "Enter a timeout from 1 to 600 seconds.", true);
  return null;
}
