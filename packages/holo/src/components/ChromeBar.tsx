'use client';

import React from 'react';

export const CALIBRATIONS = [
  { id: 'terra', pip: '#7FE4FF', name: 'Terra' },
  { id: 'stanton', pip: '#FFAE3B', name: 'Stanton' },
  { id: 'pyro', pip: '#FF6B4A', name: 'Pyro' },
  { id: 'nyx', pip: '#B78BFF', name: 'Nyx' },
] as const;

export type CalibrationId = (typeof CALIBRATIONS)[number]['id'];

/**
 * Beam selector: four lit pips, no swatch chrome.
 *
 * The pips stay 7px dots inside 44px buttons on touch — the TARGET grows around
 * the visual rather than the visual scaling up.
 */
export function CalibrationPips({
  active = 'terra',
  onSelect,
  label = 'Calibration',
}: {
  active?: string;
  onSelect?: (id: CalibrationId) => void;
  label?: React.ReactNode;
}) {
  return (
    <span className="hp-cal">
      {label ? <span className="cl">{label}</span> : null}
      {CALIBRATIONS.map((c) => (
        <button
          key={c.id}
          type="button"
          aria-pressed={c.id === active}
          aria-label={`${c.name} calibration`}
          style={{ ['--pip' as string]: c.pip } as React.CSSProperties}
          onClick={() => onSelect && onSelect(c.id)}
        />
      ))}
    </span>
  );
}

/** Closes a popover on outside click and Escape. */
function useDismiss(open: boolean, close: () => void) {
  React.useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    const onDoc = (e: MouseEvent) => {
      const t = e.target;
      // Guard the cast: a click can land on a non-Element node, and `closest`
      // exists only on Elements.
      if (!(t instanceof Element)) return close();
      if (!t.closest('[data-pop]')) close();
    };
    document.addEventListener('keydown', onKey);
    document.addEventListener('click', onDoc);
    return () => {
      document.removeEventListener('keydown', onKey);
      document.removeEventListener('click', onDoc);
    };
  }, [open, close]);
}

export interface NavItem {
  id: string;
  label: string;
  active?: boolean;
  /**
   * Real destination. When present the item renders as a plain anchor to it
   * rather than calling `onNavigate` — which is what the web app wants, so the
   * browser gets a real URL, middle-click and prefetch.
   */
  href?: string;
}

export interface NavSection {
  title: string;
  items: NavItem[];
}

export interface AccountItem {
  id: string;
  label: string;
  href?: string;
  /**
   * Count shown beside the label, and mirrored onto the handle control so it is
   * visible without opening the menu.
   *
   * An addition made during the port, and not a cosmetic one: the flat
   * `AccountMenu` carried an inbound-share badge on every signed-in page — "N
   * people have shared a record with you" — and the projection had nowhere to
   * put it, so the only way to learn of a new share became visiting `/sharing`
   * and looking. A notification you have to go looking for is not one.
   */
  badge?: number;
}

export interface ChromeBarProps {
  handle?: string;
  /** Citizen record number. NOT surfaced on /me — the product has no such field
   *  (gap B6), and a fabricated one would be worse than an absent one. */
  citizen?: string;
  emitter?: string;
  clock?: React.ReactNode;
  live?: boolean;
  calibration?: string;
  onCalibrate?: (id: CalibrationId) => void;
  /** Flat link nodes, when the caller is not using grouped `sections`. */
  links?: React.ReactNode;
  sections?: NavSection[];
  account?: AccountItem[];
  /**
   * Renders the chrome's real links.
   *
   * The default is a plain `<a href>`, which is correct for a design system
   * that knows nothing about a router — real anchors give middle-click,
   * open-in-new-tab and a status-bar target, which `onNavigate` alone does not.
   *
   * But a plain anchor is a FULL DOCUMENT LOAD. In the flat product the same
   * links were `next/link` and every chrome navigation was a client
   * transition; after the projection port every one of them reloaded the page,
   * losing scroll, state and about a second. Measured, not assumed: a marker
   * set on `window` did not survive a nav click.
   *
   * So the host injects its own link renderer here and keeps both properties.
   */
  renderLink?: (props: {
    href: string;
    children: React.ReactNode;
    onClick?: () => void;
    'aria-current'?: 'page';
    role?: string;
    className?: string;
  }) => React.ReactNode;
  onNavigate?: (id: string) => void;
  onSignIn?: () => void;
  onSignOut?: () => void;

  /* ── Additions for /me (gaps A5, A6, B6) ──────────────────────────────
   * The flat product's identity header was deliberately range-INDEPENDENT: a
   * stable "who am I" anchor that did not shift with the range control below
   * it. In the projection the callouts and the ring are all range-scoped, so
   * the lifetime figures move up here where nothing moves — which is what
   * keeps that intent visible rather than losing it. */
  /** Supporter chip, rendered beside the handle. */
  supporter?: React.ReactNode;
  /** "Citizen since 2021", from `enlistment_date`. Year only. */
  since?: React.ReactNode;
  /** The lifetime identity figures. Range-independent by construction. */
  readouts?: React.ReactNode;
  /** Trailing control slot — the range tabs live here (gap A6). */
  trailing?: React.ReactNode;
}

/**
 * Top identity row. Two popovers, both dismissible: the site nav and the
 * account menu.
 *
 * The nav is GROUPED. Thirteen destinations in one flat list is a directory,
 * not a menu — a reader cannot tell that "Trust" is a public page and "Sharing"
 * is their own data. Sections carry that. Gate the set with
 * `navFor({ signedIn, role })` before it gets here: a signed-out visitor should
 * not even see the labels of pages they cannot open. Hiding a link is
 * presentation, not protection — guard the route too.
 *
 * The handle is a CONTROL, not a label.
 *
 * Fit is measured, never guessed: chrome collapse is not a breakpoint, because
 * the link count is a consumer's choice.
 */
export function ChromeBar({
  handle,
  citizen,
  emitter,
  clock,
  live = true,
  calibration,
  onCalibrate,
  links,
  sections,
  account,
  renderLink,
  onNavigate,
  onSignIn,
  onSignOut,
  supporter,
  since,
  readouts,
  trailing,
}: ChromeBarProps) {
  const rowRef = React.useRef<HTMLDivElement | null>(null);
  const [fit, setFit] = React.useState({ nav: 'collapsed', dense: '0' });
  const [navOpen, setNavOpen] = React.useState(false);
  const [acctOpen, setAcctOpen] = React.useState(false);
  useDismiss(navOpen, () => setNavOpen(false));
  useDismiss(acctOpen, () => setAcctOpen(false));

  const grouped = Array.isArray(sections) && sections.length > 0;
  const flat = React.Children.toArray(links);
  const hasNav = grouped || flat.length > 0;

  React.useLayoutEffect(() => {
    const row = rowRef.current;
    if (!row) return;
    let raf = 0;
    const measure = () => {
      const prevNav = row.getAttribute('data-nav');
      const prevDense = row.getAttribute('data-dense');
      row.setAttribute('data-measuring', '');
      const fits = () => row.scrollWidth <= row.clientWidth;
      const steps: [string, string][] = hasNav
        ? [
            ['inline', '0'],
            ['collapsed', '0'],
            ['collapsed', '1'],
            ['collapsed', '2'],
          ]
        : [
            ['collapsed', '0'],
            ['collapsed', '1'],
            ['collapsed', '2'],
          ];
      let chosen = steps[steps.length - 1];
      for (const [nav, dense] of steps) {
        row.setAttribute('data-nav', nav);
        row.setAttribute('data-dense', dense);
        if (fits()) {
          chosen = [nav, dense];
          break;
        }
      }
      row.removeAttribute('data-measuring');
      if (prevNav) row.setAttribute('data-nav', prevNav);
      else row.removeAttribute('data-nav');
      if (prevDense) row.setAttribute('data-dense', prevDense);
      setFit((p) =>
        p.nav === chosen[0] && p.dense === chosen[1]
          ? p
          : { nav: chosen[0], dense: chosen[1] },
      );
    };
    measure();
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(measure);
    });
    ro.observe(row);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [hasNav, flat.length, grouped, handle, citizen, emitter, clock]);

  React.useEffect(() => {
    if (fit.nav === 'inline') setNavOpen(false);
  }, [fit.nav]);

  const go = (id: string) => (e: React.MouseEvent) => {
    e.preventDefault();
    setNavOpen(false);
    setAcctOpen(false);
    onNavigate && onNavigate(id);
  };
  const close = () => {
    setNavOpen(false);
    setAcctOpen(false);
  };


  /** One place both link sites go through, so the host's renderer cannot be
   *  wired into the nav and forgotten on the account menu. */
  const link = (props: {
    key: string;
    href: string;
    children: React.ReactNode;
    onClick?: () => void;
    'aria-current'?: 'page';
    role?: string;
  }) => {
    const { key, ...rest } = props;
    return (
      <React.Fragment key={key}>
        {renderLink ? renderLink(rest) : <a {...rest}>{rest.children}</a>}
      </React.Fragment>
    );
  };

  /** Total of every account item's badge — the inbound-share count today. */
  const acctBadge = (account || []).reduce((n, a) => n + (a.badge ?? 0), 0);

  return (
    <div
      className="hp-top"
      ref={rowRef}
      data-nav={hasNav ? fit.nav : undefined}
      data-dense={fit.dense}
    >
      <span className="id">Starstats</span>
      {live ? (
        <span className="live">
          <i />
          Projection live
        </span>
      ) : (
        <span className="held">Projection held</span>
      )}

      {hasNav ? (
        <span className="hp-navwrap" data-pop>
          <button
            type="button"
            className="hp-navtoggle"
            aria-expanded={navOpen}
            aria-label={navOpen ? 'Close navigation' : 'Open navigation'}
            onClick={() => {
              setNavOpen((o) => !o);
              setAcctOpen(false);
            }}
          >
            <svg width="15" height="15" viewBox="0 0 18 18" aria-hidden="true">
              <path
                d={
                  navOpen ? 'M4 4l10 10M14 4L4 14' : 'M3 5h12M3 9h12M3 13h12'
                }
                stroke="currentColor"
                strokeWidth="1.4"
                strokeLinecap="round"
                fill="none"
              />
            </svg>
          </button>
          <nav
            className="hp-lk"
            data-open={navOpen ? 'true' : undefined}
            aria-label="Site"
          >
            {grouped
              ? sections!.map((s) => (
                  <span className="grp" key={s.title}>
                    <span className="ttl">{s.title}</span>
                    {s.items.map((it) =>
                      it.href ? (
                        link({
                          key: it.id,
                          href: it.href,
                          'aria-current': it.active ? 'page' : undefined,
                          onClick: close,
                          children: it.label,
                        })
                      ) : (
                        <a
                          key={it.id}
                          href={'#' + it.id}
                          aria-current={it.active ? 'page' : undefined}
                          onClick={go(it.id)}
                        >
                          {it.label}
                        </a>
                      ),
                    )}
                  </span>
                ))
              : flat}
          </nav>
        </span>
      ) : null}

      <span className="sp" />
      {readouts ? <span className="hp-idstats">{readouts}</span> : null}
      {emitter ? <span className="emit">Emitter {emitter}</span> : null}
      {trailing}
      {onCalibrate ? (
        <CalibrationPips active={calibration} onSelect={onCalibrate} />
      ) : null}
      {clock ? <span className="clock">{clock}</span> : null}

      {/* Account. A signed-out visitor gets a way in; a signed-in one gets
          the menu where settings and sign-out are expected to live. */}
      {handle ? (
        <span className="hp-acct" data-pop>
          <button
            type="button"
            className="btn"
            aria-expanded={acctOpen}
            onClick={() => {
              setAcctOpen((o) => !o);
              setNavOpen(false);
            }}
          >
            <i aria-hidden="true" />@{handle}
            {/* Mirrored onto the CONTROL, not just the menu items: a badge that
                only appears once you open the menu tells you nothing you did
                not already have to go looking for. */}
            {acctBadge ? (
              <i className="hp-badge" aria-label={`${acctBadge} new`}>
                {acctBadge > 99 ? '99+' : acctBadge}
              </i>
            ) : null}
          </button>
          {supporter}
          <div
            className="menu"
            data-open={acctOpen ? 'true' : undefined}
            role="menu"
          >
            <span className="who">
              @{handle}
              {since ? <b>{since}</b> : null}
              {citizen ? <b>Citizen {citizen}</b> : null}
            </span>
            {(account || []).map((a) =>
              a.href ? (
                link({
                  key: a.id,
                  href: a.href,
                  role: 'menuitem',
                  onClick: close,
                  children: (
                    <>
                      {a.label}
                      {a.badge ? <i className="hp-badge">{a.badge}</i> : null}
                    </>
                  ),
                })
              ) : (
                <a
                  key={a.id}
                  href={'#' + a.id}
                  role="menuitem"
                  onClick={go(a.id)}
                >
                  {a.label}
                  {a.badge ? <i className="hp-badge">{a.badge}</i> : null}
                </a>
              ),
            )}
            {onSignOut ? (
              <button
                type="button"
                className="out"
                role="menuitem"
                onClick={() => {
                  setAcctOpen(false);
                  onSignOut();
                }}
              >
                Sign out
              </button>
            ) : null}
          </div>
        </span>
      ) : onSignIn ? (
        <button type="button" className="hp-signin" onClick={onSignIn}>
          Sign in
        </button>
      ) : null}
    </div>
  );
}
