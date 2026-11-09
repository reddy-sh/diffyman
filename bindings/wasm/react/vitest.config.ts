import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["test/**/*.test.{ts,tsx}"],
    environment: "node",
    // The corpus test runs every case through the real WASM engine.
    testTimeout: 60_000,
  },
});
