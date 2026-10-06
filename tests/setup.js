import { plugin, Transpiler } from "bun";
import { compile, compileModule } from "svelte/compiler";

// Exercise the actual rune controller with Svelte's compiler, outside a browser.
const typescript = new Transpiler({ loader: "ts" });
plugin({
  name: "svelte-rune-tests",
  setup(build) {
    build.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({
      contents: compile(await Bun.file(path).text(), {
        filename: path,
        generate: "server",
        css: "external",
      }).js.code,
      loader: "js",
    }));
    build.onLoad({ filter: /\.svelte\.ts$/ }, async ({ path }) => {
      const source = typescript.transformSync(await Bun.file(path).text());
      return {
        contents: compileModule(source, { filename: path, generate: "client" }).js.code,
        loader: "js",
      };
    });
  },
});
