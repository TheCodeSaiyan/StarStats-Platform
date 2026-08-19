import { defineConfig } from 'vitest/config';
import path from 'path';

export default defineConfig({
  resolve: {
    // `api.ts` imports 'server-only' to prevent accidental browser
    // bundle inclusion. In unit tests there is no Next.js runtime, so
    // we stub it out with an empty module — the same approach Next.js
    // recommends for Jest setups.
    alias: {
      'server-only': path.resolve(
        __dirname,
        'src/test-setup-stubs/server-only.ts',
      ),
      // Mirror tsconfig.json `paths` so component tests can use the
      // same `@/` import alias as the rest of the codebase.
      '@': path.resolve(__dirname, 'src'),
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test-setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
