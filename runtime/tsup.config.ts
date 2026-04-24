import { defineConfig } from "tsup";

export default defineConfig({
  entry: {
    index: "src/index.ts",
    "middleware/index": "src/middleware/index.ts",
    "proxy/index": "src/proxy/index.ts",
    cli: "src/cli.ts",
  },
  format: ["esm"],
  dts: true,
  sourcemap: true,
  clean: true,
  target: "node18",
  splitting: true,
  treeshake: true,
});
