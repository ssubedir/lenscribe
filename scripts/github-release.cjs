// Called by actions/github-script after the source manifests have been checked.
/**
 * @typedef {{ owner: string, repo: string }} Repository
 * @typedef {{ id: number, tag_name: string, draft: boolean }} ReleaseSummary
 * @typedef {ReleaseSummary & { assets: { name: string, id: number }[], html_url: string }} Release
 * @typedef {{
 *   github: {
 *     paginate: (method: unknown, input: Repository) => Promise<ReleaseSummary[]>,
 *     rest: { repos: {
 *       listReleases: unknown,
 *       createRelease: (input: Repository & { tag_name: string, name: string, draft: boolean, prerelease: boolean, generate_release_notes: boolean }) => Promise<{ data: Release }>,
 *       getRelease: (input: Repository & { release_id: number }) => Promise<{ data: Release }>,
 *       getReleaseAsset: (input: Repository & { asset_id: number, headers: { accept: string } }) => Promise<{ data: unknown }>,
 *       updateRelease: (input: Repository & { release_id: number, draft: boolean }) => Promise<{ data: Release }>
 *     } }
 *   },
 *   context: { repo: Repository },
 *   core: {
 *     setOutput: (name: string, value: string) => void,
 *     info: (message: string) => void,
 *     summary: { addLink: (label: string, url: string) => { write: () => Promise<unknown> } }
 *   }
 * }} Runtime
 */

/**
 * @typedef {{ type: string, sha: string }} GitObject
 * @typedef {{
 *   github: { rest: { git: {
 *     getRef: (input: Repository & { ref: string }) => Promise<{ data: { object: GitObject } }>,
 *     getTag: (input: Repository & { tag_sha: string }) => Promise<{ data: { object: GitObject } }>,
 *     createRef: (input: Repository & { ref: string, sha: string }) => Promise<unknown>
 *   } } },
 *   context: { repo: Repository },
 *   core: { info: (message: string) => void }
 * }} TagRuntime
 */

/** @param {unknown} error */
function statusOf(error) {
  return error && typeof error === "object" && "status" in error ? error.status : undefined;
}

/** @param {TagRuntime & { tag: string, commit: string }} options */
async function ensureTag({ github, context, core, tag, commit }) {
  // Resolve annotated tags as well as the lightweight tags this workflow creates.
  async function existingCommit() {
    let object;
    try {
      const { data } = await github.rest.git.getRef({ ...context.repo, ref: `tags/${tag}` });
      object = data.object;
    } catch (error) {
      if (statusOf(error) === 404) return null;
      throw error;
    }
    while (object.type === "tag") {
      const { data } = await github.rest.git.getTag({ ...context.repo, tag_sha: object.sha });
      object = data.object;
    }
    if (object.type !== "commit") throw new Error(`${tag} does not point to a commit.`);
    return object.sha;
  }

  let existing = await existingCommit();
  if (!existing) {
    try {
      await github.rest.git.createRef({ ...context.repo, ref: `refs/tags/${tag}`, sha: commit });
      core.info(`Created ${tag} at ${commit}.`);
      return;
    } catch (error) {
      // Another run may have created the tag after the initial lookup.
      if (![409, 422].includes(Number(statusOf(error)))) throw error;
      existing = await existingCommit();
      if (!existing) throw error;
    }
  }
  if (existing !== commit)
    throw new Error(`${tag} points to a different commit. Existing tags are never overwritten.`);
  core.info(`Using existing tag ${tag} at ${commit}.`);
}

/** @param {Runtime & { tag: string }} options */
async function prepare({ github, context, core, tag }) {
  const releases = await github.paginate(github.rest.repos.listReleases, context.repo);
  let release = releases.find((entry) => entry.tag_name === tag);
  if (release && !release.draft)
    throw new Error(`${tag} is already published. Use a new version tag.`);
  if (!release) {
    const response = await github.rest.repos.createRelease({
      ...context.repo,
      tag_name: tag,
      name: `Lenscribe ${tag}`,
      draft: true,
      prerelease: tag.includes("-"),
      generate_release_notes: true,
    });
    release = response.data;
  }
  core.setOutput("release-id", String(release.id));
  core.setOutput("tag", tag);
  core.info(`Using draft release ${tag} (${release.id}).`);
}

/** @param {Runtime & { releaseId: string, tag: string }} options */
async function publish({ github, context, core, releaseId, tag }) {
  const { data: release } = await github.rest.repos.getRelease({
    ...context.repo,
    release_id: Number(releaseId),
  });
  if (!release.draft || release.tag_name !== tag)
    throw new Error("The release changed while installers were being built. Publication stopped.");
  if (!release.assets.length) throw new Error("The release has no installers to publish.");
  const names = release.assets.map((asset) => asset.name);
  if (
    !names.includes("latest.json") ||
    !names.some((name) => name.endsWith(".exe.sig")) ||
    !names.some((name) => name.endsWith(".AppImage.sig")) ||
    names.filter((name) => name.endsWith(".app.tar.gz.sig")).length < 2
  )
    throw new Error(
      "Signed updater assets are incomplete. Keep the release draft and retry the failed builds.",
    );
  const manifestAsset = release.assets.find((asset) => asset.name === "latest.json");
  if (!manifestAsset) throw new Error("Updater manifest is missing.");
  const { data } = await github.rest.repos.getReleaseAsset({
    ...context.repo,
    asset_id: manifestAsset.id,
    headers: { accept: "application/octet-stream" },
  });
  const raw =
    typeof data === "string"
      ? data
      : data instanceof ArrayBuffer
        ? Buffer.from(data).toString("utf8")
        : ArrayBuffer.isView(data)
          ? Buffer.from(data.buffer, data.byteOffset, data.byteLength).toString("utf8")
          : null;
  let manifest;
  try {
    manifest = raw === null ? data : JSON.parse(raw);
  } catch {
    throw new Error("Updater manifest is invalid. Publication stopped.");
  }
  validateManifest(manifest, context.repo, tag, release.assets);
  const { data: published } = await github.rest.repos.updateRelease({
    ...context.repo,
    release_id: release.id,
    draft: false,
  });
  await core.summary.addLink(`Download Lenscribe ${tag}`, published.html_url).write();
}

/** @param {unknown} manifest @param {Repository} repo @param {string} tag @param {{name: string, id: number}[]} assets */
function validateManifest(manifest, repo, tag, assets) {
  const fail = () => {
    throw new Error(
      "Updater manifest is incomplete or points to the wrong release. Publication stopped.",
    );
  };
  if (
    !manifest ||
    typeof manifest !== "object" ||
    !("version" in manifest) ||
    typeof manifest.version !== "string" ||
    manifest.version.replace(/^v/, "") !== tag.replace(/^v/, "") ||
    !("platforms" in manifest) ||
    !manifest.platforms ||
    typeof manifest.platforms !== "object"
  )
    return fail();
  const platforms = /** @type {Record<string, unknown>} */ (manifest.platforms);
  const selected = new Set();
  for (const [platform, suffix] of [
    ["windows-x86_64", ".exe"],
    ["linux-x86_64", ".AppImage"],
    ["darwin-aarch64", ".app.tar.gz"],
    ["darwin-x86_64", ".app.tar.gz"],
  ]) {
    const entry = platforms[platform];
    if (
      !entry ||
      typeof entry !== "object" ||
      !("signature" in entry) ||
      typeof entry.signature !== "string" ||
      !entry.signature.trim() ||
      !("url" in entry) ||
      typeof entry.url !== "string"
    )
      return fail();
    try {
      const url = new URL(entry.url);
      const prefix = `/${repo.owner}/${repo.repo}/releases/download/${encodeURIComponent(tag)}/`;
      if (url.username || url.password || url.search || url.hash) return fail();
      const asset = assets.find(
        (asset) =>
          (url.origin === "https://github.com" &&
            url.pathname.startsWith(prefix) &&
            asset.name === decodeURIComponent(url.pathname.slice(prefix.length))) ||
          (url.origin === "https://api.github.com" &&
            url.pathname === `/repos/${repo.owner}/${repo.repo}/releases/assets/${asset.id}`),
      );
      if (!asset || !asset.name.endsWith(suffix) || selected.has(asset.id)) return fail();
      selected.add(asset.id);
    } catch {
      return fail();
    }
  }
}

module.exports = { ensureTag, prepare, publish };
