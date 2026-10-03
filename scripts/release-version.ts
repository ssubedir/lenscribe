import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dir, "..");
const versionPattern =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?$/;
const manifests = ["src-tauri/Cargo.toml", "src-tauri/crates/lenscribe-core/Cargo.toml"];

function validateVersion(version: string) {
  if (version !== version.trim() || !versionPattern.test(version))
    throw new Error(
      `Invalid release version: ${version}. Use a version such as 0.1.1 or 0.2.0-beta.1.`,
    );
}

async function readVersions(directory: string) {
  const paths = ["package.json", "src-tauri/tauri.conf.json", ...manifests, "src-tauri/Cargo.lock"];
  const sources = await Promise.all(
    paths.map((path) => readFile(resolve(directory, path), "utf8")),
  );
  const frontend = JSON.parse(sources[0]);
  const config = JSON.parse(sources[1]);
  const crates = sources
    .slice(2, 4)
    .map((source) => Bun.TOML.parse(source) as { package: { name: string; version: string } });
  const lock = Bun.TOML.parse(sources[4]) as { package: { name: string; version: string }[] };
  const locked = crates.map((crate) => {
    const matches = lock.package.filter((entry) => entry.name === crate.package.name);
    if (matches.length !== 1)
      throw new Error(`Expected one ${crate.package.name} entry in Cargo.lock.`);
    return matches[0];
  });
  const versions = [
    frontend.version,
    config.version,
    ...crates.map((crate) => crate.package.version),
    ...locked.map((crate) => crate.version),
  ];
  versions.forEach(validateVersion);
  return { paths, sources, versions, crates };
}

export async function checkReleaseVersion(directory: string, tag?: string) {
  const { versions } = await readVersions(directory);
  const version = versions[0];
  if (versions.some((entry) => entry !== version))
    throw new Error(
      "Release versions disagree. Run bun run release:version <version> to synchronize them.",
    );
  if (tag !== undefined && tag !== `v${version}`)
    throw new Error(`Release tag ${tag} does not match the app version v${version}.`);
  return { version, tag: `v${version}`, prerelease: version.includes("-") };
}

export async function setReleaseVersion(directory: string, version: string) {
  validateVersion(version);
  const { paths, sources, crates } = await readVersions(directory);
  const updated = sources.map((source, index) => {
    if (index < 2)
      return source.replace(
        /("version"\s*:\s*")[^"]+("\s*,?)/,
        (_, before, after) => before + version + after,
      );
    if (index < 4) {
      const replaced = source.replace(
        /(\[package\][\s\S]*?\nversion\s*=\s*")[^"]+("[^\r\n]*)/,
        (_, before, after) => before + version + after,
      );
      if (replaced === source && crates[index - 2].package.version !== version)
        throw new Error(`Could not update ${paths[index]}.`);
      return replaced;
    }
    return source.replace(/\[\[package\]\][\s\S]*?(?=\n\[\[package\]\]|$)/g, (entry) => {
      const name = (Bun.TOML.parse(entry) as { package: { name: string }[] }).package[0].name;
      return crates.some((crate) => crate.package.name === name)
        ? entry.replace(
            /(\nversion\s*=\s*")[^"]+("[^\r\n]*)/,
            (_, before, after) => before + version + after,
          )
        : entry;
    });
  });
  // Validate every replacement before writing any manifest.
  updated.slice(2).forEach((source) => Bun.TOML.parse(source));
  await Promise.all(
    updated.map((source, index) => writeFile(resolve(directory, paths[index]), source)),
  );
  return checkReleaseVersion(directory);
}

if (import.meta.main) {
  try {
    const args = Bun.argv.slice(2);
    let result;
    if (args[0] === "--check" && args.length <= 2)
      result = await checkReleaseVersion(root, args[1]);
    else if (args.length === 1) result = await setReleaseVersion(root, args[0]);
    else throw new Error("Usage: bun run release:version <version> | --check [v<version>]");
    console.log(`Release versions match ${result.tag}${result.prerelease ? " (prerelease)" : ""}.`);
  } catch (cause) {
    console.error(cause instanceof Error ? cause.message : String(cause));
    process.exitCode = 1;
  }
}
