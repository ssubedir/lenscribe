import { expect, mock, test } from "bun:test";
import { ensureTag } from "../scripts/github-release.cjs";

type GitObject = { type: string; sha: string };

function fixture(initial: GitObject | null = null) {
  let object = initial;
  const git = {
    getRef: mock(async (_input: unknown) => {
      if (!object) throw Object.assign(new Error("Tag not found"), { status: 404 });
      return { data: { object } };
    }),
    getTag: mock(async (_input: unknown) => ({
      data: { object: { type: "commit", sha: "release-commit" } },
    })),
    createRef: mock(async (_input: unknown) => {
      object = { type: "commit", sha: "release-commit" };
    }),
  };
  return {
    git,
    setRef: (value: GitObject) => {
      object = value;
    },
    github: { rest: { git } },
    context: { repo: { owner: "test", repo: "scribe" } },
    core: { info: mock((_message: string) => {}) },
    tag: "v0.1.1",
    commit: "release-commit",
  };
}

test("a new release creates its tag at the validated checkout commit", async () => {
  const options = fixture();
  await ensureTag(options);
  expect(options.git.createRef).toHaveBeenCalledWith({
    owner: "test",
    repo: "scribe",
    ref: "refs/tags/v0.1.1",
    sha: "release-commit",
  });
});

test("an existing tag at the same commit can be retried without mutation", async () => {
  const options = fixture({ type: "commit", sha: "release-commit" });
  await ensureTag(options);
  expect(options.git.createRef).not.toHaveBeenCalled();
});

test("existing annotated tags resolve to their original source commit", async () => {
  const options = fixture({ type: "tag", sha: "annotated-tag" });
  await ensureTag(options);
  expect(options.git.getTag).toHaveBeenCalledWith({
    owner: "test",
    repo: "scribe",
    tag_sha: "annotated-tag",
  });
  expect(options.git.createRef).not.toHaveBeenCalled();
});

test("an existing tag is never changed to a different release commit", async () => {
  const options = fixture({ type: "commit", sha: "another-commit" });
  await expect(ensureTag(options)).rejects.toThrow("different commit");
  expect(options.git.createRef).not.toHaveBeenCalled();
});

test("a tag pointing to a tree or blob cannot be released", async () => {
  const options = fixture({ type: "tree", sha: "tree-object" });
  await expect(ensureTag(options)).rejects.toThrow("does not point to a commit");
  expect(options.git.createRef).not.toHaveBeenCalled();
});

test("permission failures are not treated as missing tags", async () => {
  const options = fixture();
  const error = Object.assign(new Error("Forbidden"), { status: 403 });
  options.git.getRef.mockRejectedValueOnce(error);
  await expect(ensureTag(options)).rejects.toBe(error);
  expect(options.git.createRef).not.toHaveBeenCalled();
});

test("concurrent tag creation is reusable only when it targets the same commit", async () => {
  for (const sha of ["release-commit", "another-commit"]) {
    const options = fixture();
    options.git.createRef.mockImplementationOnce(async () => {
      options.setRef({ type: "commit", sha });
      throw Object.assign(new Error("Reference already exists"), { status: 422 });
    });
    if (sha === "release-commit") await ensureTag(options);
    else await expect(ensureTag(options)).rejects.toThrow("different commit");
    expect(options.git.createRef).toHaveBeenCalledTimes(1);
    expect(options.git.getRef).toHaveBeenCalledTimes(2);
  }
});

test("tag creation errors retain the real error when no tag was created", async () => {
  const options = fixture();
  const error = Object.assign(new Error("Reference validation failed"), { status: 422 });
  options.git.createRef.mockRejectedValueOnce(error);
  await expect(ensureTag(options)).rejects.toBe(error);
});
