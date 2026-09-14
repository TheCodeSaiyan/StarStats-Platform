import nextCoreWebVitals from "eslint-config-next/core-web-vitals";
import nextTypescript from "eslint-config-next/typescript";

const eslintConfig = [...nextCoreWebVitals, ...nextTypescript, {
  // eslint-config-next 16 ships React Compiler's hook rules. Two of them
  // flag 21 existing sites: `set-state-in-effect` (14 — state derived in
  // an effect that should be render-time) and `purity` (7 — Date.now() /
  // Math.random() during render). They are right, and each fix changes
  // when a component re-renders, so they are a refactor, not a dependency
  // bump. Held at `warn` so the upgrade lands and the count stays on every
  // lint run; lift to `error` as the sites are fixed.
  rules: {
    'react-hooks/set-state-in-effect': 'warn',
    'react-hooks/purity': 'warn',
  },
}, {
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
}];

export default eslintConfig;
