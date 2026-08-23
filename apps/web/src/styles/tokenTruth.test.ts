import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

/**
 * CSS custom-property tripwire (3.6).
 *
 * Styled-but-undefined tokens pass every other gate — typecheck, lint, RTL,
 * and e2e all stay green because nothing asserts computed style, so a
 * `var(--does-not-exist)` just renders as nothing (borderless card, invisible
 * selected-state, indistinguishable status). This test is the only thing that
 * catches it: every `var(--x)` used WITHOUT a fallback in the app source must
 * resolve to a token defined in the stylesheets.
 *
 * Fallback forms (`var(--x, y)`) are intentional forward-compat defaults and
 * are deliberately NOT required to be defined.
 */

// vitest runs from the package root (apps/web).
const SRC = join(process.cwd(), 'src');
// The canonical design tokens now live in the shared `design-tokens` package
// (consumed by both apps/web and apps/tray-ui). Include it so tokens defined
// there still count as defined by this tripwire.
const SHARED_TOKENS = join(process.cwd(), '..', '..', 'packages', 'design-tokens');
/**
 * The PROJECTION's tokens — the beam vocabulary (`--hot`, `--beam`, `--dim`,
 * `--void`, the `--bR/--bG/--bB` channels) — live in the `holo` package, which
 * the app now depends on for every surface.
 *
 * Added when a component drawn in the beam tripped this test: the tokens were
 * real and defined, just defined somewhere this tripwire had never been told
 * about. Leaving it out would have pushed callers into writing fallbacks for
 * tokens that do not need them, which is exactly the noise this test exists to
 * prevent.
 *
 * NOTE the scoping difference, because it matters for what "defined" means
 * here: these are declared on `[data-cal]`, not `:root`. A component that uses
 * them OUTSIDE a projection stage resolves them to nothing. This test proves
 * they exist; it does not prove the element is inside a stage.
 */
const HOLO_TOKENS = join(process.cwd(), '..', '..', 'packages', 'holo', 'styles');

function walk(dir: string, exts: string[]): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      out.push(...walk(p, exts));
    } else if (
      exts.some((e) => name.endsWith(e)) &&
      !name.endsWith('.test.ts') &&
      !name.endsWith('.test.tsx')
    ) {
      out.push(p);
    }
  }
  return out;
}

describe('CSS custom-property truth', () => {
  it('every var(--x) used without a fallback is defined in the stylesheets', () => {
    // Defined tokens (any position on a line) across every web stylesheet.
    const defined = new Set<string>();
    for (const f of [
      ...walk(SRC, ['.css']),
      ...walk(SHARED_TOKENS, ['.css']),
      ...walk(HOLO_TOKENS, ['.css']),
    ]) {
      for (const m of readFileSync(f, 'utf8').matchAll(/(--[a-z0-9-]+)\s*:/gi)) {
        defined.add(m[1]);
      }
    }

    // Used tokens with NO fallback comma — `var(--x)` immediately closed.
    const missing = new Map<string, string>();
    for (const f of walk(SRC, ['.tsx', '.ts'])) {
      for (const m of readFileSync(f, 'utf8').matchAll(
        /var\(\s*(--[a-z0-9-]+)\s*\)/gi,
      )) {
        if (!defined.has(m[1]) && !missing.has(m[1])) {
          missing.set(m[1], f.replace(process.cwd(), '.'));
        }
      }
    }

    expect(
      Array.from(missing, ([t, f]) => `${t} used at ${f}`),
    ).toEqual([]);
  });
});
