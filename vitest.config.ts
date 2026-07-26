import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  // Svelte 5 ships separate browser and SSR entrypoints. Without the browser
  // condition, Vite resolves the SSR build and mounting a component in a test
  // throws `lifecycle_function_unavailable`.
  resolve: { conditions: ["browser"] },
  test: {
    include: ["src/**/*.test.ts"],
    environment: "jsdom",
  },
});
