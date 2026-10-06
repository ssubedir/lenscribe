import { afterEach, expect, mock, test } from "bun:test";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkReleaseVersion, setReleaseVersion } from "../scripts/release-version";
import { prepare, publish } from "../scripts/github-release.cjs";

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(
    directories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })),
  );
});

async function versionFixture() {
  const directory = await mkdtemp(join(tmpdir(), "lenscribe-release-test-"));
  directories.push(directory);
  await mkdir(join(directory, "src-tauri/crates"), { recursive: true });
  const sources = {
    "package.json":
      '{\n  "name": "lenscribe",\n  "version": "0.1.0",\n  "scripts": {"test": "bun test"}\n}\n',
    "src-tauri/tauri.conf.json": '{"version":"0.1.0","bundle":{"active":true}}\n',
    "src-tauri/Cargo.toml":
      '# Keep this comment\r\n[package]\r\nname = "lenscribe"\r\nversion = "0.1.0"\r\n\r\n[dependencies]\r\nserde = "1"\r\n',
    "src-tauri/crates/Cargo.toml": '[package]\nname = "lenscribe-core"\nversion = "0.1.0"\n',
    "src-tauri/Cargo.lock":
      'version = 4\n\n[[package]]\nname = "lenscribe"\nversion = "0.1.0"\n\n[[package]]\nname = "lenscribe-core"\nversion = "0.1.0"\n\n[[package]]\nname = "unrelated"\nversion = "0.1.0"\nchecksum = "keep-me"\n',
  };
  await Promise.all(
    Object.entries(sources).map(([path, source]) => writeFile(join(directory, path), source)),
  );
  return { directory, sources };
}

test("version command updates every manifest and only workspace lock entries", async () => {
  const { directory } = await versionFixture();
  expect(await setReleaseVersion(directory, "0.2.0-beta.1")).toEqual({
    version: "0.2.0-beta.1",
    tag: "v0.2.0-beta.1",
    prerelease: true,
  });
  const manifest = await readFile(join(directory, "src-tauri/Cargo.toml"), "utf8");
  expect(manifest).toContain(
    '# Keep this comment\r\n[package]\r\nname = "lenscribe"\r\nversion = "0.2.0-beta.1"\r\n',
  );
  expect(manifest).toContain('serde = "1"');
  const lock = Bun.TOML.parse(await readFile(join(directory, "src-tauri/Cargo.lock"), "utf8")) as {
    package: { name: string; version: string; checksum?: string }[];
  };
  expect(lock.package.find((entry) => entry.name === "unrelated")).toEqual({
    name: "unrelated",
    version: "0.1.0",
    checksum: "keep-me",
  });
});

test("release check rejects a tag that differs from the app version", async () => {
  const { directory } = await versionFixture();
  await expect(checkReleaseVersion(directory, "v9.0.0")).rejects.toThrow("does not match");
  expect(await checkReleaseVersion(directory, "v0.1.0")).toEqual({
    version: "0.1.0",
    tag: "v0.1.0",
    prerelease: false,
  });
});

test("release check rejects version drift; the version command repairs it", async () => {
  const { directory } = await versionFixture();
  await writeFile(join(directory, "src-tauri/tauri.conf.json"), '{"version":"0.1.1"}');
  await expect(checkReleaseVersion(directory)).rejects.toThrow("versions disagree");
  await setReleaseVersion(directory, "0.1.2");
  expect((await checkReleaseVersion(directory)).version).toBe("0.1.2");
});

test("invalid versions fail before changing files", async () => {
  const { directory, sources } = await versionFixture();
  for (const version of ["v0.1.0", "01.2.3", "1.2", "1.2.3-beta.01", "1.2.3\n", "$(bad)"])
    await expect(setReleaseVersion(directory, version)).rejects.toThrow("Invalid release version");
  for (const [path, source] of Object.entries(sources))
    expect(await readFile(join(directory, path), "utf8")).toBe(source);
});

test("missing workspace lock entries fail before changing manifests", async () => {
  const { directory, sources } = await versionFixture();
  await writeFile(
    join(directory, "src-tauri/Cargo.lock"),
    'version = 4\n[[package]]\nname = "lenscribe"\nversion = "0.1.0"\n',
  );
  await expect(setReleaseVersion(directory, "0.2.0")).rejects.toThrow(
    "Expected one lenscribe-core",
  );
  expect(await readFile(join(directory, "package.json"), "utf8")).toBe(sources["package.json"]);
});

function githubFixture(releases: { id: number; tag_name: string; draft: boolean }[] = []) {
  const release = {
    id: 17,
    tag_name: "v0.1.0",
    draft: true,
    assets: [
      "installer.exe",
      "installer.exe.sig",
      "linux.AppImage",
      "linux.AppImage.sig",
      "arm.app.tar.gz",
      "arm.app.tar.gz.sig",
      "intel.app.tar.gz",
      "intel.app.tar.gz.sig",
      "latest.json",
    ].map((name, id) => ({ name, id: id + 1 })),
    html_url: "https://example.test/release",
  };
  const repos = {
    listReleases: mock(() => Promise.resolve({ data: releases })),
    createRelease: mock(async (_input: unknown) => ({ data: release })),
    getRelease: mock(async (_input: unknown) => ({ data: release })),
    getReleaseAsset: mock(async (_input: unknown): Promise<{ data: unknown }> => ({
      data: JSON.stringify({
        version: "0.1.0",
        platforms: Object.fromEntries(
          [
            ["windows-x86_64", "installer.exe"],
            ["linux-x86_64", "linux.AppImage"],
            ["darwin-aarch64", "arm.app.tar.gz"],
            ["darwin-x86_64", "intel.app.tar.gz"],
          ].map(([platform, name]) => [
            platform,
            {
              signature: "fixture-signature",
              url: `https://api.github.com/repos/test/lenscribe/releases/assets/${release.assets.find((asset) => asset.name === name)?.id}`,
            },
          ]),
        ),
      }),
    })),
    updateRelease: mock(async (_input: unknown) => ({ data: { ...release, draft: false } })),
  };
  const summary = {
    addLink: mock((_label: string, _url: string) => summary),
    write: mock(async () => {}),
  };
  return {
    release,
    github: {
      rest: { repos },
      paginate: mock(async (_method: unknown, _input: unknown) => releases),
    },
    context: { repo: { owner: "test", repo: "lenscribe" } },
    core: {
      setOutput: mock((_key: string, _value: string) => {}),
      info: mock((_message: string) => {}),
      summary,
    },
  };
}

test("prepare creates a draft prerelease with generated notes", async () => {
  const fixture = githubFixture();
  await prepare({ ...fixture, tag: "v0.2.0-beta.1" });
  expect(fixture.github.rest.repos.createRelease).toHaveBeenCalledWith({
    owner: "test",
    repo: "lenscribe",
    tag_name: "v0.2.0-beta.1",
    name: "Lenscribe v0.2.0-beta.1",
    draft: true,
    prerelease: true,
    generate_release_notes: true,
  });
  expect(fixture.core.setOutput).toHaveBeenCalledWith("release-id", "17");
});

test("rerunning a failed build reuses its draft; published versions are protected", async () => {
  const fixture = githubFixture([{ id: 12, tag_name: "v0.1.0", draft: true }]);
  await prepare({ ...fixture, tag: "v0.1.0" });
  expect(fixture.github.rest.repos.createRelease).not.toHaveBeenCalled();
  expect(fixture.core.setOutput).toHaveBeenCalledWith("release-id", "12");
  const published = githubFixture([{ id: 12, tag_name: "v0.1.0", draft: false }]);
  await expect(prepare({ ...published, tag: "v0.1.0" })).rejects.toThrow("already published");
  expect(published.github.rest.repos.createRelease).not.toHaveBeenCalled();
});

test("publish exposes the completed draft and reports its download URL", async () => {
  const fixture = githubFixture();
  await publish({ ...fixture, tag: "v0.1.0", releaseId: "17" });
  expect(fixture.github.rest.repos.updateRelease).toHaveBeenCalledWith({
    owner: "test",
    repo: "lenscribe",
    release_id: 17,
    draft: false,
  });
  expect(fixture.core.summary.addLink).toHaveBeenCalledWith(
    "Download Lenscribe v0.1.0",
    fixture.release.html_url,
  );
});

test("publish stops if a release was changed or has no installers", async () => {
  const changed = githubFixture();
  changed.release.draft = false;
  await expect(publish({ ...changed, tag: "v0.1.0", releaseId: "17" })).rejects.toThrow(
    "release changed",
  );
  expect(changed.github.rest.repos.updateRelease).not.toHaveBeenCalled();
  const empty = githubFixture();
  empty.release.assets = [];
  await expect(publish({ ...empty, tag: "v0.1.0", releaseId: "17" })).rejects.toThrow(
    "no installers",
  );
  expect(empty.github.rest.repos.updateRelease).not.toHaveBeenCalled();
});

test("publish keeps incomplete signed releases in draft", async () => {
  for (const missing of [
    "latest.json",
    "installer.exe.sig",
    "linux.AppImage.sig",
    "arm.app.tar.gz.sig",
  ]) {
    const fixture = githubFixture();
    fixture.release.assets = fixture.release.assets.filter((asset) => asset.name !== missing);
    await expect(publish({ ...fixture, tag: "v0.1.0", releaseId: "17" })).rejects.toThrow(
      "updater assets are incomplete",
    );
    expect(fixture.github.rest.repos.updateRelease).not.toHaveBeenCalled();
  }
});

test("publish validates every updater platform, version and download destination", async () => {
  for (const change of [
    "missing-platform",
    "wrong-version",
    "wrong-destination",
    "wrong-asset",
    "missing-signature",
    "wrong-installer",
    "same-mac-build",
  ]) {
    const fixture = githubFixture();
    const response = await fixture.github.rest.repos.getReleaseAsset({});
    const manifest = JSON.parse(response.data as string);
    if (change === "missing-platform") delete manifest.platforms["darwin-x86_64"];
    if (change === "wrong-version") manifest.version = "0.1.1";
    if (change === "wrong-destination")
      manifest.platforms["linux-x86_64"].url = "https://example.test/linux.AppImage";
    if (change === "wrong-asset")
      manifest.platforms["linux-x86_64"].url =
        "https://github.com/test/lenscribe/releases/download/v0.1.0/missing.AppImage";
    if (change === "missing-signature") delete manifest.platforms["windows-x86_64"].signature;
    if (change === "wrong-installer")
      manifest.platforms["linux-x86_64"].url = manifest.platforms["windows-x86_64"].url;
    if (change === "same-mac-build")
      manifest.platforms["darwin-x86_64"].url = manifest.platforms["darwin-aarch64"].url;
    fixture.github.rest.repos.getReleaseAsset.mockImplementation(async () => ({
      data: new TextEncoder().encode(JSON.stringify(manifest)),
    }));
    await expect(publish({ ...fixture, tag: "v0.1.0", releaseId: "17" })).rejects.toThrow(
      "Updater manifest is incomplete",
    );
    expect(fixture.github.rest.repos.updateRelease).not.toHaveBeenCalled();
  }
});

test("publish also accepts ordinary release-download URLs for uploaded installers", async () => {
  const fixture = githubFixture();
  const response = await fixture.github.rest.repos.getReleaseAsset({});
  const manifest = JSON.parse(response.data as string);
  for (const entry of Object.values(manifest.platforms) as { url: string }[]) {
    const id = Number(entry.url.split("/").pop());
    const asset = fixture.release.assets.find((asset) => asset.id === id)!;
    entry.url = `https://github.com/test/lenscribe/releases/download/v0.1.0/${asset.name}`;
  }
  fixture.github.rest.repos.getReleaseAsset.mockImplementation(async () => ({
    data: new TextEncoder().encode(JSON.stringify(manifest)).buffer,
  }));
  await publish({ ...fixture, tag: "v0.1.0", releaseId: "17" });
  expect(fixture.github.rest.repos.updateRelease).toHaveBeenCalled();
});
