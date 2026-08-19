import type { ResolvedLocation, SupporterStatusDto } from '@/lib/api';

/** One telemetry-rail frame: a placard label over a value. */
export interface TelemetryFrame {
  label: string;
  value: string;
  /** Render the value in the accent colour (e.g. supporter status). */
  accent?: boolean;
}

/** Tier key → short supporter label. */
export function supporterLabel(key: string | null | undefined): string {
  if (key === 'coffee') return 'Coffee supporter';
  if (key === 'generous') return 'Generous supporter';
  return 'Supporter';
}

export interface TelemetryInput {
  location?: ResolvedLocation | null;
  supporter?: SupporterStatusDto | null;
  eventsTotal?: number | null;
  locationsCount?: number | null;
}

/**
 * Derive the ordered telemetry frames from the shell's fail-soft data.
 * Shared by the desktop rail (vertical) and the mobile ticker (horizontal)
 * so the two stay in lockstep. Absent data drops its frame.
 */
export function telemetryFrames({
  location = null,
  supporter = null,
  eventsTotal = null,
  locationsCount = null,
}: TelemetryInput): TelemetryFrame[] {
  const frames: TelemetryFrame[] = [];
  if (eventsTotal != null) {
    frames.push({ label: 'Events', value: eventsTotal.toLocaleString() });
  }
  if (locationsCount != null) {
    frames.push({ label: 'Locations', value: locationsCount.toLocaleString() });
  }
  const here = location
    ? (location.city ?? location.planet ?? location.system ?? 'In transit')
    : null;
  if (here) {
    frames.push({ label: 'You are here', value: here });
  }
  if (
    supporter != null &&
    (supporter.state === 'active' || supporter.state === 'lapsed')
  ) {
    frames.push({
      label: 'Status',
      value: supporterLabel(supporter.current_tier_key),
      accent: true,
    });
  }
  return frames;
}
