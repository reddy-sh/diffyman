import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  resolve: {
    // @diffyman/react and @diffyman/wasm are linked from this repository; use
    // one copy of React (and of the engine glue) for the whole app.
    dedupe: ["react", "react-dom", "@diffyman/wasm"],
  },
  // The diff worker is an ES module worker (all current browsers).
  worker: { format: "es" },
  build: {
    outDir: "dist",
    target: "es2022",
  },
});
