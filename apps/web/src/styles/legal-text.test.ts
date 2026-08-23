import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import baseline from './__fixtures__/legal-prose.json';

/**
 * The legal text is not the port's to touch.
 *
 * Terms, the privacy statement, the trust page and the About attribution are
 * legal copy. A redesign may reframe them — put them in a pane, give them a
 * measure, add an index — but it may not reword them, and a diff that quietly
 * drops a clause is exactly the kind of change a visual review does not catch.
 *
 * So this compares the VISIBLE TEXT of each legal route against a COMMITTED
 * BASELINE. It ignores markup, class names and whitespace entirely: the port
 * is expected to change all of those and nothing else.
 *
 * THE BASELINE IS A FILE, NOT `git show HEAD`. It was the latter while the port
 * was uncommitted work, and that version quietly stops testing anything the
 * moment the port lands: HEAD becomes the ported file, so the test compares it
 * against itself and passes on any wording. `__fixtures__/legal-prose.json` was
 * generated from the pre-port HEAD and is the record.
 *
 * If this fails after a deliberate legal change, regenerate the fixture IN THE
 * SAME COMMIT as the wording change, so the diff shows a human decided to
 * change legal copy. Do not loosen the test, and do not regenerate it to make
 * an unrelated commit green.
 */
const ROUTES = [
  'apps/web/src/app/terms/page.tsx',
  'apps/web/src/app/privacy/page.tsx',
  'apps/web/src/app/trust/page.tsx',
  'apps/web/src/app/about/page.tsx',
];

const REPO = join(process.cwd(), '..', '..');

/** Visible prose: text between tags, long enough not to be a class or an id. */
function prose(src: string): string[] {
  return [...src.matchAll(/>([^<>{}]{25,})</g)]
    .map((m) => m[1].replace(/\s+/g, ' ').trim())
    .filter(Boolean)
    .sort();
}

describe('legal text', () => {
  for (const path of ROUTES) {
    it(`${path.split('/').slice(-2)[0]} is unaltered`, () => {
      const recorded = (baseline as Record<string, string[]>)[path];
      // A route missing from the baseline is a silent pass, so say so.
      expect(recorded, `${path} has no recorded baseline`).toBeDefined();
      expect(recorded.length).toBeGreaterThan(20);
      const now = readFileSync(join(REPO, path), 'utf8');
      expect(prose(now)).toEqual(recorded);
    });
  }

  it('the site legal plate offers a way to read the full text', () => {
    // The plate is a summary of a longer position and appears on every
    // surface. A trademark and data-source notice that cannot be followed to
    // the documents behind it asks a reader to take it on faith.
    const plate = readFileSync(
      join(process.cwd(), 'src/components/projection/SiteLegalPlate.tsx'),
      'utf8',
    );
    expect(plate).toContain('Read the full terms');
    expect(plate).toContain("'/terms'");
  });

  it('every legal document is reachable from every other', () => {
    const index = readFileSync(
      join(process.cwd(), 'src/components/projection/LegalIndex.tsx'),
      'utf8',
    );
    for (const href of ['/terms', '/privacy', '/trust', '/donate']) {
      expect(index).toContain(`'${href}'`);
    }
  });
});
