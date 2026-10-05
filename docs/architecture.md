# Architecture

[Back to the README](../README.md)

Lenscribe is one desktop process with a Rust engine and a Svelte settings window. The engine owns folder watching, extraction, and the optional local HTTP API. Closing or destroying the settings page does not stop that work.

## Code layout

| Path | Responsibility |
| --- | --- |
| `src-tauri/crates/lenscribe-core/src/lib.rs` | Core operations, path validation, and serialized writes |
| `src-tauri/crates/lenscribe-core/src/trailer.rs` | Image detection, original bytes, and text trailer reads/writes |
| `src-tauri/crates/lenscribe-core/src/scan.rs` | Folder rules, scan state, and incremental reconciliation |
| `src-tauri/crates/lenscribe-core/src/merkle.rs` | Directory Merkle tree |
| `src-tauri/crates/lenscribe-core/src/database.rs` | SQLite connection ownership and scan transactions |
| `src-tauri/crates/lenscribe-core/src/database/` | Repositories for folders, files, search, extraction cache, jobs, and recovery |
| `src-tauri/crates/lenscribe-core/migrations/` | Numbered SQL schema upgrades |
| `src-tauri/crates/lenscribe-core/src/daemon.rs` | Settings and background lifecycle |
| `src-tauri/crates/lenscribe-core/src/extraction.rs` | Concurrent extraction, cancellation, pacing, and retries |
| `src-tauri/crates/lenscribe-core/src/llm.rs` | Vision requests through `genai` and completion validation |
| `src-tauri/crates/lenscribe-core/src/llm/connection.rs` | Bounded provider model discovery |
| `src-tauri/crates/lenscribe-core/src/http.rs` | Read-only loopback API |
| `src-tauri/src/commands.rs` | Tauri adapters; file work runs off the UI thread |
| `src/lib/core.ts` | Typed Tauri command and event wrappers |
| `src/lib/clients/` | Desktop, updater, and isolated preview adapters |
| `src/lib/settings/controller.svelte.ts` | Live status, editable drafts, validation, and Save/Discard |
| `src/lib/components/settings/` | Settings screens |
| `src/lib/components/FileInspector.svelte` | Image preview, text editing, and reprocessing |
| `src/lib/styles/` | Shared typography, controls, and layout |

Rust's serialized DTOs are the source of truth for `src/lib/generated/core.ts`, generated through [ts-rs](https://github.com/Aleph-Alpha/ts-rs). After changing a DTO, run `bun run types:generate`. `bun run check` verifies the contract before checking Svelte. Do not edit generated bindings manually.

Database migrations apply only missing versions and commit each schema change with its version. Opening a database from a newer schema fails without modifying it.

The database owns one SQLite connection behind the core's mutex. Repositories borrow that connection and keep SQL and row decoding within the persistence layer. Core operations choose the repository they need, such as `database.files().get(id)` or `database.jobs().list(folder_id)`. Scan reconciliation coordinates all repository writes in one transaction, so file records, FTS triggers, cached text, stale jobs and failures, and the Merkle root commit or roll back together. Failure records and endpoint recovery state also share a transaction. The public core API and schema remain independent of this internal layout.

## Image trailer format

PNG, JPEG, and WebP are identified by their file signatures. The scanner does not fully decode every image. Extensions are matched without case sensitivity. WebP detection checks the RIFF/WEBP header and that the declared container fits inside the original bytes.

V1 stores the following without recompressing the image or text:

```text
[all original file bytes]
LENSCRIBE-TEXT-V1
{"imageHash":"<SHA-256 hex>","processor":"<provider/model/settings version>"}
[raw UTF-8 text]
LENSCRIBE-END-V1 <image length> <payload length> <payload SHA-256 hex>
```

Both markers begin with a newline. The payload is exactly `JSON header + newline + text`. The footer ends with a newline. Lengths are 20-digit, zero-padded decimal byte counts; hashes are 64 lowercase hex characters.

The fixed-size ASCII footer locates the payload without scanning image data for markers. Reads validate lengths, the payload checksum, and the original image hash. Marker strings inside the transcription cannot change its boundaries.

Writes use a synchronized temporary file in the same directory and an atomic replacement. Identical writes do nothing. Only Lenscribe's own trailer is replaced; unmarked trailing data remains part of the original file. Expected image and record hashes prevent stale results or edits from replacing a newer image or manual correction.

Limits are 16 MiB of extracted text, 4096 bytes for the processor identifier, and 128 MiB of original bytes through the core preparation API. The desktop's vision client and previews apply the lower 20 MiB limit.

WebP's original RIFF size remains unchanged, including for lossy, lossless, transparent, and animated files. Text follows that container. [WebP readers may ignore trailing data](https://developers.google.com/speed/webp/docs/riff_container#webp_file_header); compatibility still depends on the reader. Tests decode processed PNG/WebP files and compare WebP pixels, alpha, and animation frames. JPEG tests verify the original bytes remain unchanged.

## Merkle tracking and watching

Image identity is SHA-256 of every byte before the Lenscribe trailer. A file record commits to image hash, extracted-text hash, and processor identity. A pending file differs from a successfully processed file with empty text.

Directory nodes hash sorted `(child name, node kind, child hash)` entries with UTF-8 byte ordering, length-prefixed fields, and distinct versioned prefixes. Absolute paths, timestamps, empty directories, the database, and the stored root are excluded. Equal relative image trees have equal roots in different locations. Renames change roots while preserving image identity.

The scanner keeps file metadata and the tree in memory. Watch events reconcile affected files or directory subtrees and update their ancestors. Text writes update one image's index entry. Events coalesce after 500 ms of quiet, with a 2-second maximum delay and at most 1024 retained paths. Overflow, unknown event paths, and watcher rescan notices request a full scan.

A 30-second metadata reconciliation catches changes in names, sizes, and modification times without rereading unchanged images. Startup and explicit scans verify hashes. A missed write that preserves both size and modification time can escape metadata reconciliation until a full scan. Unchanged scans do not rewrite indexed records.

Folder Rules apply to scanning, extraction preparation, and commits. Exclusion patterns use `/`, match without case sensitivity on Windows, and cannot escape the watched folder. Bare filenames match at every depth. Excluded files leave the index and queue without modifying their image or text; including them again imports existing trailers.

Invalid files are reported as scan issues and excluded from the index. Directory traversal failures abort a scan instead of silently pruning an inaccessible subtree. Symbolic links are not followed.

## Extraction and recovery

The background controller polls pending records every half second. Concurrency defaults to one image and is bounded to 1–8. Requests per minute are bounded to 0–600, where zero is unrestricted. Request starts are evenly paced across workers; cache hits do not consume requests.

The persistent cache key combines the original image SHA-256 with a processor identifier containing the provider, model, endpoint, prompt, token limit, and extraction-format version. API keys and runtime pacing limits are excluded from this identity. Identical pending images share one in-flight request. Successful empty text is reusable. Manually edited text has a separate `manual/v1` identity. Forced reprocessing bypasses the cache.

Only the original image bytes are sent to the vision model. The default prompt requests transcription in the original language and treats visible instructions as text. Valid empty text counts as processed. Truncated, filtered, refused, missing, or malformed responses leave the file unchanged and pending.

Rate limits, timeouts, and server errors retry with exponential backoff, up to five attempts per image, honoring numeric `Retry-After` headers. Attempt counts, sanitized errors, retry times, endpoint backoff, and pacing persist in SQLite. Authentication or endpoint errors block requests until configuration changes or an explicit retry. Changing the API key creates a new recovery state without invalidating cached extraction results.

Global **Retry extraction** resets failures and backoff while retaining pacing. Retrying a single file leaves other failures alone. **Reprocess** records a durable forced job; existing text stays readable until a fresh response succeeds. Older in-flight results cannot overwrite an updated image or manual edit.

Pausing stops watches and extraction, cancels in-flight requests, and keeps the backlog. Resuming scans for changes. Changing extraction settings or removing or disabling a folder cancels the previous worker before applying responses.

### Provider routing and discovery

Routing is explicit: a model name cannot select a different service.

| Provider value | Preset base URL | Request endpoint |
| --- | --- | --- |
| `openai` | `https://api.openai.com/v1` | `/chat/completions` |
| `anthropic` | `https://api.anthropic.com/v1` | `/messages` |
| `gemini` | `https://generativelanguage.googleapis.com/v1beta` | `/models/<model>:generateContent` |
| `openRouter` | `https://openrouter.ai/api/v1` | `/chat/completions` |
| `ollama` | `http://localhost:11434` | `/api/chat` |
| `groq` | `https://api.groq.com/openai/v1` | `/chat/completions` |
| `xai` | `https://api.x.ai/v1` | `/chat/completions` |
| `custom` | `http://localhost:1234/v1` | `/chat/completions` |

Anthropic, Gemini, and Ollama use native image protocols; the remaining providers use Chat Completions. Gemini accepts model IDs with or without `models/`. Hosted presets are applied by the settings UI; hand-written JSON must also supply the matching `baseUrl`.

Fetch Models requests provider catalogs or installed Ollama models, excludes models explicitly marked as text-only, and keeps entries with unknown image support marked as unverified. Catalogs are bounded to 500 entries and 10 pages. Manual model IDs remain available if discovery is unavailable.

## Search and frontend boundaries

SQLite FTS5 searches filenames and extracted text. Query terms are escaped as literals and combined with AND; results contain ranked snippets and are limited to 100. The HTTP layer runs blocking database operations off the async request thread. Its plain-text endpoint returns 404 when text is missing.

The file inspector searches literal substrings in filenames and extracted text within the selected folder, with 50 results per page. Fuzzy matching runs automatically, adding indexed word prefixes and up to one insertion, deletion, substitution, or adjacent swap for words of four or more characters. All query words must match the same file; exact filename matches precede exact text matches, followed by fuzzy matches. A temporary FTS5 vocabulary view follows the existing index, so edits, scans, and removals immediately affect searches without another persistent index or model requests. Fuzzy queries support up to eight words of 64 characters each, scan at most 50,000 candidate dictionary words, and expand each word to at most 32 alternatives. Oversized queries report an error so users can narrow the search. Semantic search and embeddings are not included.

The settings controller keeps live status separate from the editable draft. Polling cannot erase unsaved changes. Save applies configuration in Rust; Discard restores saved settings. Aggregate progress uses a grouped database query, including successful empty transcriptions.

The preview adapter is loaded only in explicit development preview mode. Its files, edits, images, and settings stay in memory. Native dialogs, command invocations, and model requests belong to the desktop adapter.

Tauri watch events use `lenscribe://watch` with typed updated reports or failures. Command wrappers are in `src/lib/core.ts`; unsubscribe from events when a component is destroyed. The daemon owns its lifecycle independently of that page.
