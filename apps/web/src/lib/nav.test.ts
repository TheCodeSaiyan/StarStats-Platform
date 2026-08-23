import { describe, it, expect } from 'vitest';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { SITE_NAV, navFor, navSections } from './nav';

/**
 * The nav model had NO test until this file, which is how it managed to be
 * wrong twice during the projection port: `/journey` (a redirect stub) was
 * offered as a destination and bounced readers straight back to the page they
 * were on, and the public set shipped with four of its nine entries so a
 * signed-out visitor reading `/kb` in the projection was offered strictly less
 * than the same visitor in the flat shell.
 *
 * Both are filesystem facts, not opinions, so they are asserted against the
 * app router itself rather than against a second hand-maintained list — a list
 * that can drift is not a guard.
 */
const APP_DIR = join(process.cwd(), 'src', 'app');

/** The `page.tsx` backing a route, or null when nothing serves it. */
function pageFileFor(href: string): string | null {
  const segments = href.split('?')[0].split('#')[0].replace(/^\//, '');
  const candidate = join(APP_DIR, segments, 'page.tsx');
  return existsSync(candidate) ? candidate : null;
}

/**
 * A route whose whole job is to bounce elsewhere: it calls `redirect()` and
 * renders nothing. The system's rule is that a permanent redirect is never
 * surfaced as a destination — the same reason "OrgPlatform" is not offered.
 */
function isRedirectStub(file: string): boolean {
  const src = readFileSync(file, 'utf8');
  if (!/\bredirect\(/.test(src)) return false;
  // Strip comments first, so a JSDoc block mentioning a tag does not read as a
  // rendered element. Then look for a CLOSING or SELF-CLOSING tag rather than
  // an opening one: `<Foo` is indistinguishable from a TypeScript generic, and
  // the first version of this guard let `/devices` through because its
  // `Record<string, …>` parameter type looked like markup. `</` and `/>` never
  // appear in a type argument.
  const code = src
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^\s*\/\/.*$/gm, '');
  return !/(<\/[A-Za-z]|\/>)/.test(code);
}

describe('SITE_NAV', () => {
  it('offers only routes that exist', () => {
    const missing = SITE_NAV.filter((n) => pageFileFor(n.href) === null).map(
      (n) => `${n.label} → ${n.href}`,
    );
    expect(missing).toEqual([]);
  });

  it('never offers a redirect stub as a destination', () => {
    const stubs = SITE_NAV.filter((n) => {
      const file = pageFileFor(n.href);
      return file !== null && isRedirectStub(file);
    }).map((n) => `${n.label} → ${n.href}`);
    expect(stubs).toEqual([]);
  });

  it('has no duplicate ids or hrefs', () => {
    const ids = SITE_NAV.map((n) => n.id);
    const hrefs = SITE_NAV.map((n) => n.href);
    expect(new Set(ids).size).toBe(ids.length);
    expect(new Set(hrefs).size).toBe(hrefs.length);
  });

  it('routes pairing to the Emitter, and offers no Hangar destination', () => {
    // `/devices` was labelled "Hangar" and was the pairing page. It double-
    // booked the word — the hangar is the RSI fleet — and split the emitter's
    // lifecycle across two destinations. Pairing moved into `/downloads`.
    expect(SITE_NAV.map((n) => n.label)).not.toContain('Hangar');
    expect(SITE_NAV.map((n) => n.href)).not.toContain('/devices');
    const emitter = SITE_NAV.find((n) => n.id === 'downloads');
    expect(emitter).toMatchObject({ label: 'Emitter', href: '/downloads' });
  });
});

describe('navFor', () => {
  it('shows a signed-out visitor public entries only', () => {
    const out = navFor({ signedIn: false });
    expect(out.every((n) => n.access === 'public')).toBe(true);
    // Not just "the user entries are absent" — their LABELS must be absent.
    // Seeing "Records" or "Calibrate" tells an outsider what exists and
    // invites a bounce off a login wall.
    const labels = out.map((n) => n.label);
    for (const secret of ['Projection', 'Records', 'Calibrate', 'Sharing']) {
      expect(labels).not.toContain(secret);
    }
  });

  it('gives a signed-in reader every public and user entry', () => {
    const out = navFor({ signedIn: true }).map((n) => n.id);
    const expected = SITE_NAV.filter((n) => n.access !== 'admin').map(
      (n) => n.id,
    );
    expect(out).toEqual(expected);
  });

  it.each([
    ['no grants', [] as string[], false],
    ['a legacy cookie', undefined, false],
    ['an unrelated grant', ['supporter'], false],
    ['moderator', ['moderator'], true],
    ['admin', ['admin'], true],
  ])('shows the console to %s: %s', (_name, staffRoles, visible) => {
    const ids = navFor({ signedIn: true, staffRoles }).map((n) => n.id);
    expect(ids.includes('admin')).toBe(visible);
  });

  it('never shows the console to a signed-out visitor, grants or not', () => {
    // Belt and braces: a stale cookie carrying staffRoles with no session must
    // not light the operator group.
    const ids = navFor({ signedIn: false, staffRoles: ['admin'] }).map(
      (n) => n.id,
    );
    expect(ids).not.toContain('admin');
  });
});

describe('navSections', () => {
  it('groups by access and marks the active entry', () => {
    const sections = navSections({ signedIn: true }, 'downloads');
    expect(sections.map((s) => s.title)).toEqual(['Site', 'Your data']);
    const active = sections
      .flatMap((s) => s.items)
      .filter((i) => i.active)
      .map((i) => i.id);
    expect(active).toEqual(['downloads']);
  });

  it('omits the operator group entirely for a non-staff reader', () => {
    const titles = navSections({ signedIn: true }).map((s) => s.title);
    expect(titles).not.toContain('Operator');
  });
});
