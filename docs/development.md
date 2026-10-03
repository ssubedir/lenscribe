# Development and releases

[Back to the README](../README.md) · [Architecture](architecture.md)

## Prerequisites

Use the Bun version declared in `package.json` and stable Rust.

- [Install Bun](https://bun.sh/docs/installation).
- [Install Rust with rustup](https://rust-lang.org/tools/install/).
- Follow [Tauri's platform prerequisites](https://v2.tauri.app/start/prerequisites/). Windows needs the MSVC build tools and WebView2; macOS needs Xcode command-line tools; Linux needs the WebKitGTK and other native development packages listed there.

## Run and build

From the repository root:

```sh
bun install --frozen-lockfile
bun run tauri dev
```

This starts Vite and the native desktop app. Restart the desktop process after changing Rust code, native plugins, capabilities, or window configuration.

For frontend work:

```sh
bun run dev
```

Open `http://127.0.0.1:1420/?preview` for sample folders, extraction status, and file-inspector data. The preview never contacts the daemon, provider, or desktop configuration and is disabled in production builds.

Build the frontend with `bun run build`. To build the release executable without packaging installers or signing updater artifacts:

```sh
bun run tauri build --no-bundle
```

Installer builds use `bun run tauri build` and require the matching updater signing key described in [Releases](#releases), because the project enables signed updater artifacts. Installer outputs are under `src-tauri/target/release/bundle/`.

## Checks

The [CI workflow](../.github/workflows/ci.yml) runs only when started manually from **Actions → CI → Run workflow**. It checks generated types, frontend types, code and documentation formatting, tests, and the frontend build. After those pass, it tests and lints Rust and builds the native app on Windows x64, Linux x64, macOS Apple Silicon, and macOS Intel. Native CI builds use `--debug --no-bundle`, so they require no installer or updater signing keys. Newer runs cancel older runs for the same branch.

Run from the repository root:

```sh
bun run check
bun run test
bun run format:check
bunx prettier --check "*.md" docs
bun run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --workspace --all-features --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings
```

`bun run check` verifies generated Rust/TypeScript bindings and Svelte types. Run `bun run types:generate` after changing serialized Rust DTOs.

Frontend tests use Bun and compile the real Svelte rune controller. They cover draft preservation, stale responses, validation, preview isolation, updater coordination, and release scripts. Rust tests cover trailers and decoding, Merkle identity, search, migrations, live watches, configuration, retries, cache reuse, concurrent extraction, and cancellation. Model tests use local mock HTTP servers; real provider credentials are not needed.

Use `bun run format` for Svelte, TypeScript, CSS, scripts, and workflow files, and `cargo fmt --manifest-path src-tauri/Cargo.toml --all` for Rust. Generated bindings are excluded from manual formatting.

## Headless core

The core can run without the desktop UI. From the repository root:

```sh
cargo run --manifest-path src-tauri/Cargo.toml -p lenscribe-core --bin lenscribe-core -- "/path/to/images" ".lenscribe/index.sqlite" 47831
```

This watches a folder, imports existing text trailers, and serves the local API. It does not enable AI extraction. Stop it with Ctrl+C.

To use the full background controller, create a configuration such as `.lenscribe/settings.json`:

```json
{
  "version": 1,
  "folders": [
    {
      "path": "C:/path/to/images",
      "enabled": true,
      "exclusions": ["temp/**"],
      "maxImageMib": 0
    }
  ],
  "api": { "enabled": true, "port": 47831 },
  "extraction": {
    "enabled": true,
    "provider": "custom",
    "baseUrl": "http://localhost:1234/v1",
    "model": "your-installed-vision-model",
    "apiKey": "",
    "maxTokens": 8192,
    "timeoutSeconds": 120,
    "concurrency": 1,
    "requestsPerMinute": 0
  }
}
```

Replace the folder path and model ID; use an absolute folder path for your OS. The custom example assumes an unauthenticated local compatible server. For another provider, use its value and matching base URL from the [provider table](architecture.md#provider-routing-and-discovery), plus its API key.

```sh
cargo run --manifest-path src-tauri/Cargo.toml -p lenscribe-core --bin lenscribe-core -- --config ".lenscribe/settings.json"
```

The database defaults to `index.sqlite` beside the configuration. An optional final argument supplies a different database path. Omitted extraction settings default to disabled. Port zero requests an available port. The runner writes startup status to stdout and diagnostics/watch events to stderr. OS login registration belongs to the desktop app.

## Logs and troubleshooting

The desktop stores `settings.json` and `index.sqlite` in Tauri's application data directory. Keep that configuration private because it contains saved API keys.

The desktop writes `lenscribe.log` while the settings window is open or closed. It appends timestamped UTC events for startup, watching, extraction, retries, and API activity. Only application diagnostics are recorded; API keys, prompts, image payloads, extracted text, and raw SDK request/response bodies are excluded. Diagnostic file paths can appear.

| Platform | Log location |
| --- | --- |
| Windows | `%LOCALAPPDATA%\com.ssubedir.lenscribe\logs\lenscribe.log` |
| Linux | `$XDG_DATA_HOME/com.ssubedir.lenscribe/logs/lenscribe.log`, or `~/.local/share/com.ssubedir.lenscribe/logs/lenscribe.log` |
| macOS | `~/Library/Logs/com.ssubedir.lenscribe/lenscribe.log` |

The file rolls over at 5 MiB and keeps one current log. Headless diagnostics go to stderr instead of this desktop log.

| Symptom | Check |
| --- | --- |
| Images stay pending | Automatic extraction is enabled and saved; monitoring is not paused; the model accepts images. |
| Authentication errors | The selected provider's saved API key and model ID. Save a corrected key or use Retry extraction. |
| Missing images | Supported extension, folder exclusions, size rules, path access, and scan issues. Symbolic links are skipped. |
| API cannot start | Another process may use the port. Choose a different port or use `0`. |
| A folder is unavailable | Reconnect it or fix permissions. The daemon retries unavailable folders every 30 seconds. |
| Processed text does not change after switching models | Existing successes are retained. Use Inspect Files → Reprocess for a fresh request. |
| Closing the window does not exit | Use Quit Lenscribe in the system tray. |

Start at login registers the current executable. Enable it from the installed app when using Lenscribe outside development; start-at-login launches hide the window. The separate start-hidden preference controls manual launches.

## Releases

The [Release workflow](../.github/workflows/release.yml) builds and validates Windows x64, Linux x64, macOS Apple Silicon, and macOS Intel packages. Linux uses an Ubuntu 22.04 build baseline. Versions come from the committed manifests; Bun and Cargo use their committed lockfiles.

### Prepare a version

For example, to prepare version `0.2.0`:

```sh
bun run release:version 0.2.0
bun run release:version --check v0.2.0
```

The script updates `package.json`, `src-tauri/tauri.conf.json`, both Rust package manifests, and the two workspace lockfile entries. It preserves dependency versions and surrounding formatting. Commit and push those changes.

### Configure signing

The workflow requires the repository Actions secret **TAURI_SIGNING_PRIVATE_KEY**, containing the updater private key that matches the public key in `src-tauri/tauri.conf.json`. If password-protected, also set **TAURI_SIGNING_PRIVATE_KEY_PASSWORD**.

The initial local key may be in `.release-signing/updater.key`. This directory is ignored and is not included when cloning the repository. A release maintainer must have the matching key and keep a private backup. Replacing the trusted public key prevents existing installations trusting updates signed with the replacement key; key rotation needs a migration plan.

Updater signatures are separate from OS installer signing. Windows builds are currently unsigned. macOS builds use ad-hoc signing and are not notarized. For OS signing, follow [Tauri's Windows signing guide](https://v2.tauri.app/distribute/sign/windows/) and [macOS signing guide](https://v2.tauri.app/distribute/sign/macos/).

### Run the workflow

1. Ensure the workflow is on the default branch and the updater secret is configured.
2. Open **Actions → Release → Run workflow** on GitHub.
3. Select the source branch and enter the matching tag, such as `v0.2.0`.

For a new tag, the workflow validates the version and signing configuration, creates the tag on GitHub, prepares a draft release, builds all four targets, and publishes only after every build and artifact validation succeeds. There is no need to create or push a tag locally. Manually pushed version tags are also supported.

A failed run leaves a draft. Use **Re-run failed jobs**, or dispatch the same tag again. Retries use the existing tag's source commit even if the selected branch has changed. Tags are never moved, and published releases cannot be rebuilt. A version such as `0.2.0-beta.1` produces a prerelease.

The workflow uses the automatic `GITHUB_TOKEN`; a separate GitHub token is not needed. Signed updater artifacts and a four-platform `latest.json` are built alongside installers. Publication validates their version, signatures, and download destinations. Missing or invalid updater artifacts keep the draft unpublished.

The **App Updates** card is currently hidden. The updater code is retained, but users install updates through release installers. The configured future feed is `https://github.com/ssubedir/lenscribe/releases/latest/download/latest.json`; GitHub's latest-release feed follows stable releases.
