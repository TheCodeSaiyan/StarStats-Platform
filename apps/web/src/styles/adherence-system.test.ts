import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';

/**
 * Design-system adherence, checked where the decisions live.
 *
 * Three faults this file exists for, all of which pass every other gate:
 *
 *   1. INLINE TYPE. 26 `React.CSSProperties` blocks across 13 files set
 *      `fontWeight: 600` with NEGATIVE letter-spacing — the flat voice — while
 *      the beam voice is thin and positively tracked. Those headings rendered
 *      in the old system's accent inside the new frame, and no stylesheet could
 *      correct them: CSS cannot override an inline declaration.
 *   2. ORPHAN CLASSES. `web-testing.md` records it plainly — "className'd
 *      components with no CSS pass every gate", because RTL and Playwright
 *      assert presence, never computed style.
 *   3. RAW COLOUR. A hex outside the token file is a colour that no
 *      calibration can reach, so it stays cyan on the amber beam.
 */
const SRC = path.join(process.cwd(), 'src');
const STYLES = [
  path.join(process.cwd(), '../../packages/holo/styles/patterns-holo.css'),
  path.join(process.cwd(), '../../packages/holo/styles/additions.css'),
  path.join(process.cwd(), 'src/styles/projection-shell.css'),
];

/** Newline splitter, defined once so a regex literal never has to survive
 *  being written into this file by a script. */
const SPLIT_NL = new RegExp(String.fromCharCode(92) + 'r?' + String.fromCharCode(92) + 'n');

/** CSS with block comments removed — a rule's prose is not a rule. */
function code(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '');
}

function walk(dir: string, test: (f: string) => boolean, out: string[] = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, test, out);
    else if (test(e.name)) out.push(p);
  }
  return out;
}

const tsx = () =>
  walk(SRC, (f) => f.endsWith('.tsx') && !f.includes('.test.'));

describe('design-system adherence', () => {
  it('sets no type or colour in an inline style object', () => {
    // Layout inline (flex, grid, gap, width) is fine and sometimes necessary —
    // a data-driven bar width cannot live in a stylesheet. TYPE and COLOUR are
    // the system's vocabulary and belong to it.
    const offenders: string[] = [];
    for (const file of tsx()) {
      const src = fs.readFileSync(file, 'utf8');
      const block = /const (\w+): React\.CSSProperties = \{([\s\S]*?)\n\};/g;
      let m: RegExpExecArray | null;
      while ((m = block.exec(src)) !== null) {
        if (/fontSize|fontWeight|letterSpacing|color:|background/.test(m[2])) {
          offenders.push(`${path.relative(SRC, file)}: ${m[1]}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it('backs every hp- class with a rule', () => {
    // Collected from className attributes and from bare string literals, since
    // several components build a class list in a variable.
    const used = new Set<string>();
    for (const file of [
      ...tsx(),
      ...walk(
        path.join(process.cwd(), '../../packages/holo/src'),
        (f) => f.endsWith('.tsx'),
      ),
    ]) {
      const src = fs.readFileSync(file, 'utf8');
      for (const m of src.matchAll(/['"`]([^'"`]*\bhp-[a-z0-9_-]+[^'"`]*)['"`]/g)) {
        for (const c of m[1].split(/\s+/)) {
          // Template-literal modifiers (`hp-slot--${slot}`) resolve at runtime;
          // their concrete forms are checked by the rules that define them.
          if (c.startsWith('hp-') && !c.includes('$') && !c.includes('{')) {
            used.add(c);
          }
        }
      }
    }
    const defined = new Set<string>();
    for (const f of STYLES) {
      for (const m of code(fs.readFileSync(f, 'utf8')).matchAll(
        /\.(hp-[a-z0-9_-]+)/g,
      )) {
        defined.add(m[1]);
      }
    }
    // `hp-slot--head` and friends are DOM hooks with no styling of their own,
    // and `hp-content` is an id. Both are legitimate; neither is a class with
    // a missing rule.
    const HOOKS = /^hp-(slot--|content$)/;
    const orphans = [...used]
      .filter((c) => !defined.has(c) && !HOOKS.test(c))
      .sort();
    expect(orphans).toEqual([]);
  });

  it('uses no raw colour outside the token file', () => {
    // The exceptions are named, not a blanket allowance:
    //   - the sanctioned series palette, which is a documented exception to the
    //     one-colour rule and cannot be derived from a single beam;
    //   - the QR code's white, which a camera has to read.
    const offenders: string[] = [];
    for (const f of STYLES) {
      const lines = code(fs.readFileSync(f, 'utf8')).split(/\r?\n/);
      lines.forEach((line, i) => {
        if (!/#[0-9A-Fa-f]{3,8}\b/.test(line)) return;

        if (line.includes('--hp-series-')) return;
        if (/background:\s*#ffffff/.test(line)) return;
        offenders.push(`${path.basename(f)}:${i + 1} ${line.trim().slice(0, 60)}`);
      });
    }
    expect(offenders).toEqual([]);
  });

  it('draws no rounded boxes', () => {
    // The system has square corners. The only curves are 50% dots and the 1-2px
    // optical rounding a hairline needs to not look chipped.
    const offenders: string[] = [];
    for (const f of STYLES) {
      for (const m of code(fs.readFileSync(f, 'utf8')).matchAll(
        /border-radius:\s*([^;]+);/g,
      )) {
        const v = m[1].trim();
        if (v === '0' || v === '50%' || /^[0-2]px$/.test(v)) continue;
        offenders.push(`${path.basename(f)}: ${v}`);
      }
    }
    expect(offenders).toEqual([]);
  });

  it('states both axes on every scroll container', () => {
    /**
     * Per CSS overflow, when ONE axis is set to something other than
     * `visible`, the other axis's `visible` COMPUTES to `auto`. So a rule
     * that says only `overflow-y: auto` has quietly asked for a horizontal
     * scroll container too, and any hairline overflow — a glow, a sub-pixel
     * rounding, one wide chip — paints a bar the author never wanted.
     *
     * `.hp-pane` and the nav menu both did this; `.hp-settings` already got
     * it right, which is what made the omission look deliberate.
     *
     * The rule: if you set one axis, say what the other one is.
     */
    const offenders: string[] = [];
    for (const f of STYLES) {
      const css = code(fs.readFileSync(f, 'utf8'));
      const rule = /([^{}]*)\{([^{}]*)\}/g;
      let m: RegExpExecArray | null;
      while ((m = rule.exec(css)) !== null) {
        const [, selector, body] = m;
        const hasY = /overflow-y:\s*(auto|scroll|hidden|clip)/.test(body);
        const hasX = /overflow-x:\s*(auto|scroll|hidden|clip)/.test(body);
        // `overflow:` shorthand sets both, so it is already explicit.
        if (/(^|[^-])overflow:\s*/.test(body)) continue;
        if (hasY === hasX) continue;
        const sel = selector.trim().split(SPLIT_NL).pop()?.trim();
        offenders.push(
          `${path.basename(f)}: ${sel} sets only overflow-${hasY ? 'y' : 'x'}`,
        );
      }
    }
    expect(offenders).toEqual([]);
  });
});
