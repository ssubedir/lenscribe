import { expect, mock, test } from "bun:test";
import { SettingsController } from "../src/lib/settings/controller.svelte";
import { createPreviewClient } from "../src/lib/clients/preview";
import type { UpdateStatus } from "../src/lib/clients/types";

function available(revision = 1): UpdateStatus {
  return {
    revision,
    supported: true,
    supportMessage: null,
    phase: "available",
    available: { version: "0.2.0", notes: "Sample release" },
    progress: null,
    lastChecked: 123,
    error: null,
  };
}

test("background update events show the notice without checking from the UI", async () => {
  const client = createPreviewClient();
  const check = mock(client.checkUpdate);
  let publish: (status: UpdateStatus) => void = () => {};
  const model = new SettingsController();
  model.start({
    ...client,
    checkUpdate: check,
    onUpdate: async (callback) => {
      publish = callback;
      return () => {};
    },
  });
  await model.refresh();
  publish(available());
  expect(model.update?.available?.version).toBe("0.2.0");
  expect(check).not.toHaveBeenCalled();
  publish({ ...available(2), phase: "downloading", progress: 40 });
  expect(model.busy).toBe("update");
  expect(model.update?.progress).toBe(40);
  model.destroy();
});

test("older status responses cannot erase a newer update event", async () => {
  const client = createPreviewClient();
  const pending = Promise.withResolvers<UpdateStatus>();
  let publish: (status: UpdateStatus) => void = () => {};
  const model = new SettingsController();
  model.start({
    ...client,
    updateStatus: () => pending.promise,
    onUpdate: async (callback) => {
      publish = callback;
      return () => {};
    },
  });
  publish(available(3));
  pending.resolve({ ...available(1), available: null, phase: "idle" });
  await model.refresh();
  expect(model.update?.available?.version).toBe("0.2.0");
  model.destroy();
});

test("unsaved settings block installation but allow checking", async () => {
  const client = createPreviewClient();
  const install = mock(client.installUpdate);
  const model = new SettingsController();
  model.start({ ...client, installUpdate: install });
  await model.refresh();
  if (!model.draft) throw new Error("Missing draft");
  model.draft.theme = "dark";
  await model.checkUpdate();
  await model.installUpdate();
  expect(model.update?.available).not.toBeNull();
  expect(install).not.toHaveBeenCalled();
  model.destroy();
});

test("installation uses the advertised version and failure restores controls", async () => {
  const client = createPreviewClient();
  const pending = Promise.withResolvers<void>();
  const install = mock((_version: string) => pending.promise);
  const model = new SettingsController();
  model.start({ ...client, installUpdate: install });
  await model.refresh();
  await model.checkUpdate();
  const installing = model.installUpdate();
  expect(model.busy).toBe("update");
  expect(install).toHaveBeenCalledWith("0.2.0");
  await model.refresh();
  expect(model.busy).toBe("update");
  model.navigate("folders");
  expect(model.page).toBe("overview");
  pending.reject(new Error("Monitoring has resumed"));
  await installing;
  expect(model.busy).toBeNull();
  expect(model.updateError).toContain("Monitoring has resumed");
  expect(model.update?.available).not.toBeNull();
  model.destroy();
});

test("destroy removes late update subscriptions and ignores their results", async () => {
  const client = createPreviewClient();
  const pending = Promise.withResolvers<() => void>();
  let publish: (status: UpdateStatus) => void = () => {};
  const remove = mock(() => {});
  const model = new SettingsController();
  model.start({
    ...client,
    onUpdate: (callback) => {
      publish = callback;
      return pending.promise;
    },
  });
  model.destroy();
  pending.resolve(remove);
  publish(available());
  await Promise.resolve();
  expect(remove).toHaveBeenCalled();
  expect(model.update).toBeNull();
});

test("preview updates remain isolated and clear the available update after simulation", async () => {
  const model = new SettingsController();
  model.start(createPreviewClient());
  await model.refresh();
  await model.checkUpdate();
  await model.installUpdate();
  expect(model.update?.available).toBeNull();
  expect(model.busy).toBeNull();
  expect(model.message).toContain("No update was installed");
  model.destroy();
});

test("polling recovers update progress and failures if events are missed", async () => {
  const client = createPreviewClient();
  const daemonStatus = mock(client.status);
  let remote = available();
  const model = new SettingsController();
  model.start({ ...client, status: daemonStatus, updateStatus: async () => remote });
  await model.refresh();
  remote = { ...available(2), phase: "downloading", progress: 40 };
  await model.refresh();
  expect(model.busy).toBe("update");
  const calls = daemonStatus.mock.calls.length;
  remote = { ...available(3), phase: "downloading", progress: 80 };
  await model.refresh();
  expect(model.update?.progress).toBe(80);
  expect(daemonStatus.mock.calls.length).toBe(calls);
  remote = { ...available(4), phase: "error", error: "Monitoring has resumed" };
  await model.refresh();
  expect(model.busy).toBeNull();
  expect(model.update?.error).toContain("Monitoring has resumed");
  model.destroy();
});
