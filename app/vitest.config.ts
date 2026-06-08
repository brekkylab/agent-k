import { defineConfig, mergeConfig, configDefaults } from 'vitest/config';
import viteConfig from './vite.config';

// Inherit the app's Vite config (resolve.alias '@', plugins) so unit/component
// tests resolve imports exactly like the app does, then layer on Vitest-only
// options. Kept separate from vite.config.ts to avoid the Vite 8 vs Vitest's
// bundled-Vite type skew that breaks a `test` block placed directly in the
// Vite config.
export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      // Playwright specs under tests/e2e/ are driven by Playwright, not Vitest.
      // Loading them under Vitest throws "Playwright Test did not expect
      // test.describe() to be called here". Vitest = unit/component tests only.
      exclude: [...configDefaults.exclude, 'tests/e2e/**'],
    },
  }),
);
