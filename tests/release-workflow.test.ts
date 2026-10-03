import { expect, mock, test } from "bun:test";

const workflow = Bun.YAML.parse(
  await Bun.file(new URL("../.github/workflows/release.yml", import.meta.url)).text(),
) as {
  jobs: {
    prepare: { steps: { id?: string; name?: string; run?: string; with?: { script?: string } }[] };
  };
};
const script = workflow.jobs.prepare.steps.find((step) => step.id === "tag")?.with?.script;
if (!script) throw new Error("Release tag preflight is missing from the workflow.");
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const preflight = new AsyncFunction("github", "context", "core", "process", script);

function fixture(tag: string, status?: number, eventName = "workflow_dispatch") {
  const getRef = mock(async (_input: unknown) => {
    if (status) throw Object.assign(new Error("GitHub request failed"), { status });
    return { data: { ref: `refs/tags/${tag}` } };
  });
  const core = {
    setFailed: mock((_message: string) => {}),
    setOutput: mock((_name: string, _value: string) => {}),
  };
  return {
    getRef,
    core,
    run: () =>
      preflight(
        { rest: { git: { getRef } } },
        { repo: { owner: "test", repo: "scribe" }, eventName, sha: "selected-branch-commit" },
        core,
        { env: { RELEASE_TAG: tag } },
      ),
  };
}

test("retry builds the existing tag instead of the selected branch", async () => {
  const existing = fixture("v0.2.0-beta.1");
  await existing.run();
  expect(existing.getRef).toHaveBeenCalledWith({
    owner: "test",
    repo: "scribe",
    ref: "tags/v0.2.0-beta.1",
  });
  expect(existing.core.setOutput).toHaveBeenCalledWith("tag", "v0.2.0-beta.1");
  expect(existing.core.setOutput).toHaveBeenCalledWith("ref", "refs/tags/v0.2.0-beta.1");
  expect(existing.core.setFailed).not.toHaveBeenCalled();
});

test("new manual release checks out the selected branch commit before creating its tag", async () => {
  const missing = fixture("v0.1.1", 404);
  await missing.run();
  expect(missing.core.setOutput).toHaveBeenCalledWith("tag", "v0.1.1");
  expect(missing.core.setOutput).toHaveBeenCalledWith("ref", "selected-branch-commit");
  expect(missing.core.setFailed).not.toHaveBeenCalled();
});

test("a deleted pushed tag is not silently recreated at the workflow commit", async () => {
  const missing = fixture("v0.1.1", 404, "push");
  await missing.run();
  expect(missing.core.setFailed).toHaveBeenCalled();
  expect(missing.core.setOutput).not.toHaveBeenCalled();
});

test("version validation must succeed before the workflow creates the tag or draft", () => {
  const steps = workflow.jobs.prepare.steps;
  const validation = steps.findIndex((step) => step.run?.includes("release:version --check"));
  const release = steps.findIndex((step) => step.id === "release");
  expect(validation).toBeGreaterThan(steps.findIndex((step) => step.id === "checkout"));
  expect(release).toBeGreaterThan(validation);
  expect(steps[release].with?.script).toContain("await ensureTag(");
  expect(steps[release].with?.script).toContain("await prepare(");
});

test("manual input rejects branch names and tag prefixes before making a request", async () => {
  for (const tag of ["main", "0.1.1", "refs/tags/v0.1.1", "v0.1.1\n", ""]) {
    const invalid = fixture(tag);
    await invalid.run();
    expect(invalid.getRef).not.toHaveBeenCalled();
    expect(invalid.core.setFailed).toHaveBeenCalled();
    expect(invalid.core.setOutput).not.toHaveBeenCalled();
  }
});

test("authentication and network failures retain the real GitHub error", async () => {
  const forbidden = fixture("v0.1.1", 403);
  await expect(forbidden.run()).rejects.toThrow("GitHub request failed");
  expect(forbidden.core.setFailed).not.toHaveBeenCalled();
  expect(forbidden.core.setOutput).not.toHaveBeenCalled();
});
