import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';

/**
 * The palette must be readable on every calibration.
 *
 * This is the guard for a defect that shipped in all four calibrations at
 * once: `--dim` was a single token carrying BOTH the micro-caps label tier
 * (8.5px uppercase at 0.22em tracking — pane context lines, plane caps, stat
 * labels, eyebrows, the crumb) AND 12-14px secondary prose. Measured against
 * `--void` it scored 4.52 on terra, 4.11 on amber, 3.19 on ember and 3.10 on
 * violet, so on three of the four every label on every screen failed WCAG AA,
 * and the one that passed did so at a size the 4.5 threshold does not cover.
 *
 * It was found by a reader saying the detail page was hard to read — not by
 * any gate. Nothing in the suite looked at colour.
 *
 * TESTED FROM THE STYLESHEET, not from a rendered page. A browser test can
 * only reach the colours some route happens to use today; parsing the token
 * file checks every calibration whether or not a screen currently exercises
 * it, and it runs in milliseconds so there is no reason to sample.
 *
 * The thresholds are the project's own, agreed rather than inherited:
 *   - micro caps (`--label`) >= 7.0. WCAG's 4.5 assumes ~16px body text; this
 *     tier is 10px uppercase at wide tracking, which is the least legible
 *     shape small text can take.
 *   - secondary prose (`--dim`) >= 4.5, held to a 5.5 floor for headroom so a
 *     later nudge to a tint or an opacity cannot silently cross the line.
 *   - `--beam` and `--hot` are value/emphasis text and must clear 4.5.
 */
const TOKENS = path.join(
  process.cwd(),
  '../../packages/holo/styles/tokens-holo.css',
);

type Rgb = [number, number, number];

function hexToRgb(hex: string): Rgb {
  const h = hex.replace('#', '');
  return [
    parseInt(h.slice(0, 2), 16),
    parseInt(h.slice(2, 4), 16),
    parseInt(h.slice(4, 6), 16),
  ];
}

function relativeLuminance([r, g, b]: Rgb): number {
  const f = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

export function contrast(a: Rgb, b: Rgb): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** Every calibration block in the token file, by the tokens it sets. */
function calibrations(): { name: string; tokens: Record<string, string> }[] {
  const css = fs.readFileSync(TOKENS, 'utf8');
  const out: { name: string; tokens: Record<string, string> }[] = [];
  // Each calibration declares beam/hot/dim/fringe/label on one line.
  const line =
    /--beam:\s*(#[0-9A-Fa-f]{6});\s*--hot:\s*(#[0-9A-Fa-f]{6});\s*--dim:\s*(#[0-9A-Fa-f]{6});\s*--fringe:\s*(#[0-9A-Fa-f]{6});\s*--label:\s*(#[0-9A-Fa-f]{6});/g;
  // Name each block by the nearest preceding selector or comment heading.
  let m: RegExpExecArray | null;
  let i = 0;
  while ((m = line.exec(css)) !== null) {
    const before = css.slice(0, m.index);
    const sel = before.match(/\[data-cal=['"]?([a-z]+)['"]?\][^{]*\{[^{}]*$/);
    out.push({
      name: sel ? sel[1] : `block-${i}`,
      tokens: {
        beam: m[1],
        hot: m[2],
        dim: m[3],
        fringe: m[4],
        label: m[5],
      },
    });
    i += 1;
  }
  return out;
}

const VOID = hexToRgb('#03060B');

describe('palette contrast', () => {
  const cals = calibrations();

  it('finds every calibration', () => {
    // A calibration that stops matching the parser would silently drop out of
    // every assertion below and the suite would still be green.
    expect(cals.length).toBeGreaterThanOrEqual(5); // :root default + 4 named
  });

  it('declares --label on every calibration', () => {
    for (const c of cals) {
      expect(c.tokens.label, c.name).toMatch(/^#[0-9A-Fa-f]{6}$/);
    }
  });

  it.each(['label', 'dim', 'beam', 'hot'])(
    '--%s clears its threshold on every calibration',
    (token) => {
      // label: the micro-caps tier. dim: secondary prose, held above its own
      // 4.5 requirement. beam/hot: values and emphasis.
      // `--label` is held at 7.4 against the VOID, not 7.0, because it is
      // almost never ON the void. Panels tint the background with beam, and
      // they STACK: a widget tile inside a pane inside the stage reaches ~6%,
      // which costs about 0.6. Measured at a 7.0 floor the placard came in at
      // 6.76 on a card and the tile eyebrows at 6.59. The headroom is the
      // panel, not padding.
      const floor: Record<string, number> = {
        label: 7.4,
        dim: 5.5,
        // 9.0, not 4.5. `--beam` is the VALUE tier and must stay clear of
        // `--label` at 7.2 — pyro's original #FF6B4A measured 7.20 and the
        // tier inverted the moment the label gained its panel headroom, which
        // the ordering assertion below caught. A value that is dimmer than its
        // own caption is not a hierarchy.
        beam: 9.0,
        hot: 4.5,
      };
      for (const c of cals) {
        const r = contrast(hexToRgb(c.tokens[token]), VOID);
        expect(
          r,
          `${c.name}: --${token} ${c.tokens[token]} is ${r.toFixed(2)}:1, needs ${floor[token]}`,
        ).toBeGreaterThanOrEqual(floor[token]);
      }
    },
  );

  it('keeps --label quieter than --beam', () => {
    // The whole point of the split is a readable label that still reads as a
    // label. If it reaches --beam, the tier collapses and a caption competes
    // with the figure it captions — which is what mixing toward --beam did on
    // ember and violet, and why these values mix toward --hot instead.
    for (const c of cals) {
      const label = contrast(hexToRgb(c.tokens.label), VOID);
      const beam = contrast(hexToRgb(c.tokens.beam), VOID);
      expect(label, `${c.name}: --label must stay under --beam`).toBeLessThan(
        beam,
      );
    }
  });

  it('keeps a real gap between --label and --beam, not just an ordering', () => {
    // The test above pins the ORDER; nothing pinned the DISTANCE, so a label
    // could creep to within a hair of the beam and still pass while the tier
    // it exists to create quietly disappeared.
    //
    // Measured directly between the two colours (not each against the void,
    // which is a weaker proxy):
    //
    //   terra 1.84   stanton 1.45   pyro 1.22   nyx 1.21
    //
    // pyro and nyx are the two whose `--beam` was lightened to clear 9:1
    // against the void where the originals only just cleared 7:1. Raising a
    // warm beam for legibility moves it TOWARD a label pinned at "7:1 and no
    // further", so those two are the tightest by construction and are the
    // reason this floor exists.
    //
    // The floor is today's worst case, so the current palette passes and any
    // further erosion fails. It is deliberately not aspirational: raising it
    // is a palette decision, and should be made by changing colours and then
    // this number, in that order.
    const FLOOR = 1.2;
    for (const c of cals) {
      const gap = contrast(hexToRgb(c.tokens.beam), hexToRgb(c.tokens.label));
      expect(
        gap,
        `${c.name}: --label and --beam are ${gap.toFixed(2)}:1 apart, under the ` +
          `${FLOOR}:1 floor — the caption tier has collapsed into the value tier`,
      ).toBeGreaterThanOrEqual(FLOOR);
    }
  });

  it('keeps --dim quieter than --label', () => {
    // Three tiers, in order: prose < label < value. Any inversion means a
    // caption is louder than the prose it introduces.
    for (const c of cals) {
      const dim = contrast(hexToRgb(c.tokens.dim), VOID);
      const label = contrast(hexToRgb(c.tokens.label), VOID);
      expect(dim, `${c.name}: --dim must stay under --label`).toBeLessThan(
        label,
      );
    }
  });
});

describe('token tier usage', () => {
  const STYLES = [
    path.join(process.cwd(), '../../packages/holo/styles/patterns-holo.css'),
    path.join(process.cwd(), '../../packages/holo/styles/additions.css'),
  ];

  it('never puts --dim on small or uppercase text', () => {
    // The rule the split encodes: anything under 12px, or uppercase at wide
    // tracking, belongs to `--label`. `--dim` is for prose at 12px and up.
    // Checked in the stylesheet because that is where the decision lives — a
    // rendered page only proves it for the elements that route happens to
    // mount.
    const offenders: string[] = [];
    for (const file of STYLES) {
      const css = fs.readFileSync(file, 'utf8');
      const rule = /([^{}]*)\{([^{}]*)\}/g;
      let m: RegExpExecArray | null;
      while ((m = rule.exec(css)) !== null) {
        const [, selector, body] = m;
        if (!body.includes('var(--dim)')) continue;
        const px = body.match(/font-size:\s*(\d+(?:\.\d+)?)px/);
        const small =
          body.includes('var(--fs-micro)') ||
          body.includes('var(--fs-xs)') ||
          (px !== null && parseFloat(px[1]) < 12);
        const caps = /text-transform:\s*uppercase/.test(body);
        if (small || caps) {
          offenders.push(
            `${path.basename(file)}: ${selector.trim().split('\n').pop()?.trim()}`,
          );
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it('never fades readable text with opacity', () => {
    // A token cannot see the opacity applied on top of it. `.hp-hint` used
    // `--label` — chosen to clear 7:1 — and then `opacity: .6`, so it rendered
    // at 3.12:1. Four more rules did the same: the core readout's detail line
    // (3.90), a plane's cap hint (3.11), the flatline hint (3.86) and the
    // steps marker (4.76). Every one passed a stylesheet reading of "uses the
    // right token".
    //
    // Recede with the COLOUR, which is measurable, never with opacity.
    //
    // Pseudo-elements are exempt: `.hp-core .n::before/::after` and the brand
    // hero's are chromatic-fringe layers drawn behind the real glyphs. They
    // duplicate text that is already present, are not in the accessibility
    // tree, and are decoration by construction.
    const offenders: string[] = [];
    for (const file of STYLES) {
      const css = fs.readFileSync(file, 'utf8');
      const rule = /([^{}]*)\{([^{}]*)\}/g;
      let m: RegExpExecArray | null;
      while ((m = rule.exec(css)) !== null) {
        const selector = m[1].trim().split('\n').pop()?.trim() ?? '';
        const body = m[2];
        if (!/(^|[^-])color:/.test(body)) continue;
        if (!/(?<!-)opacity:\s*0?\.\d+/.test(body)) continue;
        if (selector.includes('::before') || selector.includes('::after')) continue;
        // Decorative glyphs marked aria-hidden in the component.
        if (selector.includes('__caret')) continue;
        offenders.push(`${path.basename(file)}: ${selector}`);
      }
    }
    expect(offenders).toEqual([]);
  });
});
