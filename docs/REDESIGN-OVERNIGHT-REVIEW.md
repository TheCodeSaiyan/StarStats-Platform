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
- **Admin was 94% flat classes by count** — now converted; see the section
  below.

---

## Phase 5 — verification

- **typecheck** clean, **lint** clean (17 warnings, all pre-existing — unused
  `React` imports in test files and `next/image` advisories).
- **Production build** compiles; **901 unit tests** across 129 files.
- **Cold `--no-cache` container build** from `git archive HEAD`, then the image
  run and inspected: all four `--label` values (`#73A6B7` terra, `#BC986C`
  stanton, `#C88F81` pyro, `#A894CA` nyx) and `.hp-rangebar__opt` are present in
  the shipped stylesheet, and `/robots.txt` disallows all under
  `STARSTATS_NOINDEX=1`. The palette is not just in the source — it is in the
  bytes the browser gets.
- **Beta deployed** from the finished branch: `web:beta` / `web:beta-39270bb`.

### A harness pattern worth knowing

Four tests flaked tonight with one shape — click a link, assert the URL, pass in
isolation, fail under load (auth, dashboard, travel, contrast). Two causes, both
fixed at the cause rather than retried:

- `expect(page).toHaveURL()` defaults to 5s while the config allows 10s for a
  navigation, so `waitForURL` is the right call after a click.
- The three whole-app sweeps each visit ~16 routes, most of them cold, so they
  get an explicit 30s goto budget. Nothing about what they assert changed.

### Commits

```
39270bb  test: cold-route navigation budget for the sweeps
59bd6dc  fix: heatmap on the beam, no filled active controls
90e0844  fix: last inline type into the system, keyboard proven
198616b  fix: --label tier split out of --dim, and what that exposed
b1e0045  fix: docked panes swallowing the scroll wheel
39ab3dc  feat: port every surface onto the projection system
```

## Admin console — converted

Done on request after the overnight pass. The admin surface went from **249
flat-class uses to 48**, and the 48 that remain are there deliberately.

### What moved

| | before | after |
|---|---|---|
| `ss-card` | 37 | 7 |
| `ss-btn` (+ variants) | 131 | 0 |
| `ss-badge` (+ variants) | 36 | 0 |
| `ss-alert` | 6 | 0 |
| inline type literals | 173 | 135 |

- **`AdminTable`** now emits `hp-tbl` markup — the same structure and classes
  `HoloTable` produces — so all seven admin listings are drawn by the system's
  rules in one change. Its API stays `columns` with cell *render functions*,
  because every listing puts chips, links and forms inside cells, and because
  `HoloTable` is `'use client'` while admin listings are server-rendered.
  Its empty state is now `Flatline`.
- **`AdminPageHeader`** takes `.hp-pagetitle`. It was inline `32px / 600 /
  -0.02em` — the flat voice, tight and semibold — for all 17 pages.
- **30 `ss-card` containers → `Plane`**, matched by walking the element tree
  rather than by regex so nested sections closed correctly.
- **Buttons and badges → `hp-btn` / `hp-chip`** at the call sites. The
  *elements* were deliberately not replaced with `BeamButton`: 13 of them are
  `ConfirmSubmitButton`, which the project's own rules require for destructive
  server actions (it composes `useFormStatus` against double-submit). Swapping
  the class gets the system's drawing without touching a form contract.
- **SMTP banners → `BeamAlert`.**

### What the conversion exposed

**The contrast sweep had never visited admin.** Adding `/admin`,
`/admin/users`, `/admin/settings` and `/admin/audit` to both sweeps found real
failures immediately:

- SMTP form hints at **5.56:1 on 11px** — a shared `Field` helper with inline
  `fontSize: 11`.
- `.hud-tile__sub` at **5.56:1 on 9px** on `/admin`: the bridge rule was
  `.hud-tile .hud-tile__sub`, and `InstrumentStrip` renders the same trio
  *without* a `.hud-tile` ancestor, so the rule missed. Six rules widened — the
  classes name a role, not a container.
- A **bare `<input>`** (no `type` attribute) painting Chrome's own grey field,
  `rgb(59,59,59)`, inside the volume. The redraw listed input *types*; several
  admin forms write `<input>` with none. Listing types was the mistake.

### What is deliberately still flat

- **`ss-eyebrow` (26)** — a sanctioned use per CLAUDE.md: a section category
  label above a heading. The bridge already redraws it.
- **`ss-placard` (4)** — the other sanctioned use, a stat-tile caption.
- **`ss-card` (7)** — `<details>`, `<p role="alert">` and card-shaped `<Link>`s.
  A disclosure and a link are not Planes.
- **135 inline style literals** that mix type with layout in one object.
  They now render correctly (both sweeps pass on all four admin routes), and
  unpicking them is mechanical churn with real regression risk on forms.

---

## Ranked rows: the affordance did not match the target

Reported as "still not all the items in lists are clickable and the clickable
parts are not working properly". Measured on a rendered `/me` under the Travel
lens, every row and its anchor:

| row | anchor href | anchor width | share of row area | cursor |
| --- | --- | --- | --- | --- |
| Avenger Stalker | `/kb/vehicle/avenger-stalker` | 91px of 529 | 10% | pointer |
| 300i | `/kb/vehicle/300i` | 26px of 529 | 3% | pointer |
| Totally Unknown Hull | *(none)* | 0px | 0% | pointer |
| ArcCorp | `/kb/location/arccorp` | 47px of 530 | 5% | pointer |
| Crusader | `/kb/location/crusader` | 53px of 530 | 6% | pointer |

Two faults, one visible symptom. The anchor wrapped only the LABEL, so 90-97%
of every row was dead space. And `cursor: pointer` plus the hover highlight
were on `.hp-rw` unconditionally, so a row with no link at all still advertised
itself as a target.

**A stretched link does not work here, and it was tried first.** `EntityLink`
gained a `stretch` prop emitting an `inset: 0` `::after`; the build was clean,
the class was correct, and a click at the row's far edge still did nothing. Two
reasons, both structural: `.nm` is `overflow: hidden`, which clips the overlay
back to the label, and `EntityLink`'s own wrapper is `position: relative`, so
`inset: 0` fills the label rather than the row. That prop has been removed.

**What shipped instead:** `MeterRow` takes an `href` and renders the ROW as the
anchor — an anchor takes `display: grid` perfectly well, so the whole row is
one hit target with one accessible name. It takes the link component through a
`linkAs` prop rather than the `renderLink` CALLBACK `ChromeBar` uses, because
the planes are built in a server module and a function prop cannot cross the
RSC boundary while a client-component reference can (`_projection/RowLink.tsx`).
`cursor`, the hover lift and the focus ring are now scoped to `.hp-rw--link`
and `.hp-rw[role="button"]`, so a row that leads nowhere looks like one.

**The cost, stated plainly:** these rows lose the `EntityHoverCard`. Keeping it
would nest an interactive element inside a link — invalid markup and a
confusing tab order. On a list whose purpose is to be clicked through, a row
that reliably navigates is worth more than a preview. Hover cards are unchanged
everywhere else.

### Three more faults found while wiring the same rows

1. **The loadout plane's data shape was fabricated.** It declared
   `{ label?, name?, count? }`; the widget returns `{ class, label, category,
   slug }`. So `count` was always undefined and every row rendered an EMPTY
   value column — and the `slug` the widget had ALREADY resolved went unused,
   which is why loadout rows linked nowhere. Same root cause as the
   `by_kind.map` crash on beta: the `as (d: never)` casts in `BUILDERS` erase
   the widget-to-builder type relationship, so a local interface that disagrees
   with its source compiles cleanly and fails only on screen.
2. **The entities plane could never render.** `buildElements` bailed on
   `!def.load`, and the `entities` widget is a nav card with no loader. It now
   builds from the reference bundle's own per-category counts, with each row
   linking to that category's KB listing. Counts come from `bundle.counts`,
   never `catalog.size` — the catalogue is dual-keyed under `class_name` AND
   `display_name`, so its size roughly doubles the entry count.
3. **Hangar rows now link.** A pledge name is a display name, not a class id,
   and the catalogue's dual-keying resolves it — so "Avenger Titan" reaches
   `/kb/vehicle/…` through exactly the same lookup as `AEGS_Avenger_Titan`.

**Deliberately still not links,** because the target would have to be guessed:
corridor rows (`A ⇄ B` is two places, not one), docking kinds (Hangar / Pad /
Other are berth types, not entities), records and combat rows (labels for
figures), and org rows — `/orgs/{slug}` is a StarStats-native org keyed by an
app slug, while the hangar snapshot carries an RSI `sid`. Linking those would
produce dead URLs. They now render with no pointer and no hover lift, which is
the honest signal.

### And a fourth, found because rows changed tag

`.hp-rw:first-of-type .rk { color: var(--hot) }` lit the leading rank. Once a
row became an `<a>` when it links and a `<div>` when it does not, that selector
had to be re-read — and re-reading it exposed a fault that was already there:
`:first-of-type` matches the first element of each TAG among its siblings, and
a plane's `.cap` is itself a `<div>`. So in any plane whose rows are ALL
unlinked — *Where you dock*, *Records*, *Orgs*, *Combat & contracts* — the cap
took the first-div slot and **no row was lit at all**, while a plane starting
with a link looked perfectly correct. Nothing pointed at it because the two
kinds of plane were never compared.

It is now `.hp-rw .rk { --hot }` plus `.hp-rw ~ .hp-rw .rk { --label }`: "every
row after a row recedes", which is tag-blind and therefore cannot drift again
when a row's element changes. `:first-child` would not do — the rows are not
the first child of the plane.

### Guard

`apps/web/e2e/projection-rows.spec.ts`. It clicks 30px in from the row's RIGHT
edge — the part the old anchor never covered — and reads the cursor on a row
that leads nowhere. Both assertions were run against the reverted code first
and both failed, along with the sweep that forbids a nested anchor inside a
row; "the link is visible" and "the link has the right href" both PASS on the
broken build, because the anchor was present, visible, correctly addressed and
3% of the row.

The rank sweep needed two corrections before it measured anything. Checking a
single plane passed against the broken selector, because the plane it checked
began with a link; it now sweeps EVERY plane. And its docking fixture used
`total` where the API field is `total_stows`, so the widget bailed, the plane
never rendered, and the sweep quietly lost the only case it existed to cover —
a green test measuring nothing. With both corrected it fails on the old
selector, naming *Where you dock*.

The loadout fix has its own test in the same file, asserting the value column
is not blank — which is the whole difference, since every structural gate
passed on the broken version: the rows were present, correctly shaped,
correctly classed and correctly counted. It fails on the old interface.

The file also carries a note worth reading if it ever goes flaky: the lens
control is server-rendered, so a single click can land before React attaches.
The default lens draws no ranked planes, so the symptom is "no rows at all"
rather than anything about rows. The click is wrapped in `expect(...).toPass()`.

---

## The chrome bar: split, and re-ordered

**The ask:** signed-in readers should get the pages they have permission to plus
a way home, with the rest hidden, so the bar stops collapsing so easily.

**What was measured first.** A signed-in reader's bar carried **17 links**, and
`data-nav` was `collapsed` at 360, 390, 414, 768, 1024, 1280, 1440, 1600 and
1920 — inline only at 2560. Breaking the inline row down on `/me`:

| part | width |
| --- | --- |
| wordmark | 121px |
| "Projection live" | 124px |
| nav (17 links) | 687px |
| lifetime readouts | 233px |
| range tabs | 360px |
| calibration pips | 199px |
| account | 90px |
| gaps (7 × 20) | 140px |
| **total** | **1953px**, against 1372 available at 1440 |

Two separate faults, and the second is the bigger one:

1. **The bar and the menu asked the same question.** Every destination a session
   could reach went into both. Nine of a signed-in reader's seventeen were
   public pages they were not working in.
2. **The fit ladder gave up navigation FIRST.** It tried `['inline','0']` and
   then went straight to `['collapsed','0']` — so the very first thing
   sacrificed was the entire nav, the most drastic reduction available, while
   the calibration caption, the "Projection live" wording and the lifetime
   readouts all kept their full width. A caption outranked the links.

**What shipped.**

`isPrimaryNav` in `lib/nav.ts` marks the inline set: home always, the reader's
own `user` / `admin` pages always, everything else inline only while signed out.
`ChromeBar` draws the flagged items in the row and keeps the FULL grouped set in
the disclosure — which now survives an inline row, because with a split the rest
of the site lives only behind it. **Nothing became unreachable; destinations
moved rather than disappeared,** and a unit test asserts exactly that by
comparing the menu against `navFor`.

The disclosure is also now its own element (`.hp-navmenu`). It has to be: one
node cannot be a nowrap row and an open column at the same width, which is what
it was being asked to be as soon as the row could fit and still be a subset.

The density ladder is now walked WITH the nav inline, and only then does the nav
collapse. What each step drops, least useful first: the emitter id, then the
citizen line and the pips' caption, then the "Projection live" wording (the
pulsing dot carries the state) and the lifetime readouts. Those are passive
figures a reader can still get by looking; a bar with no links is not something
a reader can navigate by looking.

**Measured result** — the width at which the row goes inline:

| surface | before | after |
| --- | --- | --- |
| `/settings`, `/kb`, `/discover`, `/sharing` | never below 2560 | **1280** |
| `/me`, `/me/travel` | never below 2560 | **1600** |
| signed-out `/` | never below 2560 | **1280** |

`/me` is the heaviest surface in the product — it carries the range tabs (360px)
and the lifetime readouts on top of everything else — so 1600 is where it lands
once ornament has been spent. A 1440 laptop still collapses it. The remaining
cost is the range control, and moving that out of the chrome row is the next
thing to try; it is noted below rather than attempted, because it would push the
crumb and the stage layout down and that is a bigger change than this ask.

## Mobile: measured on a phone, which changed the answer

The first audit resized a desktop browser to 390px. That was wrong, and wrong in
the direction that hides bugs: `pointer: coarse` gates every 44px target rule in
the stylesheet and is driven by `hasTouch`, so a narrow desktop window reads the
MOUSE rules. It reported tap targets that do not exist on a device, and it
missed what the touch rules do to the chrome — under coarse the pips grow to
44px each and the row gets wider, not narrower.

Re-measured on an emulated iPhone at 390×844:

**The account control was off the screen.** It drew from x=493 to x=503 on a
390px viewport, on every signed-in surface. Sign out, Calibrate and Sharing all
live in that menu. The cause: `.hp-top` is `nowrap`, and the calibration pips at
44px each cost 188px of the 358px available. The pips have a full control on
`/settings`; the account menu has no second home. The pips now go on phones.

**The range tabs were painted, costed and untappable.** Every other item in the
row is `flex: none` by the time the layout reaches a phone, so the range strip
absorbed the whole deficit alone: an 84px box for 226px of tabs, with "All"
drawn at x=441. It now scrolls, with a mask at the edge so a hard clip does not
read as the end of the list — and the wordmark, a plain `<span>` and the only
non-interactive item competing for that room, yields on phones so the strip gets
178px instead of 84.

**Tap targets.** Brought to the WCAG 2.5.8 (AA) 24px floor, measured rather than
assumed: category tabs were 18px, facet chips 23, InfoTip triggers 20, footer
and legal links 12–13, disclosures 13, inputs and selects 26–31. Every fix is
padding around an unchanged visual. Links inline in a sentence are deliberately
left alone — that is the standard's own exception, and padding them would break
the line box.

**A CSS-ordering trap worth knowing.** Half those rules did nothing at first.
`additions.css` is imported AFTER `patterns-holo.css`, so a rule in patterns for
a class DECLARED in additions loses on source order no matter what the media
query says. `.hp-cattab`, `.hp-catchip`, `.hp-tip__t` and `.hp-select` now carry
their touch sizing in additions.css beside their own declarations.

## The ring was not clickable. At all. On any device.

Found while chasing a 13px tap target, and much worse than the thing being
chased.

Every `.hp-layer` is `position: absolute; inset: 0`, so each depth layer covers
the whole stage and the last in DOM order — callouts and panes, depth 54 — sat
on top of the ring (20) and the core (36). Sampling 400 points around the ring
at 1440px and again at 390px: **every single one returned `DIV.hp-layer` and
none returned a segment.** Clicking a segment left `data-mode` on `overview`.

The ring is the projection's primary navigation. It has been dead to a pointer
for the whole of this branch, on desktop as much as on touch.

**Nothing caught it, and the reason is instructive.** The segments are
`role="button"` with `tabIndex={0}` and real key handlers, so they take a tab
stop and activate from the keyboard — the a11y sweep that walks every tab stop
passed throughout. Only a pointer ever met the layer.

The fix: a depth layer is a coordinate space, not a surface. `.hp-layer` passes
clicks through and `.hp-layer > *` takes them back, with the two genuinely
full-bleed decorations (`.hp-hex`, `.hp-floor`) and the emitter glow excluded.
The segment LABEL also passes its clicks through now — it was a sibling of the
hit band rather than inside it, so a tap on the word did nothing, and the word
is the obvious thing to aim at.

**A measurement trap this exposed:** `getBoundingClientRect` on an SVG path
returns the GEOMETRY box and excludes the stroke — and for a transparent hit
path the stroke IS the target. The ring's hit band reads 13px that way and is
24px in practice. Both the guard and the mobile audit hit-test with
`getPointAtLength` + `elementFromPoint` instead of trusting the box.

## More rows that were not the row

The `/me` row fix was not the whole story. Three more surfaces wrapped the
anchor around the label only, and all three are now row-as-link: `/kb`'s
category list (measured at 33–58px wide in a full-width row on a phone),
`PlaceDetail`'s child places and `TaxonomyStrip`'s places.

## Guards

- `apps/web/src/lib/nav.test.ts` — membership: what the row offers signed in and
  signed out, that staff keep the console and nobody else gains it, and that the
  menu still contains every destination `navFor` returns.
- `apps/web/e2e/chrome-nav.spec.ts` — behaviour: the row is a subset, the menu
  is the site, the disclosure survives an inline row and opens at both widths,
  ornament is spent before links, and a signed-out visitor is never offered a
  gated label.
- `apps/web/e2e/mobile.spec.ts` — in a real touch context: the account control
  is on screen on every signed-in surface, every range tab is reachable, no page
  scrolls sideways, and tap targets clear 24px with the inline-sentence
  exception detected structurally.
- `apps/web/e2e/ring-hit.spec.ts` — a pointer reaches the segments rather than
  the layer, and clicking one changes `data-mode`.

Every one was run against the reverted code first and seen to fail. Two needed
correcting before they measured anything: `chrome-nav` read `data-nav` before
the fit measurement had settled and reported "collapsed" on a surface that
settles inline, and the phone suite needed the touch context described above.

---

## The render error: a rate limit was being treated as a crash

Reported as a Server Components error in the browser with the message stripped.
The digest resolved in the beta container log to a wall of:

```
reference item/slug/ventris-jumpsuit-… returned 429 Too Many Requests
 ⨯ Error: Failed to load item/ventris-jumpsuit-…: 429 Too Many Requests
```

`/kb/[category]/[slug]` threw on any non-404 failure. The comment there argued
a backend error should reach the error boundary rather than a misleading 404 —
right for a genuine error, wrong for a 429, which says "come back shortly". The
reference API is per-IP rate limited and the web container is ONE IP fronting
every SSR render, so a busy moment 429s legitimate navigations. Every one of
those became a full-page crash for an entry that exists and is already
compiled into the image.

**`rate_limited` is now its own outcome**, separate from `error`. The fetcher
honours `Retry-After` when the API sends one (clamped to 2s so a render cannot
be held open) instead of guessing at a backoff — guessing is how a retry
becomes part of the load. On exhaustion the page renders **from the shipped
`reference-data` snapshot**: `ReferenceEntryDetail` is `ReferenceEntry` plus
`metadata`, so a catalogue entry with an empty blob is a valid detail, and the
live-only sections (ship matrix, media, peer stats) already handle a missing
blob and drop out on their own. A banner says where the data came from — stale
detail served as though it were live would be worse than the crash. Only when
the snapshot has no such slug either is there a "busy, reload" page, and even
then never a throw: the entry may exist and simply be unreachable this second.

Lookup is a memoised slug index (`findEntryBySlug`), because the catalogue is
keyed by class_name and display_name, never by slug — a linear scan per
rate-limited render would be thousands of comparisons on the busiest path.

**On the load itself:** the amplifiers are already handled — every KB detail
link sets `prefetch={false}` and detail fetches carry a 1h revalidate, so
repeat views collapse. The log is many DISTINCT obscure slugs at ~1/sec, which
is a systematic walk (most likely a crawler on the noindexed beta host). That
cannot be stopped from the web tier, which is exactly why degrading is the fix
rather than the consolation prize.

## The rows that would not link: their labels were never place names

The other half of "the items in the lists are not clickable", and the
screenshot showed it: the Travel lens listed `Stanton|clio|`,
`Stanton|microTech|New Babbage`, `Rr||mic Leo` and a bare `||`.

`top_locations[].value` and `routes[].destination` are `system|planet|city`
composite keys, not names. The flat `locations` and `routes` widgets both run
`aggregateLocationBuckets` to resolve them and merge duplicates; the projection
builders skipped it and passed the raw value through. So the rows read as
machine keys AND could never link — a composite matches nothing in a catalogue
keyed by class and display name.

Measured against the shipped snapshot:

| raw | resolved | links |
| --- | --- | --- |
| `Stanton\|clio\|` | Clio | `/kb/location/clio` |
| `Stanton\|microTech\|New Babbage` | New Babbage | `/kb/location/new-babbage` |
| `Rr\|\|mic Leo` | Mic Leo | no catalogue entry |
| `\|\|` | Unknown | no catalogue entry |

Both planes now run the same resolver the widgets do, and `entityRow` gained a
`label` parameter to PIN the display text: without it a catalogue miss falls
through to `toFriendlyName`, which rewrites prose — a merged "Mission beacon"
or a fallback "Unknown" would come back mangled. The catalogue lookup still
runs, so a real place still links.

Deliberately still not links, because the target would have to be guessed:
docking kinds (Hangar / Pad / Other are berth types, not entities) and
corridors (`A ⇄ B` is two places, so there is no single destination).

## Guards, and two corrections to my own measurements

`kb.spec.ts` reproduces a 429 against a slug that really exists in the
snapshot and asserts no error boundary, that the entry still renders, and that
the page admits where the data came from. `projection-rows.spec.ts` uses the
verbatim raw values from the report and asserts no pipe survives to the screen,
that the composites became the places they name, and that Clio is a link. Both
were run against the reverted code and seen to fail.

Two things I got wrong along the way, recorded because the method matters:

1. **A throwaway probe reported a `/downloads` button "blocked by an h2" and
   collapsed disclosures "leaking content".** Both false.
   `getBoundingClientRect` returns a full-size rect for a
   `content-visibility: hidden` subtree and `display` is not `none`, so hidden
   content measured as visible. `checkVisibility()` and Playwright's
   `isVisible()` both disagree with the probe. The same flaw was in the
   committed tap-target sweep and is now fixed there.
2. **My first "production build" reproduction was invalid.** Playwright's dev
   server shares the `.next` directory and had clobbered the build, so I was
   driving a broken artifact. Running the production server ON the test port —
   so Playwright reuses it instead of starting `next dev` — is what made the
   run real. It then rendered every route cleanly, which is what proved the
   fault was data-dependent rather than structural.

---

## Open items for your review

1. **`--fs-sm` moved 11.5 → 12px and `--fs-micro` 8.5 → 10px.** Everything
   reflows slightly. The widget row unit was scaled to match, but the per-widget
   ROW COUNTS are tuned to old metrics and persisted per reader — worth a look
   on a real account with a full layout.
2. ~~**The two `--beam` values changed** (pyro, nyx).~~ **KEPT, and the
   constraint is now enforced.** Measured before deciding:

   | | old beam | new beam | vs void | beam vs label |
   | --- | --- | --- | --- | --- |
   | pyro | `#FF6B4A` | `#FF8E74` | 7.20 → 9.06 | **1.22** |
   | nyx | `#B78BFF` | `#C19BFF` | → 9.08 | **1.21** |
   | terra | unchanged | `#7FE4FF` | 13.96 | 1.84 |
   | stanton | unchanged | `#FFAE3B` | 10.99 | 1.45 |

   Both were lightened to clear 9:1 where the originals only just cleared 7:1,
   and that necessarily moved them toward a `--label` pinned at "7:1 and no
   further" — so pyro and nyx are the tightest by construction, not by
   accident. The values are kept for the legibility.

   The named constraint was only half-enforced: `palette-contrast.test.ts`
   asserted the ORDER (`label < beam`) but never the DISTANCE, so a label could
   creep to within a hair of its beam and still pass while the tier collapsed.
   There is now a 1.2:1 floor measured directly between the two colours, set at
   today's worst case so the current palette passes and any erosion fails.
   Verified by nudging nyx's label one step toward its beam: the gap falls to
   1.05:1 and the new test fails — while the old ordering test passes, which is
   precisely the hole.
3. **`HierarchicalBucketList` is still dead code**, and I now know WHY it
   cannot simply be wired back in. Attempted: put it in the `locations` widget,
   which is the obvious home — `rollUpLocations` exists, and `maxNodes` is
   documented as the cap that makes it tile-safe.

   It does not fit the data. `rollUpLocations` calls `parseLocationClass`,
   which calls `stripAndSplit` — and that splits on `_`, because it parses
   ENGINE CLASS NAMES (`Stanton1b_Lorville`). But `/v1/me/stats/locations`
   returns `top_locations` keyed `system|planet|city`, pipe-delimited. Those
   composites survive `stripAndSplit` as a single token and resolve to
   nothing, so the roll-up renders an empty tree. It typechecks perfectly and
   draws nothing — caught only by asserting a PARENT row appears with the
   summed count of its children.

   So this is orphaned because its input shape is: the raw class names it
   parses came from journey events, and no surface feeds it those any more.
   Reviving it means either an adapter from the composite keys (the hierarchy
   is right there in the string — `aggregateLocationBuckets` already splits
   it) or pointing it at a source that still emits class names. That is a data
   decision, not a wiring job, which is why it has stayed dead.

   The attempt is reverted; nothing of it is on the branch.
4. ~~**Admin is not written in the system's components**~~ **NOT REPRODUCIBLE —
   it draws in the idiom.** This was the JSX-name scan again, the measure Phase
   4 itself records as measuring nothing: 23 of 51 admin files import `holo`
   directly and the rest use the system's CSS classes, which renders
   identically. Measured properly on computed style, all 17 real admin routes
   pass the same check as every other surface.

   What WAS wrong was the measurement. `idiom.spec.ts` listed three admin
   routes out of twenty, and thirteen of the others reached the error boundary
   on a missing fixture — the boundary is itself drawn in the idiom, so those
   pages would have passed while showing "SOMETHING WENT WRONG". The sweep now
   fails on a page that did not render, is split from the core surfaces so
   neither masks the other, and covers all 17 (91–165 elements each, verified,
   rather than the 13-element error shell). Three routes were dropped from the
   list as redirect aliases to `/admin/settings` — `appearance`, `ship-matrix`
   and `smtp`, given away by all four reporting an identical 165 elements.
5. `docs/plans/` is gitignored now, so the projection port's working plan lives
   outside the repo.
6. ~~**The remaining `BUILDERS` interfaces are unverified.**~~ **DONE.**
   `defineWidget` now returns `TypedWidgetDef<D>`, carrying the loader's data
   type on a phantom field, and the `callout` / `plane` binding helpers infer
   `D` from the widget and check the builder against it. The cast survives once
   inside each helper, after the relationship has been proven. Verified by
   reintroducing the original `docking` bug (`by_kind` as an array): it now
   fails at the binding site with `Type '{ hangar: number; other: number; pad:
   number; }' is missing the following properties from type 'readonly { key:
   string; count: number; }[]'` instead of throwing `by_kind.map is not a
   function` on every render. All 21 loader-backed builders type-check as they
   stand; `entities` is exempt because it is a nav card with no loader.
7. ~~**The range control still costs `/me` its inline row at 1440.**~~ **FIXED,
   and the range control did not have to move.** Measured at 1440 on `/me`, nav
   inline at the tightest density: 1268px of content in a 1372px row — 36px
   short. The seven 20px gaps cost 140px. So the ladder was surrendering every
   destination to protect a margin, and then, having collapsed, resetting to
   dense 0 and drawing the calibration caption and the lifetime readouts at
   full width in the space the links had just vacated.

   `gap: 14px` at `data-dense="3"` frees 42px, applied only at the tier that
   exists to be the last thing tried before navigation goes. `/me` now reaches
   `nav=inline` at 1440 with 19px of spacer to spare. 1280 still collapses and
   should: even at the tighter gap it wants 1366px in a 1212px row.

   Pinned by `chrome-nav.spec.ts` on `/me` (the heaviest surface, since it
   carries the range tabs AND the lifetime readouts), asserting the density as
   well as `inline` so a later change cannot pass by dropping something a
   reader needs. Verified it fails at the old 20px gap.
8 & 9. ~~**The wordmark is hidden on phones**~~ / ~~**the calibration pips are
   gone from the phone chrome**~~ — **both confirmed necessary, with numbers.**
   Re-measured at 390px after the gap fix above, which also tightened the phone
   row: the chrome carries 302px of content plus 30px of gaps in the 358px
   available, leaving **26px free**.

   The wordmark is 121px and the pips 99px. Neither fits, with or without the
   other, so these were not close calls — there is no arrangement of that row
   that keeps them and the account menu, and the account menu is the one with
   no second home (sign out, Calibrate and Sharing all live in it). `/settings`
   carries the full calibration control.

   Left as they are. The existing phone tests already pin what matters: the
   account control stays on screen on every signed-in surface, and every range
   tab is reachable.
10. **`/me` under the default "All" lens draws no ranked planes at all** — only
   callouts and the trace. That is the lens preference behaving as written, not
   a fault, but it means the first thing a reader sees has no lists in it.

