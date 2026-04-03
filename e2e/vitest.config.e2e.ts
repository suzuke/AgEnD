import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    globals: true,
    testTimeout: 120_000,
    hookTimeout: 180_000,
    include: ["e2e/tests/**/*.test.ts"],
    exclude: ["**/node_modules/**"],
    pool: "forks",
    maxConcurrency: 1,
    sequence: { shuffle: false },
    env: {
      PATH: process.env.PATH ?? "",
    },
  },
});
