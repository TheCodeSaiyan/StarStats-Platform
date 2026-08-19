import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    // Pure data/loader package — no DOM needed.
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
