import { plugin, Transpiler } from "bun";
import { compileModule } from "svelte/compiler";

// Exercise the actual rune controller with Svelte's compiler, outside a browser.
const typescript = new Transpiler({ loader: "ts" });
plugin({
  name: "svelte-rune-tests",
  setup(build) {
    build.onLoad({ filter: /\.svelte\.ts$/ }, async ({ path }) => {
      const source = typescript.transformSync(await Bun.file(path).text());
      return {
        contents: compileModule(source, { filename: path, generate: "client" }).js.code,
        loader: "js",
      };
    });
  },
});
