import { expect, mock, test } from "bun:test";

type Step = { uses?: string; with?: Record<string, string>; env?: Record<string, string> };
type Job = {
  needs?: string | string[];
  if?: string;
  permissions?: Record<string, string>;
  steps: Step[];
};
const workflow = Bun.YAML.parse(
  await Bun.file(new URL("../.github/workflows/ci.yml", import.meta.url)).text(),
) as {
  on: Record<string, unknown>;
  permissions: Record<string, string>;
  jobs: Record<string, Job>;
};
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
function scriptFor(job: string) {
  const script = workflow.jobs[job].steps.find((step) => step.with?.script)?.with?.script;
  if (!script) throw new Error(`CI ${job} script is missing.`);
  return new AsyncFunction("github", "context", "core", "process", script);
}
const selectSource = scriptFor("source");
const reportResult = scriptFor("result");

function fixture() {
  const context = {
    repo: { owner: "test", repo: "lenscribe" },
    serverUrl: "https://github.com",
    runId: 123,
    sha: "selected-branch-commit",
    ref: "refs/heads/main",
    eventName: "issue_comment",
    actor: "rerun-actor",
    payload: {
      issue: { number: 42, pull_request: {} as unknown },
      comment: { body: "/ci", user: { login: "comment-author" } },
    },
  };
  const getCollaboratorPermissionLevel = mock(async (_input: unknown) => ({
    data: { permission: "write" },
  }));
  const get = mock(async (_input: unknown) => ({
    data: {
      state: "open",
      head: { sha: "pr-head-commit", repo: { full_name: "contributor/fork" } },
    },
  }));
  const createCommitStatus = mock(async (_input: unknown) => {});
  const listCommitStatusesForRef = mock(async (_input: unknown) => ({
    data: [
      {
        context: "unrelated-check",
        target_url: "https://example.com",
      },
      {
        context: "CI / Manual PR",
        target_url: "https://github.com/test/lenscribe/actions/runs/123",
      },
    ],
  }));
  const github = {
    rest: {
      repos: { getCollaboratorPermissionLevel, createCommitStatus, listCommitStatusesForRef },
      pulls: { get },
    },
  };
  const core = {
    setOutput: mock((_name: string, _value: string) => {}),
    info: mock((_message: string) => {}),
  };
  return {
    context,
    core,
    getCollaboratorPermissionLevel,
    get,
    createCommitStatus,
    listCommitStatusesForRef,
    select: () => selectSource(github, context, core, { env: {} }),
    report: (frontend = "success", native = "success") =>
      reportResult(github, context, core, {
        env: {
          CI_COMMIT: "pr-head-commit",
          FRONTEND_RESULT: frontend,
          NATIVE_RESULT: native,
        },
      }),
  };
}

test("manual branch CI pins the dispatch commit without publishing a PR status", async () => {
  const run = fixture();
  run.context.eventName = "workflow_dispatch";
  await run.select();
  expect(run.core.setOutput).toHaveBeenCalledWith("commit", "selected-branch-commit");
  expect(run.core.setOutput).toHaveBeenCalledWith("pr", "");
  expect(run.getCollaboratorPermissionLevel).not.toHaveBeenCalled();
  expect(run.get).not.toHaveBeenCalled();
  expect(run.createCommitStatus).not.toHaveBeenCalled();
});

test("a maintainer's PR command pins the fork head and links its pending status", async () => {
  for (const permission of ["admin", "write"]) {
    const run = fixture();
    run.getCollaboratorPermissionLevel.mockResolvedValueOnce({ data: { permission } });
    await run.select();
    expect(run.getCollaboratorPermissionLevel).toHaveBeenCalledWith({
      owner: "test",
      repo: "lenscribe",
      username: "comment-author",
    });
    expect(run.get).toHaveBeenCalledWith({ owner: "test", repo: "lenscribe", pull_number: 42 });
    expect(run.core.setOutput).toHaveBeenCalledWith("commit", "pr-head-commit");
    expect(run.core.setOutput).toHaveBeenCalledWith("pr", "42");
    expect(run.createCommitStatus).toHaveBeenCalledWith({
      owner: "test",
      repo: "lenscribe",
      sha: "pr-head-commit",
      context: "CI / Manual PR",
      state: "pending",
      description: "Manual CI is running",
      target_url: "https://github.com/test/lenscribe/actions/runs/123",
    });
  }
});

test("non-maintainers cannot select PR code or change its CI status", async () => {
  for (const permission of ["read", "none"]) {
    const run = fixture();
    run.getCollaboratorPermissionLevel.mockResolvedValueOnce({ data: { permission } });
    await expect(run.select()).rejects.toThrow("repository write access");
    expect(run.get).not.toHaveBeenCalled();
    expect(run.createCommitStatus).not.toHaveBeenCalled();
    expect(run.core.setOutput).not.toHaveBeenCalled();
  }
});

test("ordinary comments and issue commands do not select a CI source", async () => {
  for (const body of ["looks good", "ci", "/ci extra arguments"]) {
    const run = fixture();
    run.context.payload.comment.body = body;
    await run.select();
    expect(run.getCollaboratorPermissionLevel).not.toHaveBeenCalled();
    expect(run.core.setOutput).not.toHaveBeenCalled();
  }
  const issue = fixture();
  issue.context.payload.issue.pull_request = undefined;
  await issue.select();
  expect(issue.getCollaboratorPermissionLevel).not.toHaveBeenCalled();
  expect(issue.core.setOutput).not.toHaveBeenCalled();
});

test("closed PRs and permission lookup failures never publish a pending status", async () => {
  const closed = fixture();
  closed.get.mockResolvedValueOnce({
    data: {
      state: "closed",
      head: { sha: "pr-head-commit", repo: { full_name: "contributor/fork" } },
    },
  });
  await expect(closed.select()).rejects.toThrow("open pull request");
  expect(closed.createCommitStatus).not.toHaveBeenCalled();
  const forbidden = fixture();
  const error = Object.assign(new Error("Permission lookup failed"), { status: 403 });
  forbidden.getCollaboratorPermissionLevel.mockRejectedValueOnce(error);
  await expect(forbidden.select()).rejects.toBe(error);
  expect(forbidden.get).not.toHaveBeenCalled();
  expect(forbidden.createCommitStatus).not.toHaveBeenCalled();
});

test("PR CI succeeds only when frontend and every native target succeed", async () => {
  for (const [frontend, native, state] of [
    ["success", "success", "success"],
    ["failure", "skipped", "failure"],
    ["success", "failure", "failure"],
    ["cancelled", "skipped", "error"],
    ["success", "cancelled", "error"],
    ["success", "skipped", "error"],
  ]) {
    const run = fixture();
    await run.report(frontend, native);
    expect(run.listCommitStatusesForRef).toHaveBeenCalledWith({
      owner: "test",
      repo: "lenscribe",
      ref: "pr-head-commit",
      per_page: 100,
    });
    expect(run.createCommitStatus).toHaveBeenCalledWith(
      expect.objectContaining({
        sha: "pr-head-commit",
        context: "CI / Manual PR",
        state,
        target_url: "https://github.com/test/lenscribe/actions/runs/123",
      }),
    );
  }
});

test("an older run cannot overwrite the status owned by a newer request", async () => {
  const run = fixture();
  run.listCommitStatusesForRef.mockResolvedValueOnce({
    data: [
      {
        context: "CI / Manual PR",
        target_url: "https://github.com/test/lenscribe/actions/runs/456",
      },
    ],
  });
  await run.report("failure", "skipped");
  expect(run.createCommitStatus).not.toHaveBeenCalled();
});

test("PR code runs only in read-only jobs while status jobs never check out code", () => {
  expect(Object.keys(workflow.on).sort()).toEqual(["issue_comment", "workflow_dispatch"]);
  expect(workflow.on.issue_comment).toEqual({ types: ["created"] });
  expect(workflow.permissions).toEqual({ contents: "read" });
  for (const name of ["source", "result"]) {
    const job = workflow.jobs[name];
    expect(job.permissions?.statuses).toBe("write");
    expect(job.steps.every((step) => step.uses === "actions/github-script@v8")).toBe(true);
  }
  for (const name of ["frontend", "native"]) {
    const job = workflow.jobs[name];
    expect(job.permissions).toBeUndefined();
    expect(job.needs).toContain("source");
    const checkout = job.steps.find((step) => step.uses?.startsWith("actions/checkout@"));
    expect(checkout?.with?.ref).toBe("${{ needs.source.outputs.commit }}");
    expect(checkout?.with?.["persist-credentials"]).toBe(false);
    const cache = job.steps.find((step) => step.uses?.startsWith("Swatinem/rust-cache@"));
    expect(cache?.with?.["save-if"]).toBe("${{ needs.source.outputs.pr == '' }}");
  }
  expect(workflow.jobs.result.needs).toEqual(["source", "frontend", "native"]);
  expect(workflow.jobs.result.if).toContain("always()");
});
