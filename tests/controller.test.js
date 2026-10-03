import { expect, test } from "bun:test";
import { SettingsController } from "../src/lib/settings/controller.svelte";
import { createPreviewClient } from "../src/lib/clients/preview";

test("status refresh preserves drafts; discard adopts current saved settings", async () => {
  const client = createPreviewClient();
  const model = new SettingsController();
  model.start(client);
  await model.refresh();
  if (!model.draft) throw new Error("Settings did not load");
  model.draft.extraction.model = "Unsaved model";
  const remote = await client.status();
  remote.settings.startMinimized = true;
  await client.save(remote.settings);
  await model.refresh();
  expect(model.status?.settings.startMinimized).toBe(true);
  expect(model.draft.extraction.model).toBe("Unsaved model");
  expect(model.dirty).toBe(true);
  model.discard();
  expect(model.draft.startMinimized).toBe(true);
  expect(model.dirty).toBe(false);
  model.destroy();
});

test("older responses cannot replace newer status", async () => {
  const client = createPreviewClient();
  const model = new SettingsController();
  model.start(client);
  await model.refresh();
  const older = Promise.withResolvers(),
    newer = Promise.withResolvers();
  let calls = 0;
  model.client = { ...client, status: () => (++calls === 1 ? older.promise : newer.promise) };
  const first = model.refresh(),
    second = model.refresh();
  const latest = await client.status();
  latest.totalImages = 222;
  newer.resolve(latest);
  await second;
  older.resolve(await client.status());
  await first;
  expect(model.status?.totalImages).toBe(222);
  model.destroy();
});

test("save normalizes exclusions and makes the saved draft clean", async () => {
  const model = new SettingsController();
  model.start(createPreviewClient());
  await model.refresh();
  if (!model.draft) throw new Error("Settings did not load");
  model.draft.folders[0].exclusions = [" temp/** ", " "];
  await model.save();
  expect(model.status?.settings.folders[0].exclusions).toEqual(["temp/**"]);
  expect(model.dirty).toBe(false);
  expect(model.busy).toBeNull();
  model.destroy();
});

test("destroy ignores in-flight responses and removes late subscriptions", async () => {
  const client = createPreviewClient();
  const status = Promise.withResolvers(),
    subscription = Promise.withResolvers();
  let removed = false;
  const model = new SettingsController();
  model.start({ ...client, status: () => status.promise, onError: () => subscription.promise });
  model.destroy();
  status.resolve(await client.status());
  subscription.resolve(() => {
    removed = true;
  });
  await Promise.resolve();
  expect(model.status).toBeNull();
  expect(removed).toBe(true);
});

test("installing an update blocks settings, retries, navigation and polling", async () => {
  const client = createPreviewClient();
  const model = new SettingsController();
  model.start(client);
  await model.refresh();
  const before = await client.status();
  model.updateInstalling(true);
  if (!model.draft) throw new Error("Settings did not load");
  model.draft.theme = "dark";
  model.navigate("folders");
  model.discard();
  await model.save();
  await model.retry();
  expect(model.page).toBe("overview");
  expect(model.busy).toBe("update");
  expect((await client.status()).settings).toEqual(before.settings);
  model.updateInstalling(false);
  model.discard();
  expect(model.dirty).toBe(false);
  model.destroy();
});
