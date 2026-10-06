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

The [CI workflow](../.github/workflows/ci.yml) runs manually. From a pull request's conversation page, post a new comment containing only `/ci`. Repository maintainers with write access can use this command; it tests the PR's current head commit, including PRs from forks, and reports a **CI / Manual PR** status on that commit with a link to the run. Each new `/ci` comment resolves the latest commit, and all jobs in that run test the same pinned commit. Post `/ci` again after pushing changes. The command becomes available after this workflow is merged into the default branch. Opening a PR or pushing commits does not start CI automatically.

For a repository branch, use **Actions → CI → Run workflow** and select the branch. CI checks generated types, frontend types, code and documentation formatting, tests, and the frontend build. After those pass, it tests and lints Rust and builds the native app on Windows x64, Linux x64, macOS Apple Silicon, and macOS Intel. Native CI builds use `--debug --no-bundle`, so they require no installer or updater signing keys. Newer jobs cancel older jobs for the same branch or PR. Build jobs have read-only repository access; only separate API jobs can report commit statuses, and PR runs do not save shared Rust caches.

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

Frontend tests use Bun and compile the real Svelte rune controller. They cover draft preservation, stale responses, validation, preview isolation, updater coordination, and release scripts. Rust tests cover trailers and decoding, Merkle identity, search, storage recovery, live watches, configuration, retries, cache reuse, concurrent extraction, and cancellation. Model tests use local mock HTTP servers; real provider credentials are not needed.

Use `bun run format` for Svelte, TypeScript, CSS, scripts, and workflow files, and `cargo fmt --manifest-path src-tauri/Cargo.toml --all` for Rust. Generated bindings are excluded from manual formatting.

## Headless core

The core can run without the desktop UI. From the repository root:

```sh
cargo run --manifest-path src-tauri/Cargo.toml -p lenscribe-core --bin lenscribe-core -- "/path/to/images" ".lenscribe/index.wedb" 47831
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

The database defaults to `index.wedb` beside the configuration. An optional final argument supplies a different database path. Omitted extraction settings default to disabled. Port zero requests an available port. The runner writes startup status to stdout and diagnostics/watch events to stderr. OS login registration belongs to the desktop app.

## Storage and backups

Canonical records live in the `index.wedb` directory. WeDB is the only storage backend, holding file and folder IDs, extracted text, cache entries, queued requests, retry state, and Merkle roots. Existing WeDB stores and logical backups retain their schema and remain compatible.

This version no longer imports SQLite databases or remaps `.sqlite` paths to WeDB directories. SQLite-only installations need an earlier release with migration support before upgrading; old database files are left untouched. Rescanning processed images recovers their embedded text, but cannot recover the old database's cache, queue, or retry state.

Use **General → Export Backup** for a consistent `.lenscribe-backup` file while Lenscribe runs. Do not copy a live WeDB directory as a backup. Images and settings are separate and are not included in the export. The current restore entry point is the Rust core API; there is no restore button yet:

```rust
let core = lenscribe_core::Core::restore_database("saved.lenscribe-backup", "restored.wedb")?;
```

Restoration validates the checksum, schema, text hashes, IDs, and references before reserving a new destination directory. Existing files and directories are rejected. Point the headless runner at the restored directory, or quit the desktop app and move the validated restored directory into the application's data location after preserving its current store. Only one process can open a store at a time.

## Logs and troubleshooting

The desktop stores `settings.json` and the `index.wedb` directory in Tauri's application data directory. Keep that configuration private because it contains saved API keys.

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

Open **Actions → Release bump → Run workflow** for the [Release bump workflow](../.github/workflows/release-bump.yml), select the branch to update, and enter a version such as `0.1.3` without the `v` prefix. The workflow must be merged into the default branch before the manual trigger appears.

The workflow updates `package.json`, `src-tauri/tauri.conf.json`, both Rust package manifests, and the two workspace lockfile entries. It verifies that every version matches the requested tag, then commits and pushes directly to the selected branch using the automatic `GITHUB_TOKEN`. No PR is opened. An unchanged version creates no commit. Branch protection still applies, and a concurrent branch update causes the push to fail rather than overwrite changes; rerun the workflow if needed.

To prepare the version locally instead:

```sh
bun run release:version x.x.x
bun run release:version --check vx.x.x
```

Commit and push local version changes. After either approach, start the **Release** workflow with the matching tag, such as `v0.1.3`, to build and publish the release.

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

### In-app updates

Installed release builds check `https://github.com/ssubedir/lenscribe/releases/latest/download/latest.json` 30 seconds after daemon startup, then every 24 hours while the process runs. These checks run in Rust and continue with the settings window hidden. Only stable versions are offered. Development builds make no update requests; the design preview simulates updates without network or desktop access.

An available update appears beside the sidebar version. Clicking it opens **General → App Updates**, where users can check manually, read release notes, and choose **Update & Restart**. Unsaved settings must be saved or discarded before installation. Navigation from the file inspector keeps its existing unsaved-text guard.

Downloads keep image processing running. The official plugin verifies the artifact signature and its signed version before shutdown. Installation then waits for daemon tasks and atomic writes, syncs the database, runs the installer off the UI thread, and restarts the app. Queue state is retained. Windows exits after launching its installer; macOS and AppImage installations restart after replacement. A download or signature failure leaves monitoring running. A shutdown or installer failure attempts to resume the daemon with its saved settings, including the user's paused preference, and reports when a manual restart is needed.

Checks and installations share one native operation lock. Update state and progress are published to the UI with revision numbers; stale responses cannot erase a newer notice. Polling also retrieves state if an event is missed. Linux `.deb` installations direct users to package updates instead of replacing the installed executable with an AppImage.

Before shipping updater changes, perform an installed-release smoke test on each supported platform:

1. Install an older signed release with the same updater public key and a compatible signed feed, then publish a newer signed release.
2. Hide the settings window, confirm the background check finds the release, then open the sidebar notice and its release notes.
3. Confirm unsaved settings and inspector edits are preserved. Queue images or reprocessing, install the update, and verify the new version, saved configuration, text, local API, and remaining queue after restart.
4. Interrupt a download and confirm monitoring continues and retry works. Automated tests separately cover tampering, signed-version mismatches, concurrent requests, shutdown failures, installer failures, and daemon recovery.
