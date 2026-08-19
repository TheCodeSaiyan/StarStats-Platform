'use client';

/**
 * Client wrapper around [`QuantumWarp`] that picks a flow angle per
 * route via `usePathname()`. The angle prop flips on navigation, the
 * inner canvas tweens toward the new target — no remount.
 *
 * The bare `<QuantumWarp />` previously sat in `app/layout.tsx`,
 * which made `angle` constant at the default (180) for every screen
 * — the design intent of "each route has its own flow direction"
 * (see prototype `quantum-warp.jsx` + `app.jsx` `angleFor`) was
 * never wired up in production. This component restores that wiring.
 *
 * Static map for known routes mirrors the prototype's `FIXED_ANGLE`
 * intuition (signup → upward, donate → strong sweep, etc.) and
 * extends it for the production-only paths. Unmapped paths fall
 * through to a deterministic hash so navigating to e.g. an org
 * detail page always gives the same direction without us having
 * to enumerate every slug.
 *
 * Per route intensity (see `pathnameIntensity`) further damps the
 * warp on data-dense screens — see design audit v2 §08 polish list:
 * "The warp background still competes on data-dense pages. Tune
 * down on table-heavy routes (Uploads list, Submissions list,
 * Devices, Settings)." Intensity multiplies the inner canvas's
 * baseline opacity via a wrapper layer, keeping the streak field
 * legible as ambience without competing with tabular data.
 */

import { usePathname } from 'next/navigation';
import { QuantumWarp } from './QuantumWarp';

const FIXED: Readonly<Record<string, number>> = {
  '/': 200,
  '/auth/login': 290,
  '/auth/signup': 270,
  '/auth/forgot-password': 310,
  '/auth/reset-password': 305,
  '/auth/verify': 280,
  '/auth/magic-link': 260,
  '/auth/email-change': 220,
  '/auth/totp-verify': 155,
  '/me': 180,
  '/metrics': 170,
  '/submissions': 30,
  '/admin': 0,
  '/devices': 120,
  '/orgs': 60,
  '/journey': 100,
  '/me/travel': 100,
  '/uploads': 200,
  '/settings': 195,
  '/settings/2fa': 155,
  '/support': 245,
  '/donate': 245,
  '/privacy': 215,
  // Adjacent to /privacy so the two legal pages read as one family.
  '/terms': 210,
  // Same family — /trust is the plain-English front door to what
  // /privacy and /terms say in full.
  '/trust': 220,
};

/** Tiny deterministic hash so any unmapped pathname gets a stable
 *  angle (same path → same direction across visits). djb2-ish. */
function hashAngle(s: string): number {
  let h = 5381;
  for (let i = 0; i < s.length; i++) {
    h = (h * 33) ^ s.charCodeAt(i);
  }
  return ((h % 360) + 360) % 360;
}

function angleFor(pathname: string): number {
  if (FIXED[pathname] != null) return FIXED[pathname];
  // Try progressively shorter prefixes so e.g. /orgs/foo inherits
  // /orgs's angle without enumerating every slug.
  const segments = pathname.split('/').filter(Boolean);
  for (let i = segments.length; i > 0; i--) {
    const prefix = '/' + segments.slice(0, i).join('/');
    if (FIXED[prefix] != null) return FIXED[prefix];
  }
  return hashAngle(pathname);
}

/** Routes where the warp is dimmed because the page is data-dense
 *  (tables, long forms) and the streak field competes with content.
 *  Prefix-matched so detail routes like `/devices/[id]` inherit. See
 *  design audit v2 §08. */
const DIM_PREFIXES = ['/uploads', '/submissions', '/devices', '/settings', '/admin'] as const;

/**
 * Per-route intensity multiplier applied to the warp canvas opacity.
 * Default screens render at full strength (1.0); table- and form-heavy
 * routes render at a damped value so the ambience reads but does not
 * fight tabular data for attention. See design audit v2 §08 polish
 * list ("Tune down on table-heavy routes").
 *
 * Dimmed value of 0.35 was picked to keep the streak field visible
 * as background motion without it pulling focus from rows/cells.
 */
function pathnameIntensity(pathname: string): number {
  for (const prefix of DIM_PREFIXES) {
    if (pathname === prefix || pathname.startsWith(prefix + '/')) return 0.35;
  }
  return 1.0;
}

export function QuantumWarpBackground() {
  const pathname = usePathname();
  const intensity = pathnameIntensity(pathname);
  // Wrap the canvas in a fixed, pointer-events:none layer whose opacity
  // multiplies QuantumWarp's baseline 0.65 alpha. Skipping the wrapper
  // when intensity===1 keeps the DOM identical to the pre-tuning version
  // on full-strength routes (no extra layer, no stacking-context change).
  if (intensity === 1) {
    return <QuantumWarp angle={angleFor(pathname)} />;
  }
  return (
    <div
      aria-hidden="true"
      data-warp-intensity="dim"
      style={{
        position: 'fixed',
        inset: 0,
        pointerEvents: 'none',
        zIndex: 0,
        opacity: intensity,
      }}
    >
      <QuantumWarp angle={angleFor(pathname)} />
    </div>
  );
}
