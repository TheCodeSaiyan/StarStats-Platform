/**
 * `holo` — the StarStats projection design system as React components.
 *
 * One visual language: a holographic volume. No cards, no borders, no fills —
 * line, glow and depth do the work a card layer used to. Source of truth for
 * the language is `.claude/skills/starstats-design/`; these are the vendored,
 * typed, App-Router-ready implementations of it.
 *
 * Rules that are easy to get wrong, repeated here because they get broken:
 *   - One face: Chakra Petch. No second family.
 *   - ONLY FIGURES GLOW. Labels, captions and body copy never do.
 *   - Never a filled or rounded button (`BeamButton` is a lit hairline box),
 *     never a boxed input (`BeamInput` is a lit underline).
 *   - Calibration is `data-cal`, never `data-theme`.
 *   - `Plane` tilt needs a `perspective` ancestor; `Projection` supplies 1500px.
 *   - Touch: grow the target around the visual, never scale the visual. 44px.
 *   - Declare surface intent (`surface="brand" | "console"`), never infer it.
 *   - No emoji. Geometric Unicode + 1.4–1.6 stroke SVG only.
 *
 * Deliberately absent, upstream and here: Modal/Dialog, Toast, Tabs, Avatar,
 * Pagination, Accordion, Select. The projection routes depth changes through
 * panes rather than modals. If you need one, raise it — do not invent it.
 *
 * Import the CSS once, at the app root: `import 'holo/styles.css'`.
 */

export { Projection, Depth } from './components/Projection';
// The sanctioned series palette — a narrow, documented exception to the
// one-colour rule, for multi-series identity only.
export { seriesColor, seriesDash, SERIES_SLOTS } from './series';
// Overlay comparison — the browse screen's own feature, vendored from the kit.
export { CompareChart, CompareBar } from './components/CompareChart';
export type { CompareStat, CompareSeries } from './components/CompareChart';
// Signed-out surfaces. Vendored when the landing page ported; nothing before it
// needed a brand statement or a legal footer.
export { BrandHero } from './components/BrandHero';
export type { BrandHeroProps } from './components/BrandHero';
export { LegalPlate, CIG_DISCLAIMER } from './components/LegalPlate';
export type { LegalPlateProps } from './components/LegalPlate';
export type {
  ProjectionProps,
  ProjectionMode,
  ProjectionSurface,
  Calibration,
} from './components/Projection';

export { Ring } from './components/Ring';
export type {
  RingProps,
  RingSegment,
  RingNode,
  RingTick,
} from './components/Ring';

export { CoreReadout } from './components/CoreReadout';
export type { CoreReadoutProps } from './components/CoreReadout';

export {
  Callout,
  CalloutField,
  CALLOUT_SLOTS,
  slotFor,
} from './components/Callout';
export type { CalloutProps, CalloutPosition } from './components/Callout';

export { Pane, SubStats } from './components/Pane';
// The operator shell — index rail plus work area. Pair with
// `Projection surface="console"`.
export { Console } from './components/Console';
export type { ConsoleItem, ConsoleGroup } from './components/Console';
export type { PaneProps, SubStatItem } from './components/Pane';

export { Plane, MeterRow, LogRow } from './components/Plane';
// The projection's two chart shapes. `values` is required — see the component.
export { Trace } from './components/Trace';
export type { TraceProps } from './components/Trace';
export type {
  PlaneProps,
  MeterRowProps,
  LogRowProps,
} from './components/Plane';

export { LensRail, Crumb, RangeTabs } from './components/LensRail';
export type { LensItem, CrumbPart } from './components/LensRail';

export { ChromeBar, CalibrationPips, CALIBRATIONS } from './components/ChromeBar';
export type {
  ChromeBarProps,
  NavItem,
  NavSection,
  AccountItem,
  CalibrationId,
} from './components/ChromeBar';

export { BeamButton, BeamChip } from './components/BeamButton';
export type { BeamButtonProps, BeamButtonVariant } from './components/BeamButton';

export { BeamInput, BeamSelect, BeamSwitch } from './components/BeamInput';
export type { BeamInputProps, BeamSelectProps } from './components/BeamInput';

export { HoloTable, HoloKV } from './components/HoloTable';
export type { HoloColumn, HoloKVItem } from './components/HoloTable';

export { CalibrationChoice, BeamChoice } from './components/CalibrationChoice';
export type {
  CalibrationChoiceProps,
  BeamChoiceProps,
} from './components/CalibrationChoice';

export { BeamTextarea } from './components/BeamTextarea';
export type { BeamTextareaProps } from './components/BeamTextarea';

export { BeamAlert } from './components/BeamAlert';
export type { BeamAlertProps, BeamAlertTone } from './components/BeamAlert';

export { Flatline } from './components/Flatline';
export type { FlatlineProps, FlatlineReason } from './components/Flatline';

export { BeamTip } from './components/BeamTip';
export type { BeamTipProps } from './components/BeamTip';

export { LayoutEditor, useLayout, UseLayout } from './components/LayoutEditor';
export type {
  CatalogueEntry,
  LayoutApi,
  LayoutEditorProps,
  UseLayoutOptions,
} from './components/LayoutEditor';

export {
  layoutMapNodes,
  equalShares,
  sharesFromWeights,
  MAX_MAP_NODES,
} from './ring-layout';
export type { MapStop, MapEdge, MapLayout } from './ring-layout';
