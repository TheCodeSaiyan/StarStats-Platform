# Redesign — overnight review

Work done on `feat/redesign` overnight 2026-08-23 → 24, from one instruction:
sweep the port for colour/contrast, design-system adherence, accessibility
beyond colour, and spec fidelity; ground everything in measurement; note any
place I improved on the spec.

Every number below was measured, not estimated. Where a fix is a departure from
the kit it is marked **DEPARTS** with the reason.

---

## Phase 1 — colour and contrast

### What started it

You said the detail page was hard to read. It was, and the cause was systemic
rather than local.

`--dim` was one token doing two incompatible jobs: the **micro-caps label
tier** (8.5px uppercase at 0.22em tracking — pane context lines, plane caps,
stat labels, eyebrows, the crumb) *and* 12–14px secondary prose. Measured
against `--void`:

| calibration | `--dim` before | ratio | verdict |
|---|---|---|---|
| terra | `#3E7F96` | 4.52 | scrapes AA, at a size AA does not cover |
| stanton | `#97662E` | 4.11 | **fails AA** |
| pyro | `#9A4632` | 3.19 | **fails badly** |
| nyx | `#6B4E9E` | 3.10 | **fails badly** |

So on three of four calibrations every label on every screen failed WCAG AA.
Nothing in the suite looked at colour, so nothing caught it.

### The amendment

A third tier. `--dim` stays secondary prose; **`--label`** is new and carries
the micro-caps tier. Values are each calibration's `--dim` mixed toward its own
`--hot`, far enough to clear the target and no further.

| calibration | `--dim` | `--label` | `--beam` |
|---|---|---|---|
| terra | `#528EA3` 5.56 | `#73A6B7` 7.60 | `#7FE4FF` 13.96 |
| stanton | `#A97E4C` 5.57 | `#BC986C` 7.56 | `#FFAE3B` 10.99 |
| pyro | `#B77564` 5.56 | `#C88F81` 7.46 | `#FF8E74` 9.06 |
| nyx | `#927BBA` 5.57 | `#A894CA` 7.51 | `#C19BFF` 9.08 |

Agreed thresholds: **7.0 for anything under 12px** (WCAG's 4.5 assumes ~16px
body text), **4.5 otherwise**, with `--dim` held at a 5.5 floor for headroom.

Mixing toward `--hot` rather than `--beam` was deliberate: toward `--beam` also
reaches 7:1, but on pyro and nyx it lands within a hair of `--beam` itself and
the tier collapses — a caption must stay quieter than the figure it captions.

**DEPARTS (1): `--beam` raised on pyro and nyx.** `#FF6B4A` measured 7.20 and
`#B78BFF` 7.88. Once `--label` gained its panel headroom, pyro's value tier was
*dimmer than its own caption*. Both now clear 9.0, so the three tiers have real
separation. The matching `--bR/--bG/--bB` triplets were updated with them —
they feed every tint and glow in the system and would otherwise drift from the
beam they are named for.

**DEPARTS (2): type floor raised.** `--fs-micro` 8.5px → **10px**, tracking
eased 0.22em → 0.18em on that tier, and `--fs-sm` 11.5px → **12px**. Contrast
was only half the problem; stroke weight was the other half, and 8.5px
uppercase at wide tracking is the least legible shape small text can take.
`--fs-sm` at 11.5px also sat on the wrong side of the project's own 12px line,
so all 20 rules using it silently needed 7:1 while carrying a 5.5:1 token.

### Faults found while measuring

Each of these was invisible to a stylesheet reading of "uses the right token".

1. **`opacity` defeats the token contract.** Five rules picked a compliant
   token then faded it. `.hp-hint` used `--label` (7.4) with `opacity: .6` and
   rendered at **3.12**. Also `.hp-core .d` 3.90, `.hp-plane .cap i` 3.11,
   `.hp-nosig .h` 3.86, `.hp-steps li::before` 4.76. All now recede with the
   colour instead. Pseudo-element fringe layers keep their opacity — they
   duplicate text already present and are not in the accessibility tree.

2. **An inline `opacity: 0.8` in `BeamInput.tsx`** on the field hint — 5.01 on
   a 10px line, and unreachable from any stylesheet (CSS cannot override an
   inline declaration). Now a class.

3. **The semantic aliases never applied.** `tokens-holo.css` maps `--fg-dim`
   and friends onto the beam in its `:root` block; `design-tokens/starstats-tokens.css`
   sets the same names to the flat palette. Both match `html`, so load order
   decided it — and the flat palette won. Measured inside a projection,
   `--fg-dim` resolved to `#717F86`, not `var(--dim)`. Every flat component
   rendering through the bridge drew in flat colours with the beam sitting
   unused beside it. Now declared on `.ss-projection-root`, where custom-property
   inheritance settles it without a specificity fight.

4. **The bridge was scoped to `.hp-stage`.** `/u/[handle]` docks its pane
   *below* the volume — necessarily, because `.hp-pane` is invisible outside
   detail mode — which puts it outside `.hp-stage` and outside all 38 bridge
   rules. Re-scoped to `.ss-projection-root`, same specificity, strictly wider.

5. **The ground was the wrong colour.** `body { background: var(--bg) }`
   resolves at the body, outside the projection root, so every projection page
   sat on the flat `#0B1014` while its palette was designed against `#03060B`.
   Cost ~0.8 of contrast on every label over a tinted panel. Fixed with
   `body:has(.ss-projection-root)`.

6. **Decorative overlays intercepted the pointer.** `.hp-plane::before/::after`
   are 11px corner brackets with no `pointer-events: none`, so they could eat a
   click anywhere they overlapped a control — and pseudo-elements report as
   their originating element, which is why the failure read as "the tile
   intercepts pointer events". Six decorative pseudo-elements fixed; underlines
   drawn on a control's own box were left alone, since the pointer is already
   on the control there.

7. **A tile body collapsing to nothing.** Widget row heights are tuned so a
   tile "fits its summary content EXACTLY" — zero slack — so the 10px micro
   tier pushed a compact tile's `.hud-tile__body` to **clientHeight 0** against
   16px of content. The body and every control in it were in the DOM and
   clipped out of existence. `--hud-row` 22 → 24px inside the projection (the
   same ~9% the type grew, so every widget keeps its tuned proportion), plus a
   one-line `min-height` so a body can never collapse again. Row *counts* were
   deliberately not touched: they are persisted in every reader's saved layout.

### Guards added

- `src/styles/palette-contrast.test.ts` — parses the token file: every
  calibration clears its tier floor, tier ordering `dim < label < beam` holds,
  no `--dim` on small or uppercase text, no opacity on readable text. Runs in
  milliseconds, checks every calibration whether or not a route exercises it.
  Verified failing against the old values.
- `e2e/contrast.spec.ts` — 16 surfaces × 4 calibrations, composited
  backgrounds, `aria-hidden` excluded, no per-selector skip list. Verified
  failing against the wrong ground.

Both were built after a first attempt at the harness reported two elements at
**1.00:1** — "invisible text". They were not: `.ss-card` is a 3.5% beam tint and
the harness read it as solid beam. The measurement was wrong, not the page. The
compositing note in the spec exists so the next person does not chase it.

---

## Phase 2 — design-system adherence

### Inline type: the last pocket the port could not reach

26 `React.CSSProperties` blocks across 13 files still set **type**, and always
in the flat voice — `fontWeight: 600` with **negative** letter-spacing, where
the beam voice is thin and positively tracked. Those headings rendered in the
old system's accent inside the new frame, and no stylesheet could correct them
because CSS cannot override an inline declaration.

Files: `orgs/page`, `orgs/new`, `contracts/page`, `contracts/[canonicalId]`,
`admin/settings`, `admin/users/[id]` (+ its Delete and Restriction panels),
`privacy`, `terms`, `settings/widget-sharing`, `InferenceRuleForm`,
`ShipMatrixGallery`.

All 26 are now shared classes — `.hp-pagetitle`, `.hp-sectiontitle`,
`.hp-kvlabel`, `.hp-kvvalue`, `.hp-fine`, `.hp-code`, plus four specific ones —
so the same role looks the same on every page, which is the point of having a
system. The legal pages' `codeStyle` became `.hp-code`; **no legal text
changed**, and `legal-text.test.ts` still passes against its committed baseline.

### Clean on inspection

- **Raw colour**: none outside the token file. The series palette is the
  sanctioned exception you approved; the QR code's white is documented and a
  camera has to read it.
- **Rounded boxes**: every `border-radius` is `0`, `50%` (dots) or 1–2px
  optical rounding on a hairline.
- **Orphan classes**: three found and given real rules (`.hp-authsteps`,
  `.hp-recindex`, `.hp-legalindex` — index strips that were classNames with no
  CSS at all). `hp-slot--*` are DOM hooks with no styling by design and
  `hp-content` is an id; both are legitimate and are excluded by name.

`src/styles/adherence-system.test.ts` guards all four, verified failing against
a reintroduced inline block.

---

## Phase 3 — accessibility beyond colour

`e2e/a11y-focus.spec.ts` operates the keyboard rather than inspecting markup:
8 routes, 22 tab stops each.

- **Every tab stop is visible and shows focus.** Measured as a computed-style
  fingerprint (outline, box-shadow, border, background, colour, underline)
  taken with and without focus on the same element — so "there is a
  `:focus-visible` rule somewhere" is not enough; it has to take effect on that
  element. Verified failing with the focus rules stripped.
- **Every control has an accessible name.** Icon-only buttons included.
- **No invisible tab stops.** A control clipped to zero size still takes a stop
  and a reader tabs into nothing.

**Result: clean on the first run.** The single failure was `<nextjs-portal>`,
the dev-mode error overlay, which is not in the production bundle — excluded by
name with the reason, not because it was inconvenient.

---

## Phase 4 — spec fidelity, re-read

### Method, and a correction to it

I first compared each kit screen's **component vocabulary** against its route's
and got a long list of apparent gaps — `/contracts` "uses no system components",
auth "does not use BeamInput". Both were false. `/contracts` uses the system's
CSS classes (`hp-plane flat`, `hp-catchip`, `hp-kvlabel`) rather than the React
components, which renders identically, and auth's `.ss-input` is redrawn by the
bridge into the same lit underline `BeamInput` produces. **Using the class is
equivalent to using the component**, so a JSX-name scan measures nothing.

The measure that does work is what the browser paints. `e2e/idiom.spec.ts`
walks every element inside a projection on 8 surfaces and checks computed
style: no rounded corners beyond a 50% dot or 1–2px optical rounding, no opaque
fills. Exemptions are named individually with the reason — the void ground, the
skip link, a sticky pane header, data marks whose fill IS the value, the QR's
white — so the list cannot quietly become a way to pass.

### What it found

1. **The activity heatmap ignored the calibration entirely.** `.ss-heatcell`
   colours come from `--grid-1..4`, and those were missing from the alias block
   I added in Phase 1 — so every heatmap rendered in the flat teal `#4FB8A1` on
   all four beams. The most prominent element on a profile, drawn in the old
   system's accent. Now aliased onto the beam ramp.
2. **Rounded corners on the flat chip and the heat cell** — 4px and 3px. A grid
   of rounded cells reads as the previous design however the colours are set.
3. **The range selector filled its selected option** with `var(--accent)` and
   void-coloured text — the flat "filled pill", the one idiom the projection
   does not have. Inline-styled in `RangeBar.tsx`, so no stylesheet could reach
   it. Now `.hp-rangebar__opt`, marking current with a lit edge like every other
   current control in the system.
4. One more `.hp-stage`-scoped rule (`select option`) re-scoped to the
   projection root, for the same reason as the 38 in Phase 1.

### Where the port genuinely departs from the kit, and why

Recorded here rather than fixed, because each is a considered decision:

- **`/u/[handle]` is a volume with a docked pane**, not the kit's single stage.
  `.hp-pane` is `opacity: 0` outside detail mode and this screen has no in-page
  lens to open, so a pane inside the volume renders invisible and inert.
- **The public ring shows event types, not lenses.** The kit gives one equal
  segment per published lens; equal segments draw a distribution that does not
  exist.
- **Docs, legal and marketing routes are prose**, and use prose classes through
  the bridge rather than panes and planes. That is the right shape for reading
  copy and is not a fidelity gap.
- **Admin is 94% flat classes by count**, entirely through the bridge. It
  passes both the contrast and idiom sweeps, so it renders correctly; it is
  simply not written in the system's own components. Converting ~20 admin pages
  is real work with no visible outcome, so it is listed, not done.

---

## Phase 5 — verification

Final gate on the branch: typecheck, lint (17 pre-existing warnings, none new),
production build, unit suite, full Playwright suite, and a beta image built from
the branch.

## Open items for your review

1. **`--fs-sm` moved 11.5 → 12px and `--fs-micro` 8.5 → 10px.** Everything
   reflows slightly. The widget row unit was scaled to match, but the per-widget
   ROW COUNTS are tuned to old metrics and persisted per reader — worth a look
   on a real account with a full layout.
2. **The two `--beam` values changed** (pyro, nyx). They are more legible but
   visibly lighter; if you dislike them, the constraint to preserve is
   `--label < --beam` with real separation.
3. **`HierarchicalBucketList` is still dead code** — a two-level system → body →
   place roll-up with no equivalent on any current page. It is the one orphaned
   Journey component with no replacement.
4. **Admin is not written in the system's components** (see Phase 4).
5. `docs/plans/` is gitignored now, so the projection port's working plan lives
   outside the repo.

