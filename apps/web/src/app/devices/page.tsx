/**
 * Connected Uplinks — per-client identity surface. Each paired desktop client gets
 * its own tab; inside the tab we show pairing info plus an Activity
 * section listing recent ingest batches the device posted.
 *
 * The "no raw retention" stance is hard: we show per-batch metadata
 * (counts, build, timestamp) and nothing else. There is no per-line
 * drilldown affordance by design.
 *
 * Historical context: this surface used to be split between /devices
 * (identity) and /uploads (batches). The audit (v2 §03/§07) folded the
 * latter into a per-device Activity tab here so users see a single
 * device's pipeline rather than two parallel global lists. /uploads
 * now redirects here.
 *
 * Per-device filtering: the ingest handler stamps the device_id off
 * the bearer token's device claim into the audit payload (migration
 * 0026), and `getIngestHistory` passes `device_id=<active tab id>` so
 * the API returns only that device's batches. Legacy rows (pre-0026)
 * have no device_id and won't appear under any device tab — they're
 * still visible by hitting the endpoint without the filter, which
 * isn't surfaced in this UI by design.
 */

import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  ApiCallError,
  getIngestHistory,
  listDevices,
  revokeDevice,
  startPairing,
  type DeviceDto,
  type IngestBatchDto,
  type IngestHistoryResponse,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { InstrumentStrip } from '@/components/hud/InstrumentStrip';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { setUplinkSyncAction } from './_actions/set-device-sync';

export const metadata = { title: "Devices" };

interface SearchParams {
  code?: string;
  expires?: string;
  error?: string;
  device?: string;
}

const ACTIVITY_LIMIT = 25;

const mainStyle: React.CSSProperties = {
  maxWidth: 'none',
  margin: 0,
  padding: 0,
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
};

const headerSubtitleStyle: React.CSSProperties = {
  margin: '6px 0 0',
  color: 'var(--fg-muted)',
  fontSize: 14,
};

const stepGridStyle: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))',
  gap: 16,
};

const cardHeaderStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 4,
  marginBottom: 12,
};

const cardTitleStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 15,
  fontWeight: 600,
  letterSpacing: '-0.01em',
};

const formStyle: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
  margin: 0,
};

const formActionsStyle: React.CSSProperties = {
  display: 'flex',
  marginTop: 4,
};

const pairCodeFrameStyle: React.CSSProperties = {
  background: 'var(--bg-elev)',
  border: '1px dashed var(--accent)',
  borderRadius: 'var(--r-md)',
  padding: '26px 24px',
  textAlign: 'center',
};

const pairCodeStyle: React.CSSProperties = {
  fontSize: 38,
  color: 'var(--accent)',
  letterSpacing: '0.18em',
  fontWeight: 500,
  userSelect: 'all',
};

const pairCodeMetaStyle: React.CSSProperties = {
  marginTop: 12,
  color: 'var(--fg-dim)',
  fontSize: 12,
};

const pairCodeFootnoteStyle: React.CSSProperties = {
  marginTop: 12,
  color: 'var(--fg-dim)',
  fontSize: 12,
};

const emptyStyle: React.CSSProperties = {
  textAlign: 'center',
  padding: '40px 20px',
  color: 'var(--fg-muted)',
  fontSize: 14,
};

const emptyTitleStyle: React.CSSProperties = {
  fontSize: 16,
  color: 'var(--fg)',
  marginBottom: 6,
};

const sessionFootStyle: React.CSSProperties = {
  marginTop: 12,
  color: 'var(--fg-muted)',
  fontSize: 13,
};

const syncToggleFormStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  marginTop: 12,
};

const syncToggleLabelStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 6,
  fontSize: 12,
  cursor: 'pointer',
};

const syncToggleSpanStyle: React.CSSProperties = {
  textTransform: 'uppercase',
  letterSpacing: '0.08em',
};

const tabStripStyle: React.CSSProperties = {
  display: 'flex',
  gap: 4,
  flexWrap: 'wrap',
  borderBottom: '1px solid var(--border)',
};

const factGridStyle: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))',
  gap: 16,
  marginBottom: 20,
};

const factLabelStyle: React.CSSProperties = {
  fontSize: 11,
  color: 'var(--fg-dim)',
  textTransform: 'uppercase',
  letterSpacing: '0.06em',
  marginBottom: 4,
};

const factValueStyle: React.CSSProperties = {
  fontSize: 14,
  color: 'var(--fg)',
};

export default async function DevicesPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const session = await getSession();
  if (!session) redirect('/auth/login?next=/devices');

  const { code, expires, error, device: selectedParam } =
    await props.searchParams;

  // Pull the current device list. If the token has expired (401),
  // bounce to login rather than rendering a half-broken page.
  let devices: Awaited<ReturnType<typeof listDevices>> = { devices: [] };
  try {
    devices = await listDevices(session.token);
  } catch (e) {
    if (e instanceof ApiCallError && e.status === 401) {
      redirect('/auth/login?next=/devices');
    }
    throw e;
  }

  async function pairAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/devices');

    const label = String(formData.get('label') ?? '').trim();
    try {
      const pairing = await startPairing(session.token, { label });
      const params = new URLSearchParams({
        code: pairing.code,
        expires: pairing.expires_at,
      });
      redirect(`/devices?${params.toString()}`);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/devices');
      }
      throw e;
    }
  }

  async function revokeAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/devices');

    const id = String(formData.get('device_id') ?? '');
    if (!id) redirect('/devices?error=missing_id');

    try {
      await revokeDevice(session.token, id);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/devices');
      }
      throw e;
    }
    redirect('/devices');
  }

  const deviceList = devices.devices;
  const pairedCount = deviceList.length;
  // Default tab = first device unless the URL pinned a specific one.
  const activeDevice =
    deviceList.find((d) => d.id === selectedParam) ?? deviceList[0] ?? null;

  // Activity feed is scoped to the active device tab via the
  // `device_id` query param (server-side filter on the audit payload
  // stamped at ingest time). We only fetch when at least one device
  // exists (otherwise the section is moot).
  let activity: IngestHistoryResponse | null = null;
  if (activeDevice) {
    try {
      activity = await getIngestHistory(session.token, {
        limit: ACTIVITY_LIMIT,
        deviceId: activeDevice.id,
      });
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/devices');
      }
      // Non-fatal — render the tab without activity.
      activity = { batches: [] };
    }
  }

  return (
    <main className="ss-screen-enter" style={mainStyle}>
      <InstrumentStrip
        title={<h1 className="hud-tile__title" style={{ margin: 0, fontSize: 18 }}>Devices</h1>}
        context="Uplinks · paired clients"
      />
      <p style={headerSubtitleStyle}>
        Run the StarStats tray app, click <em>Pair</em>, type the code below.
        Codes expire in 5 minutes and burn on first use.
      </p>
      <p style={{ margin: '8px 0 0', color: 'var(--fg-muted)', fontSize: 12 }}>
        Toggling an uplink off here immediately stops it from syncing. To
        turn it back on, you&apos;ll also need to enable it from the uplink
        itself.
      </p>

      {error && (
        <div className="ss-alert ss-alert--danger" role="alert">
          Couldn&apos;t complete that action. Try again.
        </div>
      )}

      <div style={stepGridStyle}>
        <section className="ss-card ss-card-pad">
          <div style={cardHeaderStyle}>
            <span className="ss-eyebrow">Step 1</span>
            <h2 style={cardTitleStyle}>Generate a pairing code</h2>
          </div>
          <form action={pairAction} style={formStyle}>
            <label className="ss-label">
              <span className="ss-label-text">Device label</span>
              <input
                className="ss-input"
                type="text"
                name="label"
                placeholder="Daisy's gaming PC"
                spellCheck={false}
                autoComplete="off"
              />
              <small style={{ color: 'var(--fg-dim)', fontSize: 12 }}>
                Optional — helps you tell devices apart in the list.
              </small>
            </label>
            <div style={formActionsStyle}>
              <button type="submit" className="ss-btn ss-btn--primary">
                Generate pairing code
              </button>
            </div>
          </form>
        </section>

        <section className="ss-card ss-card-pad">
          <div style={cardHeaderStyle}>
            <span className="ss-eyebrow">
              {code ? 'Active code' : 'Awaiting code'}
            </span>
            <h2 style={cardTitleStyle}>Paste this into the tray app</h2>
          </div>
          {code ? (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              <div style={pairCodeFrameStyle}>
                <div className="mono ss-pair-code" style={pairCodeStyle}>
                  {code}
                </div>
                {expires && (
                  <div style={pairCodeMetaStyle}>
                    Expires{' '}
                    <span
                      className="mono"
                      style={{ color: 'var(--fg-muted)' }}
                    >
                      <ExpiryRelative iso={expires} />
                    </span>
                  </div>
                )}
              </div>
              <div style={pairCodeFootnoteStyle}>
                Each code is single-use. Generate a new one if it expires.
              </div>
            </div>
          ) : (
            <div
              style={{
                ...pairCodeFrameStyle,
                borderStyle: 'dashed',
                borderColor: 'var(--border-strong)',
              }}
            >
              <div
                className="mono ss-pair-code"
                style={{ ...pairCodeStyle, color: 'var(--fg-dim)' }}
              >
                ———
              </div>
              <div style={pairCodeMetaStyle}>
                Generate a code on the left to begin.
              </div>
            </div>
          )}
        </section>
      </div>

      <section className="ss-card ss-card-pad">
        <div style={cardHeaderStyle}>
          <span className="ss-eyebrow">Connected</span>
          <h2 style={cardTitleStyle}>
            Paired devices ({pairedCount})
          </h2>
        </div>

        {pairedCount === 0 || !activeDevice ? (
          <div style={emptyStyle}>
            <div style={emptyTitleStyle}>No devices yet</div>
            <div>
              Generate a pairing code above, then run the StarStats tray and
              click <em>Pair</em>. Don&apos;t have the app yet?{' '}
              <Link href="/downloads" style={{ color: 'var(--accent)' }}>
                Download the tray client
              </Link>
              .
            </div>
          </div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
            <DeviceTabStrip devices={deviceList} activeId={activeDevice.id} />
            <DeviceFacts device={activeDevice} revokeAction={revokeAction} syncAction={setUplinkSyncAction} />
            <DeviceActivity
              device={activeDevice}
              batches={activity?.batches ?? []}
            />
          </div>
        )}
      </section>

      <p style={sessionFootStyle}>
        Signed in as <strong>{session.claimedHandle}</strong>.
      </p>
    </main>
  );
}

// -- Tab strip ------------------------------------------------------

function DeviceTabStrip({
  devices,
  activeId,
}: {
  devices: DeviceDto[];
  activeId: string;
}) {
  return (
    <nav style={tabStripStyle} aria-label="Paired devices">
      {devices.map((d) => {
        const isActive = d.id === activeId;
        const href = (`/devices?device=${encodeURIComponent(d.id)}`) as Route;
        const isOnline = isDeviceOnline(d);
        return (
          <Link
            key={d.id}
            href={href}
            style={{
              padding: '10px 14px',
              fontSize: 13,
              fontWeight: 500,
              color: isActive ? 'var(--fg)' : 'var(--fg-muted)',
              borderBottom: isActive
                ? '2px solid var(--accent)'
                : '2px solid transparent',
              textDecoration: 'none',
              marginBottom: -1,
              display: 'inline-flex',
              alignItems: 'center',
              gap: 8,
            }}
          >
            <span className="mono">{d.label || 'unlabeled'}</span>
            {isOnline && (
              <span
                style={{
                  background: 'var(--ok)',
                  width: 7,
                  height: 7,
                  borderRadius: '50%',
                  display: 'inline-block',
                }}
                aria-label="online"
              />
            )}
          </Link>
        );
      })}
    </nav>
  );
}

// -- Device facts (pairing info) -----------------------------------

function DeviceFacts({
  device,
  revokeAction,
  syncAction,
}: {
  device: DeviceDto;
  revokeAction: (formData: FormData) => Promise<void>;
  syncAction: (formData: FormData) => Promise<void>;
}) {
  const online = isDeviceOnline(device);
  return (
    <div>
      <div style={factGridStyle}>
        <Fact label="Label">
          <span className="mono">{device.label || 'unlabeled'}</span>
        </Fact>
        <Fact label="Status">
          {online ? (
            <span className="ss-badge ss-badge--ok">
              <span className="ss-badge-dot" />
              Online
            </span>
          ) : (
            <span style={{ color: 'var(--fg-muted)' }}>Offline</span>
          )}
        </Fact>
        <Fact label="Paired">
          <span style={{ color: 'var(--fg-muted)' }}>
            <RelativeTime iso={device.created_at} />
          </span>
        </Fact>
        <Fact label="Last seen">
          {device.last_seen_at ? (
            <span style={{ color: 'var(--fg-muted)' }}>
              <RelativeTime iso={device.last_seen_at} />
            </span>
          ) : (
            <span style={{ color: 'var(--fg-dim)' }}>never</span>
          )}
        </Fact>
        <Fact label="Device ID">
          <span
            className="mono"
            style={{ fontSize: 12, color: 'var(--fg-dim)' }}
            title={device.id}
          >
            {shortenId(device.id)}
          </span>
        </Fact>
      </div>
      <form action={syncAction} style={syncToggleFormStyle}>
        <input type="hidden" name="device_id" value={device.id} />
        <label style={syncToggleLabelStyle}>
          <input
            type="checkbox"
            name="enabled"
            defaultChecked={device.sync_enabled ?? false}
          />
          <span style={syncToggleSpanStyle}>Cloud sync</span>
        </label>
        <button
          type="submit"
          className="ss-btn"
          style={{ padding: '6px 14px', fontSize: 12 }}
        >
          Save
        </button>
      </form>
      <form action={revokeAction} style={{ margin: '8px 0 0' }}>
        <input type="hidden" name="device_id" value={device.id} />
        <ConfirmSubmitButton
          confirm="Revoke this device? It will stop syncing and must be re-paired from the tray to reconnect."
          pendingLabel="Revoking…"
          style={{ padding: '6px 14px', fontSize: 12 }}
        >
          Revoke this device
        </ConfirmSubmitButton>
      </form>
    </div>
  );
}

function Fact({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div style={factLabelStyle}>{label}</div>
      <div style={factValueStyle}>{children}</div>
    </div>
  );
}

// -- Activity (batches) --------------------------------------------

function DeviceActivity({
  device,
  batches,
}: {
  device: DeviceDto;
  batches: IngestBatchDto[];
}) {
  return (
    <section
      style={{
        borderTop: '1px solid var(--border)',
        paddingTop: 20,
      }}
    >
      <div style={cardHeaderStyle}>
        <span className="ss-eyebrow">Activity</span>
        <h3 style={{ ...cardTitleStyle, fontSize: 16 }}>
          Recent ingest batches
        </h3>
        <p style={{ margin: '4px 0 0', color: 'var(--fg-dim)', fontSize: 12 }}>
          Per-batch metadata only — raw lines are not retained. Showing the
          most recent {ACTIVITY_LIMIT} batches from{' '}
          <span className="mono">{device.label || 'this device'}</span>.
        </p>
      </div>
      {batches.length === 0 ? (
        <div style={emptyStyle}>
          <div style={emptyTitleStyle}>No batches yet</div>
          <div>
            Once{' '}
            <span className="mono">{device.label || 'this device'}</span>{' '}
            posts an ingest batch it will appear here.
          </div>
        </div>
      ) : (
        <div className="ss-table-wrap">
          <table className="ss-table" style={{ fontSize: 13 }}>
            <thead>
              <tr>
                <th style={{ textAlign: 'left' }}>When</th>
                <th style={{ textAlign: 'left' }}>Batch</th>
                <th style={{ textAlign: 'left' }}>Build</th>
                <th style={{ textAlign: 'right' }}>Total</th>
                <th style={{ textAlign: 'right' }}>Accepted</th>
                <th style={{ textAlign: 'right' }}>Duplicate</th>
                <th style={{ textAlign: 'right' }}>Rejected</th>
              </tr>
            </thead>
            <tbody>
              {batches.map((b) => (
                <BatchRow key={b.seq} batch={b} />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function BatchRow({ batch }: { batch: IngestBatchDto }) {
  const rejectionPct =
    batch.total > 0 ? (batch.rejected / batch.total) * 100 : 0;
  return (
    <tr>
      <td style={{ color: 'var(--fg-muted)' }}>
        {formatRelativeTime(batch.occurred_at)}
      </td>
      <td>
        <span
          className="mono"
          style={{ fontSize: 12, color: 'var(--fg-dim)' }}
          title={batch.batch_id}
        >
          {shortenBatchId(batch.batch_id)}
        </span>
      </td>
      <td className="mono" style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
        {batch.game_build ?? '—'}
      </td>
      <td className="mono" style={{ textAlign: 'right' }}>
        {batch.total.toLocaleString()}
      </td>
      <td
        className="mono"
        style={{ textAlign: 'right', color: 'var(--ok)' }}
      >
        {batch.accepted.toLocaleString()}
      </td>
      <td
        className="mono"
        style={{ textAlign: 'right', color: 'var(--fg-dim)' }}
      >
        {batch.duplicate.toLocaleString()}
      </td>
      <td
        className="mono"
        style={{
          textAlign: 'right',
          color:
            batch.rejected > 0
              ? rejectionPct > 5
                ? 'var(--danger)'
                : 'var(--warn)'
              : 'var(--fg-dim)',
        }}
      >
        {batch.rejected.toLocaleString()}
      </td>
    </tr>
  );
}

// -- Time / id helpers ---------------------------------------------

function isDeviceOnline(d: DeviceDto): boolean {
  if (!d.last_seen_at) return false;
  return Date.now() - Date.parse(d.last_seen_at) < 5 * 60 * 1000;
}

function shortenId(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 8)}…${id.slice(-3)}`;
}

function shortenBatchId(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 8)}…${id.slice(-3)}`;
}

function formatRelativeTime(iso: string): string {
  const ts = new Date(iso).getTime();
  if (Number.isNaN(ts)) return iso;
  const diffMs = Date.now() - ts;
  if (diffMs < 60_000) return 'just now';
  if (diffMs < 3_600_000) return `${Math.floor(diffMs / 60_000)}m ago`;
  if (diffMs < 86_400_000) return `${Math.floor(diffMs / 3_600_000)}h ago`;
  if (diffMs < 7 * 86_400_000) return `${Math.floor(diffMs / 86_400_000)}d ago`;
  return new Date(iso).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
  });
}

function ExpiryRelative({ iso }: { iso: string }) {
  const seconds = Math.max(
    0,
    Math.round((Date.parse(iso) - Date.now()) / 1000),
  );
  if (seconds < 60) return <>in {seconds}s</>;
  return <>in ~{Math.round(seconds / 60)}m</>;
}

function RelativeTime({ iso }: { iso: string }) {
  const seconds = Math.max(
    0,
    Math.round((Date.now() - Date.parse(iso)) / 1000),
  );
  if (seconds < 60) return <>just now</>;
  if (seconds < 3600) return <>{Math.round(seconds / 60)}m ago</>;
  if (seconds < 86400) return <>{Math.round(seconds / 3600)}h ago</>;
  return <>{Math.round(seconds / 86400)}d ago</>;
}
