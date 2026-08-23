'use client';

import React from 'react';
import { useRouter } from 'next/navigation';
import { chromeLink } from '@/components/projection/chromeLink';
import type { Route } from 'next';
import {
  Projection,
  ChromeBar,
  Crumb,
  type Calibration,
  type CalibrationId,
  type NavSection,
} from 'holo';
import { SiteLegalPlate } from '@/components/projection/SiteLegalPlate';
import { useShellData } from '@/components/projection/ShellData';
import { AdminNav } from '../_components/AdminNav';

/**
 * `/admin/**` — the **Console**, in the projection.
 *
 * NOT `PaneSurface`. Every other ported surface hands the projection a list of
 * sections and lets it own the panes; the admin layout's job is the opposite —
 * it frames ~20 pages it knows nothing about, each of which renders its own
 * content.
 *
 * The shell is the system's own `Console` — index rail plus work area — via
 * `AdminNav`, per `Console.prompt.md`. Two earlier passes hand-built a strip
 * above the content instead (rounded pills, then hairline tabs); both were
 * bespoke shells for a component the system already ships, and neither scaled
 * to twenty routes without wrapping onto several lines.
 *
 * `surface="console"` is DECLARED, not inferred. The system's own note on it:
 * console "kills the ambience entirely (no parallax, scanlines or floor)
 * because at eight hours a day it is noise". This is the one surface in the
 * product that argument is actually about.
 *
 * NO `heading` ON THE CRUMB. Every other ported surface passes it because the
 * projection has no page title of its own — but each admin page already renders
 * its own `<h1>` via `AdminPageHeader`, and that h1 names the specific page
 * (Users, Audit log, Parser rules) far better than a shared crumb could. Adding
 * one here would put two h1s on all twenty of them.
 *
 * The single `role="main"` for the whole admin surface lives here now; it used
 * to be on the layout's wrapper div. Exactly one landmark per page still holds,
 * and it is still a DIV rather than a `<main>` element because globals.css
 * clamps a bare `<main>` into a 720px legacy column that would crush every
 * admin table.
 */
export function ConsoleShell({
  handle,
  calibration,
  nav,
  onCalibrate,
  children,
}: {
  handle: string;
  calibration: Calibration;
  nav: NavSection[];
  onCalibrate: (id: string) => void | Promise<void>;
  children: React.ReactNode;
}) {
  const router = useRouter();
  const { inboundShares } = useShellData();
  const [cal, setCal] = React.useState<Calibration>(calibration);
  React.useEffect(() => setCal(calibration), [calibration]);
  const [recalKey, setRecalKey] = React.useState(0);

  // Local beam state, not the server prop: the persist action deliberately
  // does not revalidate, so rendering the prop would fire the shock ring over
  // a volume that stayed the old colour until the next navigation.
  const calibrate = (id: CalibrationId) => {
    setCal(id);
    setRecalKey((k) => k + 1);
    onCalibrate(id);
  };

  return (
    <div className="ss-projection-root">
      <Projection
        calibration={cal}
        recalKey={recalKey}
        surface="console"
        parallax={false}
        chrome={
          <ChromeBar
            renderLink={chromeLink}
            handle={handle}
            calibration={cal}
            onCalibrate={calibrate}
            sections={nav}
            live
            account={[
              { id: 'me', label: 'Projection', href: '/me' },
              {
                id: 'sharing',
                label: 'Sharing',
                href: '/sharing',
                // Same reason as `/me`: the Console builds its own `ChromeBar`.
                badge: inboundShares > 0 ? inboundShares : undefined,
              },
              { id: 'settings', label: 'Calibrate', href: '/settings' },
            ]}
            onNavigate={(id) => router.push(`/${id}` as Route)}
          />
        }
        crumb={
          <Crumb
            parts={[
              { t: 'Projection', onClick: () => router.push('/me' as Route) },
              { t: 'Console' },
            ]}
          />
        }
      >
        {/* `Console` is the system's operator shell — index rail plus work
            area — and it WRAPS the content rather than sitting above it. The
            first two passes at this surface hand-built a strip instead: pills,
            then hairline tabs. Both were bespoke shells for something the
            system already ships, and the rail is also the shape that scales to
            twenty routes without wrapping onto four lines. */}
        <AdminNav>
          <div className="hp-conwork">
            {children}
            {/* Brand book §11: the attribution and outbound links must be
                reachable from every signed-in surface. The flat
                `.ss-app-footer` carried that and the projection hides it. */}
            <SiteLegalPlate />
          </div>
        </AdminNav>
      </Projection>
    </div>
  );
}
