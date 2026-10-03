import { expect, mock, test } from "bun:test";
import { createUpdater } from "../src/lib/clients/updater";
import type { DownloadEvent } from "@tauri-apps/plugin-updater";

function fixture(failure?: "download" | "prepare" | "install") {
  const order: string[] = [];
  const record = async (stage: string) => {
    order.push(stage);
    if (failure === stage) throw new Error("Sensitive internal error");
  };
  const update = {
    version: "0.2.0",
    body: "Release notes",
    download: mock(async (onEvent: (event: DownloadEvent) => void) => {
      onEvent({ event: "Started", data: { contentLength: 100 } });
      onEvent({ event: "Progress", data: { chunkLength: 40 } });
      await record("download");
      onEvent({ event: "Finished" });
    }),
    install: mock(() => record("install")),
    close: mock(async () => {
      order.push("close");
    }),
  };
  const dependencies = {
    check: mock(async () => update),
    prepare: mock(() => record("prepare")),
    relaunch: mock(() => record("relaunch")),
  };
  return { update, dependencies, order, client: createUpdater(dependencies) };
}

test("update download and signature verification finish before monitoring stops", async () => {
  const { client, order } = fixture();
  expect(await client.checkUpdate()).toEqual({ version: "0.2.0", notes: "Release notes" });
  const progress: (number | null)[] = [];
  await client.installUpdate((percent) => progress.push(percent));
  expect(progress).toEqual([0, 40, 100]);
  expect(order).toEqual(["download", "prepare", "install", "relaunch", "close"]);
});

test("a failed download or signature never stops monitoring or runs the installer", async () => {
  const { client, order } = fixture("download");
  await client.checkUpdate();
  await expect(client.installUpdate(() => {})).rejects.toThrow("Monitoring is still running");
  expect(order).toEqual(["download", "close"]);
  await expect(client.installUpdate(() => {})).rejects.toThrow("Check for updates");
});

test("failed shutdown or installation reports how to resume monitoring", async () => {
  for (const stage of ["prepare", "install"] as const) {
    const { client, dependencies } = fixture(stage);
    await client.checkUpdate();
    await expect(client.installUpdate(() => {})).rejects.toThrow("Restart Lenscribe");
    expect(dependencies.relaunch).not.toHaveBeenCalled();
  }
});

test("a second check releases the old resource; shutdown releases late responses", async () => {
  const { client, update, dependencies } = fixture();
  await client.checkUpdate();
  await client.checkUpdate();
  expect(update.close).toHaveBeenCalledTimes(1);
  const pending = Promise.withResolvers<typeof update>();
  dependencies.check.mockImplementation(() => pending.promise);
  const checking = client.checkUpdate();
  await Promise.resolve();
  await client.dispose?.();
  pending.resolve(update);
  expect(await checking).toBeNull();
  expect(update.close).toHaveBeenCalledTimes(3);
});

test("overlapping checks or installs cannot replace the selected update", async () => {
  const { client, update, dependencies } = fixture();
  const pending = Promise.withResolvers<typeof update>();
  dependencies.check.mockImplementation(() => pending.promise);
  const checking = client.checkUpdate();
  await expect(client.checkUpdate()).rejects.toThrow("already in progress");
  await expect(client.installUpdate(() => {})).rejects.toThrow("Check for updates");
  pending.resolve(update);
  await checking;
  const downloading = Promise.withResolvers<void>();
  update.download.mockImplementation(() => downloading.promise);
  const installing = client.installUpdate(() => {});
  await expect(client.installUpdate(() => {})).rejects.toThrow("Check for updates");
  await expect(client.checkUpdate()).rejects.toThrow("already in progress");
  downloading.resolve();
  await installing;
});
