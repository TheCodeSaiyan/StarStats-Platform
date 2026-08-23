import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

/**
 * Design-system adherence, checked mechanically.
 *
 * The system ships `_adherence.oxlintrc.json`, an oxlint config declaring its
 * component contracts and its hard visual rules. This is the subset of those
 * rules that can be checked without adding a linter to the toolchain, and it
 * exists because the same two violations kept reappearing across ~45 pages and
 * were what made ported screens read as "the old one in a new box":
 *
 *   1. A ROUNDED BOX. The system has no radius on any container or control.
 *      Round DOTS are fine — the pattern layer uses `border-radius: 50%` ten
 *      times for chip and status dots — and 1–2px hairline corners are its own.
 *      Anything else is the flat system leaking through.
 *   2. A RAW HEX COLOUR outside a `var()` fallback. A literal cannot follow the
 *      beam, so it stays its original hue when a reader recalibrates. This was
 *      live on the one element whose whole job was comparison.
 *
 * INLINE STYLES ARE WHY THIS IS A TEST AND NOT A STYLESHEET RULE. CSS cannot
 * override an inline declaration, so the `.hp-stage` primitive redraw is
 * powerless against `style={{ borderRadius: 8 }}` — the only place to catch it
 * is here.
 */
const SRC = join(process.cwd(), 'src');

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...walk(p));
    else if (p.endsWith('.tsx') && !p.includes('.test.')) out.push(p);
  }
  return out;
}

/** `50%` (a dot) and 0–2px (the system's own hairline corners) are allowed. */
const ROUNDED = /borderRadius: (?:'(?!50%|0)[^']*'|(?:999|[3-9]|[1-9]\d+)\b(?!%))/g;

/** A hex literal that is NOT the fallback arm of a `var()`. */
const RAW_HEX = /(?<!var\([^)]{0,80})#[0-9A-Fa-f]{6}\b/g;

describe('design-system adherence', () => {
  it('no rounded boxes in inline styles', () => {
    const hits: string[] = [];
    for (const f of walk(SRC)) {
      const src = readFileSync(f, 'utf8');
      for (const m of src.matchAll(ROUNDED)) {
        hits.push(`${f.replace(SRC, 'src')}: ${m[0]}`);
      }
    }
    expect(hits).toEqual([]);
  });

  it('no raw hex colours outside a var() fallback', () => {
    const hits: string[] = [];
    for (const f of walk(SRC)) {
      const src = readFileSync(f, 'utf8')
        // Comments explain what was removed and name the old values; they are
        // documentation, not colour.
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/^\s*(?:\/\/|\*).*$/gm, '');
      for (const m of src.matchAll(RAW_HEX)) {
        hits.push(`${f.replace(SRC, 'src')}: ${m[0]}`);
      }
    }
    expect(hits).toEqual([]);
  });
});
