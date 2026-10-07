# Contributing to Lenscribe

Bug reports, documentation, UI improvements, and code contributions are welcome.

## Issues and feature requests

Search [existing issues](https://github.com/ssubedir/lenscribe/issues) before opening a new one. For bugs, include your Lenscribe version, OS, reproduction steps, expected behavior, and relevant sanitized [logs](#logs). Discuss substantial changes in an issue before implementation.

Use sample images you can share and leave private data out of reports. Report vulnerabilities through the [security policy](SECURITY.md).

## Set up the project

Install [Bun](https://bun.sh/docs/installation) using the version in `package.json`, [stable Rust](https://rust-lang.org/tools/install/), and your platform's [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). Clone the repository, or your fork when contributing:

```sh
git clone https://github.com/ssubedir/lenscribe.git
cd lenscribe
bun install --frozen-lockfile
bun run tauri dev
```

For UI work with sample data, run `bun run dev` and open `http://127.0.0.1:1420/?preview`. Build an executable without installer signing with `bun run tauri build --no-bundle`. Cargo uses `src-tauri/target/`; stop native builds before running `cargo clean --manifest-path src-tauri/Cargo.toml` to clear it.

## Make and check your changes

Keep pull requests focused and add meaningful tests for changed behavior. Preserve original image bytes, durable processing, and protection against stale results. Keep domain policies, application workflows, ports, and adapters separate. Storage and file-format changes need explicit compatibility handling.

After changing Rust DTOs, run `bun run types:generate`; do not edit `src/lib/generated/core.ts` by hand. Run the relevant checks from the repository root:

```sh
bun run check
bun run test
bun run format:check
bunx prettier --check "*.md" .github
bun run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --workspace --all-features --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings
```

Provider tests use local mock servers and need no real API keys. Documentation-only changes need formatting and link checks. Keep Markdown paragraphs on one source line.

## Submit a pull request

Open a pull request against `main` describing the problem, resulting behavior, and validation. Include sample-data screenshots for UI changes and note compatibility changes or checks you could not run.

CI runs manually: maintainers with write access can post `/ci` as a new PR comment after each update, or use **Actions → CI → Run workflow** for a branch.

## Logs

Desktop diagnostics are written to `lenscribe.log`. Remove private paths and data before sharing:

- Windows: `%LOCALAPPDATA%\com.ssubedir.lenscribe\logs\lenscribe.log`
- Linux: `$XDG_DATA_HOME/com.ssubedir.lenscribe/logs/lenscribe.log`, falling back to `~/.local/share/com.ssubedir.lenscribe/logs/lenscribe.log`
- macOS: `~/Library/Logs/com.ssubedir.lenscribe/lenscribe.log`

## Releases

Maintainers run **Actions → Release bump → Run workflow** with a version such as `0.1.5`, then **Actions → Release → Run workflow** with the matching tag, `v0.1.5`. The workflows commit the bump, create the tag, build packages, and publish after validation.

Set the repository secret `TAURI_SIGNING_PRIVATE_KEY` to the updater key matching the public key in `src-tauri/tauri.conf.json`, plus `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if password-protected. Keep the private key backed up outside Git; changing the trusted public key requires a migration plan for existing installations.

Contributions are provided under the project's [MIT license](LICENSE).
