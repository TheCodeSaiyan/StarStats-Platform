import { FlatCompat } from '@eslint/eslintrc';

const compat = new FlatCompat({
  baseDirectory: import.meta.dirname,
});

const eslintConfig = [
  ...compat.extends('next/core-web-vitals', 'next/typescript'),
  {
    // E2E tests live outside src and use Playwright globals + Node
    // built-ins; lint them in isolation rather than through the
    // browser-oriented next/typescript preset.
    ignores: [
      '.next/**',
      'node_modules/**',
      'e2e/**',
      'playwright-report/**',
      'test-results/**',
    ],
  },
];

export default eslintConfig;
