# Contributing to Lenscribe

Bug reports, documentation, UI improvements, and code contributions are welcome. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Issues and feature requests

Search [existing issues](https://github.com/ssubedir/lenscribe/issues) before opening a new one, then choose the bug report or feature request template. For bugs, include your Lenscribe version, OS, steps to reproduce, expected behavior, and relevant log lines. The [development guide](docs/development.md#logs-and-troubleshooting) explains where to find logs.

Use synthetic images or examples you have permission to share. Remove API keys, private paths, image contents, and extracted text from public reports when they contain sensitive information. Report security vulnerabilities privately using the [security policy](SECURITY.md).

For substantial changes, start with an issue explaining the problem and proposed approach so maintainers can discuss the scope before implementation.

## Set up the project

Fork the repository, clone your fork, and create a branch for your change. Install the Bun version declared in `package.json`, stable Rust, and your platform's [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

From the repository root:

```sh
bun install --frozen-lockfile
bun run tauri dev
```

For frontend work with sample data, use `bun run dev` and open `http://127.0.0.1:1420/?preview`. See the [development guide](docs/development.md) for setup, builds, and the headless runner, and [architecture](docs/architecture.md) for the code layout and processing behavior.

## Make and check your changes

Keep each pull request focused on one problem. Match the existing Rust, Svelte, and TypeScript conventions, and add regression coverage for fixes or tests for changed behavior. Update documentation when setup, user behavior, or APIs change.

Preserve original image bytes, transaction boundaries, and protection against stale extraction results. Put business values and policies in `domain`, workflows in `application`, I/O contracts in `ports`, and external integrations in `adapters`. Keep concrete wiring in `composition` or `runtime`, and provider requests and credentials inside adapters. WeDB schema changes need explicit versioning and upgrade handling; the numbered SQL migrations are legacy import test fixtures and must retain their shipped definitions.

Rust DTOs define the frontend contract. After changing serialized types, run `bun run types:generate` and include the generated changes; do not edit `src/lib/generated/core.ts` by hand.

Run the relevant [development checks](docs/development.md#checks) before submitting. Provider tests use local mock servers and do not require real API keys. Documentation-only changes need formatting and link checks; they do not need the app's runtime test suites.

To check documentation formatting:

```sh
bunx prettier --check "*.md" docs .github
```

Use `bun run format` for frontend code and workflow files, and `cargo fmt --manifest-path src-tauri/Cargo.toml --all` for Rust. Keep Markdown paragraphs on one source line, following the existing `proseWrap: never` setting.

## Submit a pull request

Open a pull request against `main`. Explain the problem, resulting behavior, and validation performed. Link the relevant issue and include screenshots for visible UI changes, using sample data. Note compatibility changes or database migrations, and say which checks you could not run.

CI runs manually. A maintainer can start **Actions → CI → Run workflow** on a repository branch; do not assume opening a pull request starts checks automatically.

Contributions are provided under the project's [MIT license](LICENSE).
