import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    globals: true,
    testTimeout: 10000,
    exclude: [
      "**/node_modules/**",
      ".worktrees/**",
      ".claude/worktrees/**",
    ],
    env: {
      PATH: process.env.PATH ?? "",
    },
  },
});
