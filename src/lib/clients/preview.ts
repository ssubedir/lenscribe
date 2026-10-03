// Imported only by the explicit development preview. No desktop or network access.
import type { FileDetails, FileRecord } from "../generated/core";
import type { AppClient } from "./types";
import { applyPreviewSettings, createPreviewStatus } from "./fixtures";

const receipt = `<svg xmlns="http://www.w3.org/2000/svg" width="244" height="236" viewBox="0 0 244 236"><rect width="244" height="236" fill="#f8f6ee"/><g fill="#3d4439" font-family="Consolas,monospace" font-size="11"><text x="22" y="35" font-weight="bold">THE COFFEE SHOP</text><text x="22" y="58">October 2, 2026</text><path d="M22 75H222M22 144H222" stroke="#aab49f" stroke-dasharray="4 3"/><text x="22" y="99">Flat White</text><text x="184" y="99">$4.50</text><text x="22" y="121">Croissant</text><text x="184" y="121">$3.00</text><text x="22" y="168" font-weight="bold">Total</text><text x="184" y="168" font-weight="bold">$7.50</text><text x="122" y="206" text-anchor="middle">Thank you!</text></g></svg>`;

function sampleFiles(folderId: number): FileDetails[] {
  return [
    {
      id: folderId * 100 + 1,
      folderId,
      relativePath: "receipt-October.png",
      imageHash: "a".repeat(64),
      imageLength: 128000,
      textHash: "b".repeat(64),
      recordHash: "c".repeat(64),
      processor: "vision-model/preview",
      text: "THE COFFEE SHOP\nOctober 2, 2026\n\nFlat White        $4.50\nCroissant         $3.00\n\nTotal             $7.50\nThank you!",
    },
    {
      id: folderId * 100 + 2,
      folderId,
      relativePath: "notes/meeting.png",
      imageHash: "d".repeat(64),
      imageLength: 204800,
      textHash: "e".repeat(64),
      recordHash: "f".repeat(64),
      processor: "vision-model/preview",
      text: "Project notes\nShip the next release on Friday.",
    },
    {
      id: folderId * 100 + 3,
      folderId,
      relativePath: "new-image.png",
      imageHash: "1".repeat(64),
      imageLength: 98000,
      textHash: null,
      recordHash: "2".repeat(64),
      processor: null,
      text: null,
    },
  ];
}

export function createPreviewClient(): AppClient {
  let status = createPreviewStatus(),
    revision = 0;
  const folders = new Map<number, FileDetails[]>();
  const files = (folderId: number) => {
    if (!folders.has(folderId)) folders.set(folderId, sampleFiles(folderId));
    return folders.get(folderId)!;
  };
  const file = (fileId: number) => {
    const result = files(Math.floor(fileId / 100)).find((entry) => entry.id === fileId);
    if (!result) throw new Error("This preview image no longer exists.");
    return result;
  };
  const current = (expected: FileRecord) => {
    const result = file(expected.id);
    if (result.imageHash !== expected.imageHash) throw new Error("The image changed.");
    return result;
  };
  return {
    mode: "preview",
    pollInterval: 0,
    async status() {
      return structuredClone(status);
    },
    async save(settings) {
      status = applyPreviewSettings(status, settings);
      return structuredClone(status);
    },
    async retry() {
      return structuredClone(status);
    },
    async discoverModels() {
      return {
        models: [{ id: "preview-vision", name: "Preview Vision", vision: true }],
        truncated: false,
      };
    },
    async checkUpdate() {
      return { version: "0.2.0", notes: "Preview release. No update will be downloaded." };
    },
    async installUpdate(onProgress) {
      onProgress(100);
    },
    async chooseFolder() {
      return "";
    },
    async onError() {
      return () => {};
    },
    async listFiles(folderId, query, offset) {
      const matches = files(folderId).filter((entry) =>
        entry.relativePath.toLowerCase().includes(query.toLowerCase()),
      );
      return structuredClone({ files: matches.slice(offset, offset + 50), total: matches.length });
    },
    async fileDetails(fileId) {
      return structuredClone(file(fileId));
    },
    async filePreview(fileId) {
      file(fileId);
      return "data:image/svg+xml;charset=utf-8," + encodeURIComponent(receipt);
    },
    async editFile(expected, text) {
      const result = current(expected);
      if (result.recordHash !== expected.recordHash)
        throw new Error("The text changed before it could be saved.");
      Object.assign(result, { text, processor: "manual/v1", recordHash: `preview-${++revision}` });
      return {
        file: structuredClone(result),
        message: "Preview text updated. No files were changed.",
      };
    },
    async queueFile(expected) {
      current(expected);
      return "Preview request queued. No images were sent.";
    },
  };
}
