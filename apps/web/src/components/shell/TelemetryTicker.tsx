import { telemetryFrames, type TelemetryInput } from './telemetry';

/**
 * Mobile telemetry ticker (bridge). On mobile the desktop telemetry rail
 * (LeftRail) collapses into the off-canvas drawer, so its live readouts
 * would be hidden. This renders the SAME frames (via the shared
 * `telemetryFrames`) as a horizontal, scrollable strip pinned under the
 * command bar. Desktop hides it via CSS; the rail hides its own telemetry
 * section on mobile. Server component — no client state.
 */
export function TelemetryTicker(props: TelemetryInput) {
  const frames = telemetryFrames(props);
  if (frames.length === 0) return null;
  return (
    <div className="ss-telemetry-ticker" aria-label="Live telemetry">
      {frames.map((f) => (
        <div key={f.label} className="ss-ticker-frame hud-tile--live">
          <span className="ss-ticker-frame__label ss-placard">{f.label}</span>
          <span
            className="ss-ticker-frame__value mono"
            style={f.accent ? { color: 'var(--accent)' } : undefined}
          >
            {f.value}
          </span>
        </div>
      ))}
    </div>
  );
}
