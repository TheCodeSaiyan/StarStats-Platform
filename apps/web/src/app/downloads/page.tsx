/**
 * `/downloads` — the Emitter.
 *
 * ONE SURFACE FOR THE WHOLE CLIENT LIFECYCLE. This route absorbed `/devices`:
 * download the emitter, pair it to your account, watch what it sends, revoke
 * it. The design system names the desktop client the emitter and treats those
 * as one thing — its own Emitter screen lists "Pair this machine to your
 * account" as a step of installing, and COVERAGE files the uplink table with
 * the client rather than as a destination of its own. `/devices` now redirects
 * here, so every inbound link still lands somewhere correct.
 *
 * It is also what freed the word **Hangar**, which was pointing at paired
 * devices while the actual hangar — the RSI fleet — lives in the `hangar` and
 * `fleet` widgets and the fleet pane on `/settings`.
 *
 * PUBLIC, PARTIALLY. Anyone may read the download half signed out. The pairing
 * and uplink groups are built ONLY when there is a session: the access model
 * forbids showing a signed-out visitor even the LABEL of something they cannot
 * open, and the lens rail is built from the groups this render produces.
 *
 * Everything below the download half is lifted from `/devices` unchanged —
 * the pairing flow, per-device tabs as real links, the two-gate cloud-sync
 * consent, revocation, and the per-batch-metadata-only activity table. Those
 * judgements are load-bearing and are documented at their call sites.
 */
import type { Metadata } from 'next';
import Link from 'next/link';
import type { Route } from 'next';
import { redirect } from 'next/navigation';
import {
  Plane,
  BeamButton,
  BeamChip,
  BeamInput,
  Flatline,
  HoloKV,
  HoloTable,
  type Calibration,
} from 'holo';
import {
  ApiCallError,
  getIngestHistory,
  listDevices,
  revokeDevice,
  startPairing,
  type DeviceDto,
  type IngestHistoryResponse,
} from '@/lib/api';
import { getSession } from '@/lib/session';
import { getTheme } from '@/lib/theme';
import { navSections } from '@/lib/nav';
import { setCalibrationAction } from '@/app/me/_projection/actions';
import { ConfirmSubmitButton } from '@/components/forms/ConfirmSubmitButton';
import { fetchTrayReleases } from '@/lib/github-releases.server';
import type { TrayReleaseSet } from '@/lib/github-releases';
import { setUplinkSyncAction } from '@/app/devices/_actions/set-device-sync';
import { ReleasePlanes } from './_projection/ReleasePlanes';
import {
  EmitterProjection,
  type EmitterSection,
} from './_projection/EmitterProjection';
// Plain module, not the client one — a server component reading `.key` off a
// `'use client'` export gets a client reference, not the value.
import {
  EMITTER_GROUP,
  PAIR_GROUP,
  UPLINKS_GROUP,
} from './_projection/groups';

export const metadata: Metadata = {
  title: 'Emitter',
  description:
    'Download the StarStats tray client — the local app that reads your Star Citizen Game.log. Windows and Linux builds, pulled live from the latest release.',
};

interface SearchParams {
  code?: string;
  expires?: string;
  error?: string;
  device?: string;
}

const ACTIVITY_LIMIT = 25;

export default async function EmitterPage(props: {
  searchParams: Promise<SearchParams>;
}) {
  const { code, expires, error, device: selectedParam } =
    await props.searchParams;

  // No redirect for an absent session — this surface is public. Everything
  // below simply narrows to the download half.
  const session = await getSession();

  let calibration: Calibration = 'terra';
  try {
    calibration = (await getTheme(session?.token)) as Calibration;
  } catch {
    // Preference read failed; the default stands.
  }

  let set: TrayReleaseSet = { stable: null, prerelease: null };
  let releasesFailed = false;
  try {
    set = await fetchTrayReleases();
  } catch (err) {
    // Log the real cause (GitHub outage / rate-limit / bad shape) so a
    // "Couldn't reach the release feed" state is diagnosable server-side —
    // mirrors the fetcher pattern in lib/reference.ts.
    console.error('tray releases fetch failed', err);
    releasesFailed = true;
  }

  // Pull the current device list. A 401 means the cookie outlived the token,
  // so bounce to login rather than rendering a half-broken page — but only
  // for someone who HAD a session; a visitor just gets the download half.
  let deviceList: DeviceDto[] = [];
  if (session) {
    try {
      deviceList = (await listDevices(session.token)).devices;
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/downloads');
      }
      throw e;
    }
  }

  async function pairAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/downloads');

    const label = String(formData.get('label') ?? '').trim();
    try {
      const pairing = await startPairing(session.token, { label });
      const params = new URLSearchParams({
        code: pairing.code,
        expires: pairing.expires_at,
      });
      redirect(`/downloads?${params.toString()}`);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/downloads');
      }
      throw e;
    }
  }

  async function revokeAction(formData: FormData) {
    'use server';
    const session = await getSession();
    if (!session) redirect('/auth/login?next=/downloads');

    const id = String(formData.get('device_id') ?? '');
    if (!id) redirect('/downloads?error=missing_id');

    try {
      await revokeDevice(session.token, id);
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/downloads');
      }
      throw e;
    }
    redirect('/downloads');
  }

  const pairedCount = deviceList.length;
  // Default tab = first device unless the URL pinned a specific one.
  const activeDevice =
    deviceList.find((d) => d.id === selectedParam) ?? deviceList[0] ?? null;

  // Activity feed is scoped to the active device tab via the `device_id` query
  // param (server-side filter on the audit payload stamped at ingest time). We
  // only fetch when at least one device exists (otherwise the section is moot).
  let activity: IngestHistoryResponse | null = null;
  if (session && activeDevice) {
    try {
      activity = await getIngestHistory(session.token, {
        limit: ACTIVITY_LIMIT,
        deviceId: activeDevice.id,
      });
    } catch (e) {
      if (e instanceof ApiCallError && e.status === 401) {
        redirect('/auth/login?next=/downloads');
      }
      // Non-fatal — render the tab without activity.
      activity = { batches: [] };
    }
  }

  const sections: EmitterSection[] = [
    {
      id: 'emitter',
      title: 'Emitter',
      ctx: 'The client that reads your log',
      group: EMITTER_GROUP.key,
      node: (
        <>
          <p className="hp-prose">
            A small desktop app that reads what Star Citizen already writes to
            its <code>Game.log</code> — sessions, travel, loadouts — and keeps
            it on your machine until you sign in and turn on sync. Pick your
            platform below; the tray keeps itself up to date after that.
          </p>
          <ReleasePlanes
            stable={set.stable}
            prerelease={set.prerelease}
            error={releasesFailed}
          />
        </>
      ),
    },

    {
      id: 'after-install',
      title: 'After install',
      ctx: 'three steps',
      group: EMITTER_GROUP.key,
      node: (
        <>
          <HoloKV
            items={[
              { k: 'Point Game.log at your install', v: 'required' },
              { k: 'Pair this machine to your account', v: 'optional' },
              { k: 'Paste an RSI session cookie', v: 'optional' },
            ]}
          />
          <p className="hp-prose">
            {session ? (
              <>Pairing lives under <b>Pair</b>, above.</>
            ) : (
              <>
                <Link href={'/auth/login?next=/downloads' as Route}>
                  Sign in
                </Link>{' '}
                to pair this machine — the tray parses your log either way, and
                nothing leaves it until you turn sync on.
              </>
            )}
          </p>
        </>
      ),
    },

    // ------------------------------------------------------------------
    // Signed-in only. Built conditionally, not hidden with CSS: the lens rail
    // is derived from these sections, so an absent session must not produce a
    // "Pair" or "Uplinks" label at all.
    // ------------------------------------------------------------------
    ...(session
      ? ([
          {
            id: 'pair',
            title: 'Generate a pairing code',
            ctx: 'Step 1',
            group: PAIR_GROUP.key,
            node: (
              <>
                <p className="hp-prose">
                  Run the StarStats tray, click <em>Pair</em>, and type the code
                  below. Codes expire in 5 minutes and burn on first use.
                </p>
                <form action={pairAction} className="hp-formcol">
                  <BeamInput
                    id="device-label"
                    label="Device label"
                    type="text"
                    name="label"
                    placeholder="Daisy's gaming PC"
                    spellCheck={false}
                    autoComplete="off"
                    hint="Optional — helps you tell devices apart in the list."
                  />
                  <BeamButton
                    type="submit"
                    variant="primary"
                    style={{ alignSelf: 'flex-start' }}
                  >
                    Generate pairing code
                  </BeamButton>
                </form>
              </>
            ),
          },

          {
            id: 'code',
            title: 'Paste this into the tray',
            ctx: code ? 'Active code' : 'Awaiting code',
            group: PAIR_GROUP.key,
            node: code ? (
              <>
                {/* A code read off one screen and typed into another, so it is
                    set wide and lit — mis-transcribing it is the failure
                    mode. */}
                <div className="hp-paircode">{code}</div>
                {expires ? (
                  <p className="hp-prose">
                    Expires <ExpiryRelative iso={expires} />.
                  </p>
                ) : null}
                <p className="hp-prose">
                  Each code is single-use. Generate a new one if it expires.
                </p>
              </>
            ) : (
              <>
                <div className="hp-paircode hp-paircode--idle">———</div>
                <p className="hp-prose">Generate a code to begin.</p>
              </>
            ),
          },

          {
            id: 'uplinks',
            title: `Paired devices (${pairedCount})`,
            ctx: 'Connected',
            group: UPLINKS_GROUP.key,
            node:
              pairedCount === 0 || !activeDevice ? (
                // The flat page offered a "Get the tray app →" action here.
                // It pointed at `/downloads` — which is now this same page, a
                // group away — so the affordance would send a reader in a
                // circle. The hint already names the two steps.
                <Flatline
                  title="No devices yet"
                  reason="no-signal"
                  hint="Generate a pairing code, then run the StarStats tray and click Pair."
                />
              ) : (
                <>
                  <p className="hp-prose">
                    Toggling an uplink off here stops it syncing immediately. To
                    turn it back on you will also need to enable it from the
                    uplink itself — neither side can force the other.
                  </p>

                  {/* Per-device tabs. Real links: selecting one re-fetches that
                      device's batches server-side. */}
                  <nav className="hp-devtabs" aria-label="Paired devices">
                    {deviceList.map((d) => (
                      <Link
                        key={d.id}
                        href={
                          `/downloads?device=${encodeURIComponent(d.id)}` as Route
                        }
                        aria-current={
                          d.id === activeDevice.id ? 'page' : undefined
                        }
                      >
                        {d.label || 'unlabeled'}
                        {isDeviceOnline(d) ? <i aria-label="online" /> : null}
                      </Link>
                    ))}
                  </nav>

                  <div style={{ marginTop: 18 }}>
                    <HoloKV
                      items={[
                        { k: 'Label', v: activeDevice.label || 'unlabeled' },
                        {
                          k: 'Status',
                          v: isDeviceOnline(activeDevice) ? (
                            <BeamChip tone="good" dot>
                              Online
                            </BeamChip>
                          ) : (
                            <BeamChip>Offline</BeamChip>
                          ),
                        },
                        {
                          k: 'Paired',
                          v: <RelativeTime iso={activeDevice.created_at} />,
                        },
                        {
                          k: 'Last seen',
                          v: activeDevice.last_seen_at ? (
                            <RelativeTime iso={activeDevice.last_seen_at} />
                          ) : (
                            'never'
                          ),
                        },
                        {
                          k: 'Device ID',
                          v: (
                            <span title={activeDevice.id}>
                              {shortenId(activeDevice.id)}
                            </span>
                          ),
                        },
                      ]}
                    />
                  </div>

                  {/* Two-gate model: this is the SERVER-side half of the
                      consent. The uplink holds its own local intent, and both
                      must be true for a sync to happen. Either side can
                      withdraw; neither can force the other — the same shape as
                      SSH keys and authorized_keys. */}
                  <Plane tilt="flat" cap="Cloud sync" style={{ marginTop: 20 }}>
                    <form action={setUplinkSyncAction} className="hp-formrow">
                      <input
                        type="hidden"
                        name="device_id"
                        value={activeDevice.id}
                      />
                      <label className="hp-check">
                        <input
                          type="checkbox"
                          name="enabled"
                          defaultChecked={activeDevice.sync_enabled ?? false}
                        />
                        <span>Allow this uplink to sync</span>
                      </label>
                      <BeamButton type="submit">Save</BeamButton>
                    </form>
                  </Plane>

                  <form action={revokeAction} style={{ marginTop: 20 }}>
                    <input
                      type="hidden"
                      name="device_id"
                      value={activeDevice.id}
                    />
                    <ConfirmSubmitButton
                      confirm="Revoke this device? It will stop syncing and must be re-paired from the tray to reconnect."
                      pendingLabel="Revoking…"
                      className="hp-btn hp-btn--danger"
                    >
                      Revoke this device
                    </ConfirmSubmitButton>
                  </form>
                </>
              ),
          },
        ] satisfies EmitterSection[])
      : []),

    ...(session && activeDevice
      ? [
          {
            id: 'activity',
            title: 'Recent ingest batches',
            ctx: 'Activity',
            group: UPLINKS_GROUP.key,
            node: (
              <>
                <p className="hp-prose">
                  {/* The no-raw-retention stance is hard, and saying so here is
                      the point: per-batch metadata only, with no drill-down
                      affordance by design. */}
                  Per-batch metadata only — raw lines are not retained. Showing
                  the most recent {ACTIVITY_LIMIT} batches from{' '}
                  <span className="val">
                    {activeDevice.label || 'this device'}
                  </span>
                  .
                </p>
                {(activity?.batches ?? []).length === 0 ? (
                  <Flatline
                    title="No batches yet"
                    reason="no-data"
                    hint={`Once ${activeDevice.label || 'this device'} posts an ingest batch it will appear here.`}
                  />
                ) : (
                  <Plane tilt="flat" cap="Batches" style={{ marginTop: 18 }}>
                    <HoloTable
                      columns={[
                        { key: 'when', label: 'When' },
                        { key: 'batch', label: 'Batch' },
                        { key: 'build', label: 'Build' },
                        { key: 'total', label: 'Total', numeric: true },
                        { key: 'accepted', label: 'Accepted', numeric: true },
                        { key: 'duplicate', label: 'Duplicate', numeric: true },
                        { key: 'rejected', label: 'Rejected', numeric: true },
                      ]}
                      rows={(activity?.batches ?? []).map((b) => ({
                        key: String(b.seq),
                        when: formatRelativeTime(b.occurred_at),
                        batch: (
                          <span title={b.batch_id}>
                            {shortenBatchId(b.batch_id)}
                          </span>
                        ),
                        build: b.game_build ?? '—',
                        total: b.total.toLocaleString(),
                        accepted: b.accepted.toLocaleString(),
                        duplicate: b.duplicate.toLocaleString(),
                        rejected: b.rejected.toLocaleString(),
                      }))}
                    />
                  </Plane>
                )}
              </>
            ),
          } satisfies EmitterSection,
        ]
      : []),
  ];

  return (
    <EmitterProjection
      handle={session?.claimedHandle}
      calibration={calibration}
      nav={navSections(
        { signedIn: Boolean(session), staffRoles: session?.staffRoles },
        'downloads',
      )}
      groups={
        session
          ? [EMITTER_GROUP, PAIR_GROUP, UPLINKS_GROUP]
          : [EMITTER_GROUP]
      }
      sections={sections}
      notice={
        error
          ? {
              tone: 'bad' as const,
              message: "Couldn't complete that action. Try again.",
            }
          : null
      }
      onCalibrate={async (id: string) => {
        'use server';
        await setCalibrationAction(id);
      }}
    />
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
