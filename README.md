<p align="center">
  <img src="static/lenscribe-logo-v2.png" width="80" height="80" alt="Lenscribe logo" />
</p>

<h1 align="center">Lenscribe</h1>

<p align="center">
  <strong>Make your images searchable with the tools you already use.</strong>
</p>

<p align="center">Lenscribe watches your folders, uses a vision model to extract text from your images, and saves it <strong>inside each image file</strong>. The text stays with the image, so you can search and read it with familiar tools like <code>grep</code>, <code>cat</code>, and PowerShell's <code>Get-Content</code>, or access it through a local API with <code>curl</code>.</p>

<p align="center">
  <a href="https://github.com/ssubedir/lenscribe/releases/latest">
    <img src="https://img.shields.io/github/v/release/ssubedir/lenscribe?label=version&amp;color=2e7564" alt="Latest release version" />
  </a>
  <a href="https://github.com/ssubedir/lenscribe/actions/workflows/ci.yml">
    <img src="https://github.com/ssubedir/lenscribe/actions/workflows/ci.yml/badge.svg" alt="CI checks and native builds" />
  </a>
  <a href="#install">
    <img src="https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-2e7564" alt="Platforms: Windows, macOS, and Linux" />
  </a>
</p>

<p align="center"><a href="https://github.com/ssubedir/lenscribe/releases">Download</a> · <a href="#getting-started">Getting started</a> · <a href="#search-your-images">Search your images</a> · <a href="CONTRIBUTING.md">Contributing</a> · <a href="https://github.com/ssubedir/lenscribe/issues">Report an issue</a></p>

<p align="center">
  <img src="docs/screenshots/overview.png" width="1000" alt="Lenscribe Overview with image counts, processing status, and watched folders" />
</p>

---

## What it does

- Watches folders and subfolders for **PNG, JPEG, and WebP** images.
- Extracts text with hosted or local vision models and keeps it inside the image.
- Searches filenames and extracted text with fuzzy matching; lets you preview, edit, and reprocess images.
- Reuses matching extraction results and resumes pending work after a restart.
- Includes a system tray, start at login, light/dark themes, and signed in-app updates.

## Install

Download a package for your platform from [GitHub Releases](https://github.com/ssubedir/lenscribe/releases).

| Platform            | Packages                |
| ------------------- | ----------------------- |
| Windows x64         | Setup `.exe` and `.msi` |
| Linux x64           | `.deb` and `.AppImage`  |
| macOS Apple Silicon | `.dmg`                  |
| macOS Intel         | `.dmg`                  |

Windows installers are currently unsigned. macOS builds use ad-hoc signing and are not notarized.

Use **General → App Updates** to check for updates or update and restart. Linux in-app updates support AppImage installations; update `.deb` packages through your package manager.

## Getting started

1. Open **Watched Folders**, choose a folder, and save. Use **Folder Rules** to exclude paths or set a size limit.
2. In **AI Extraction**, choose a provider, select an image-capable model with **Fetch Models** or enter its model ID, and add an API key if required.
3. Enable **Automatic text extraction** and save. Lenscribe processes pending images and watches for new arrivals.
4. Check progress in **Overview**, or open **Watched Folders → Inspect Files** to search, preview, edit, and reprocess images.

Automatic extraction starts disabled. Scanning alone indexes files and existing text without sending images to a model. Extraction and previews are limited to **20 MiB per image**. Review extracted text when accuracy matters.

Use **Reprocess** for a fresh extraction after changing models; existing processed images keep their text until you request it.

### Supported providers

**OpenAI, Anthropic, Google Gemini, OpenRouter, Groq, xAI, Ollama**, and **Custom / OpenAI Compatible** endpoints are supported. Choose a model that accepts images.

Hosted providers use preset URLs. **Base URL** is editable only for Ollama and compatible endpoints; enter the server's base URL without the chat or generation endpoint.

## Search your images

### Use your files directly

The text is readable without Lenscribe running. Search an image on Linux, macOS, or Git Bash:

```sh
grep -a "coffee" "/path/to/image.webp"
```

The `-a` option lets `grep` read the image as text. Read the file with `cat`:

```sh
cat "/path/to/image.webp"
```

Or read it in PowerShell:

```powershell
Get-Content "C:/path/to/image.webp" -Encoding utf8
```

These commands also read binary image data and trailer markers. Use the local API for **only the extracted text**. **Search & Read** includes copyable examples.

### Use the local API

Enable **Search & Read → Local search API** and save. The default address is `http://127.0.0.1:47831`.

```sh
# Search filenames and extracted text.
curl -G --data-urlencode "q=coffee" "http://127.0.0.1:47831/search"

# Read the text for a file ID returned by search.
curl "http://127.0.0.1:47831/files/1/text"
```

In Windows PowerShell, use `curl.exe` to call the curl executable explicitly.

The API is read-only, unauthenticated, and restricted to loopback. Other processes on your computer can query it while enabled.

## How the text stays with the image

Lenscribe uses **end-of-file (EOF) steganography**: it appends readable, unencrypted UTF-8 text after the original image data without re-encoding it. Processing the same file again replaces Lenscribe's own text trailer.

Compatibility depends on readers accepting trailing data. Re-saving or re-encoding an image in another application may discard its extracted text.

## Data and privacy

- Images are sent to your selected provider **only when extraction is enabled**. A local Ollama or compatible server can keep processing on your machine.
- API keys are stored **as plain text** in local `settings.json` in the application's data directory.
- The local database holds filenames, extracted text, cached results, and processing state. Copying a processed image also copies its embedded text.
- Logs can include file paths, but exclude API keys, prompts, image payloads, extracted text, and raw model request/response bodies.

**General → Index & Cache** offers backups, index rebuilding, and cache cleanup. Backups exclude images and settings; restoration currently requires the Rust core API.

## Community

Built with **Tauri 2, Svelte/TypeScript, and Rust**. See [Contributing](CONTRIBUTING.md) for setup, checks, and [log locations](CONTRIBUTING.md#logs).

[Report an issue](https://github.com/ssubedir/lenscribe/issues) · [Security Policy](SECURITY.md) · [MIT License](LICENSE)
