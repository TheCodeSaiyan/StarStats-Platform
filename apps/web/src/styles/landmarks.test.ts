import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';

const SRC = path.join(process.cwd(), 'src');

function tsxFiles(dir: string, out: string[] = []): string[] {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) tsxFiles(p, out);
    else if (e.name.endsWith('.tsx')) out.push(p);
  }
  return out;
}

/** Strip block and line comments so a rule ABOUT `<main>` is not a use of it. */
function code(src: string): string {
  return src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
}

describe('main landmark', () => {
  it('is never rendered by a page, component or skeleton', () => {
    // The projection puts `role="main"` on `#hp-content`. Anything that ALSO
    // renders a `<main>` element gives the route two main landmarks, and picks
    // up globals.css's legacy `main { max-width: 720px }` column on the way —
    // which is why the offenders carried inline `maxWidth:'none'` to fight it.
    //
    // This is invisible to every other gate. Typecheck, lint, 878 unit tests
    // and 265 e2e all passed with `/orgs/[slug]` shipping a nested `<main>`;
    // it surfaced only in a Playwright failure snapshot, by accident.
    const offenders = tsxFiles(SRC)
      .filter((f) => !f.endsWith('.test.tsx'))
      .filter((f) => /<main[\s>]/.test(code(fs.readFileSync(f, 'utf8'))))
      .map((f) => path.relative(SRC, f));

    expect(offenders).toEqual([]);
  });

  it('is provided exactly once, by the projection', () => {
    // The counterpart: deleting the elements above must not have left the app
    // with NO main landmark. `Projection` supplies it on `#hp-content`, and
    // that is the only place it may be set — a page or shell that adds its own
    // recreates the duplicate this file exists to prevent.
    const projection = path.join(
      process.cwd(),
      '../../packages/holo/src/components/Projection.tsx',
    );
    expect(fs.readFileSync(projection, 'utf8')).toContain(
      'id="hp-content" role="main"',
    );

    // Comments explaining why a page does NOT set it are fine and several
    // pages carry one; a rendered attribute is not.
    const dupes = tsxFiles(SRC)
      .filter((f) => !f.endsWith('.test.tsx'))
      .filter((f) => /role="main"/.test(code(fs.readFileSync(f, 'utf8'))))
      .map((f) => path.relative(SRC, f).split(path.sep).join('/'));

    expect(dupes).toEqual([]);
  });
});
