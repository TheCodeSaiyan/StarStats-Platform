import nextCoreWebVitals from "eslint-config-next/core-web-vitals";
import nextTypescript from "eslint-config-next/typescript";

const eslintConfig = [...nextCoreWebVitals, ...nextTypescript, {
  // eslint-config-next 16 ships React Compiler's hook rules. When it landed
  // they flagged 26 sites and were held at `warn` so the upgrade could merge;
  // the sites were then fixed (#101) and the rules raised to `error`, where
  // they stay. Two things about them worth knowing before touching either:
  //
  //   - `purity` cannot tell a server component from a client one. Seven
  //     server components call `Date.now()` during render, which is correct
  //     there (once per request, never re-rendered), and each carries a
  //     one-line disable saying so. A wrapper function would only hide the
  //     call from the analyser; the disables keep the rule meaningful for
  //     client code.
  //   - `set-state-in-effect` also flags a state write routed through a
  //     callback from a layout effect, so "measure the DOM, store the
  //     position" is out even in useLayoutEffect. The popovers write the
  //     measured position to the element's style instead, which is also one
  //     render fewer per open.
  rules: {
    'react-hooks/set-state-in-effect': 'error',
    'react-hooks/purity': 'error',
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
