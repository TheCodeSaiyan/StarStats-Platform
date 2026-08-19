import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, act, within } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  Config,
  OrgBearerStatus,
  StatusResponse,
  SyncStats,
} from '../api';
import { FieldFocusProvider } from '../hooks/useFieldFocus';
import { SettingsPane } from './SettingsPane';
import type { ReactNode } from 'react';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);

// Wrap with FieldFocusProvider which SettingsPane requires.
function wrap(ui: ReactNode) {
  return render(<FieldFocusProvider>{ui}</FieldFocusProvider>);
}

function makeConfig(overrides: Partial<Config> = {}): Config {
  return {
    gamelog_path: null,
    web_origin: null,
    auto_update_check: true,
    release_channel: 'live',
    debug_logging: false,
    parser_enable_v2_metadata: false,
    theme: 'stanton',
    sync_with_cloud: false,
    autostart_enabled: null,
    channel_mismatch_ack: null,
    remote_sync: {
      enabled: false,
      api_url: null,
      access_token: null,
      claimed_handle: null,
      priority_interval_secs: 5,
      interval_secs: 60,
      batch_size: 200,
      priority_event_types: [],
    },
    org_connector: {
      enabled: false,
      platform_url: null,
      bearer_token: null,
    },
    ...overrides,
  } as Config;
}

// SettingsPane mounts AutostartToggle and probes RSI cookie status +
// org bearer-token status + app version on mount. Mock enough to prevent
// unhandled-rejection noise while not caring about those side-effects in
// these tests. `bearer` sets what the on-mount `get_org_bearer_status`
// probe resolves to (default: not configured).
function stubInvoke(
  opts: { bearer?: OrgBearerStatus } = {},
) {
  const bearer: OrgBearerStatus = opts.bearer ?? {
    configured: false,
    preview: null,
  };
  mockedInvoke.mockImplementation((cmd: string) => {
    if (cmd === 'get_rsi_cookie_status') return Promise.resolve({ configured: false, preview: null });
    if (cmd === 'get_org_bearer_status') return Promise.resolve(bearer);
    if (cmd === 'set_org_bearer') return Promise.resolve({ configured: true, preview: '…3456' });
    if (cmd === 'clear_org_bearer') return Promise.resolve({ configured: false, preview: null });
    if (cmd === 'get_app_version') return Promise.resolve('0.0.0-test');
    if (cmd === 'get_build_release_channel') return Promise.resolve('live');
    if (cmd === 'get_autostart_enabled') return Promise.resolve(false);
    return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
  });
}

describe('SettingsPane Cloud sync card', () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    (listen as ReturnType<typeof vi.fn>).mockImplementation(() =>
      Promise.resolve(() => undefined)
    );
    stubInvoke();
  });

  it('renders the toggle disabled when unpaired', async () => {
    const cfg = makeConfig();
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);
    const toggle = await screen.findByLabelText(/Sync settings with your account/i);
    expect(toggle).toBeDisabled();
  });

  it('renders the toggle enabled when paired', async () => {
    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);
    const toggle = await screen.findByLabelText(/Sync settings with your account/i);
    expect(toggle).not.toBeDisabled();
    expect((toggle as HTMLInputElement).checked).toBe(false);
  });

  it('reads as paired from claimed_handle when access_token is null (M-T6 keychain)', async () => {
    // The real backend no longer serialises the device JWT to the UI (M-T6:
    // it's #[serde(skip)], living in the OS keychain). A genuinely paired
    // device therefore arrives with access_token=null but claimed_handle set.
    // Paired-state detection MUST derive from claimed_handle, not access_token
    // — otherwise the tray shows "unpaired" for a paired device on remount.
    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        access_token: null,
        claimed_handle: 'Daisy',
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);
    // isPaired → the sync toggle is enabled and the Unpair button (paired-view
    // only) renders.
    const toggle = await screen.findByLabelText(
      /Sync settings with your account/i,
    );
    expect(toggle).not.toBeDisabled();
    // The paired view (with the keychain note) renders only when isPaired.
    expect(
      await screen.findByText(/token stored in your OS keychain/i),
    ).toBeInTheDocument();
  });

  it('reflects sync_with_cloud=true', async () => {
    const cfg = makeConfig({
      sync_with_cloud: true,
      remote_sync: {
        ...makeConfig().remote_sync,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);
    const toggle = await screen.findByLabelText(/Sync settings with your account/i);
    expect((toggle as HTMLInputElement).checked).toBe(true);
  });

  it('clicking the toggle updates the draft state', async () => {
    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);
    const toggle = await screen.findByLabelText(/Sync settings with your account/i);
    fireEvent.click(toggle);
    expect((toggle as HTMLInputElement).checked).toBe(true);
  });
});

describe('SettingsPane capture toggle', () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    (listen as ReturnType<typeof vi.fn>).mockImplementation(() =>
      Promise.resolve(() => undefined),
    );
    stubInvoke();
  });

  it('reflects parser_enable_v2_metadata and flips it on click', async () => {
    const cfg = makeConfig({ parser_enable_v2_metadata: false });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);
    const toggle = await screen.findByLabelText(
      /Capture unrecognised log lines/i,
    );
    expect((toggle as HTMLInputElement).checked).toBe(false);
    fireEvent.click(toggle);
    expect((toggle as HTMLInputElement).checked).toBe(true);
  });
});

describe('SettingsPane sync revocation notice', () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    stubInvoke();
  });

  it('shows persistent notice after sync-revoked event fires', async () => {
    let revokeHandler: ((e: unknown) => void) | undefined;
    (listen as ReturnType<typeof vi.fn>).mockImplementation((event, handler) => {
      if (event === 'sync-revoked') revokeHandler = handler;
      return Promise.resolve(() => undefined);
    });

    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);

    // No notice initially.
    expect(screen.queryByText(/disabled for this uplink/i)).toBeNull();

    // Fire the event.
    await act(async () => {
      revokeHandler?.({});
    });

    // Notice now present.
    expect(await screen.findByText(/disabled for this uplink/i)).toBeInTheDocument();
  });

  it('clears notice when the user re-enables the toggle', async () => {
    let revokeHandler: ((e: unknown) => void) | undefined;
    (listen as ReturnType<typeof vi.fn>).mockImplementation((event, handler) => {
      if (event === 'sync-revoked') revokeHandler = handler;
      return Promise.resolve(() => undefined);
    });

    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);

    await act(async () => {
      revokeHandler?.({});
    });
    const toggle = await screen.findByLabelText(/Sync settings with your account/i);
    fireEvent.click(toggle);
    expect(screen.queryByText(/disabled for this uplink/i)).toBeNull();
  });
});

describe('SettingsPane remote-sync health indicator', () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    (listen as ReturnType<typeof vi.fn>).mockImplementation(() =>
      Promise.resolve(() => undefined)
    );
    stubInvoke();
  });

  function makeSync(overrides: Partial<SyncStats> = {}): SyncStats {
    return {
      last_attempt_at: null,
      last_success_at: null,
      last_error: null,
      batches_sent: 0,
      events_accepted: 0,
      events_duplicate: 0,
      events_rejected: 0,
      ...overrides,
    };
  }

  function makeStatus(sync: SyncStats): StatusResponse {
    return {
      tail: {
        current_path: null,
        bytes_read: 0,
        lines_processed: 0,
        events_recognised: 0,
        last_event_at: null,
        last_event_type: null,
        lines_structural_only: 0,
        lines_skipped: 0,
        lines_noise: 0,
      },
      sync,
      event_counts: [],
      total_events: 0,
      discovered_logs: [],
      account: { auth_lost: false, email_verified: null },
      hangar: {
        last_attempt_at: null,
        last_success_at: null,
        last_error: null,
        ships_pushed: 0,
        last_skip_reason: null,
      },
    };
  }

  // Locate the Remote sync card header indicator. The card sets
  // aria-label on its right-slot status group so tests don't have to
  // care about the surrounding card chrome.
  function getRemoteSyncHealth() {
    return screen.getByRole('status', { name: /remote sync health/i });
  }

  it('shows OK when sync has a recent success and no error', async () => {
    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        enabled: true,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    // Use a fresh timestamp so the staleness check (2× bulk interval)
    // doesn't fire — the OK pill requires both a recent success AND
    // a recent attempt.
    const nowIso = new Date().toISOString();
    const status = makeStatus(
      makeSync({
        last_success_at: nowIso,
        last_attempt_at: nowIso,
      }),
    );
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} status={status} />);
    expect(await screen.findByLabelText(/Sync settings/i)).toBeInTheDocument();
    expect(within(getRemoteSyncHealth()).getByText(/^OK$/)).toBeInTheDocument();
  });

  it('shows STALE when the last attempt is older than 2× the bulk interval and there is no error', async () => {
    // Regression for the 2026-05-28 outage: workers tripping the
    // auth_lost guard left sync_stats frozen on the last green
    // reading. The pill happily showed OK for 10+ hours despite
    // no traffic. STALE surfaces that silence to the user.
    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        enabled: true,
        access_token: 'tok',
        claimed_handle: 'U',
        interval_secs: 60,
      },
    });
    // Bulk interval = 60s ⇒ threshold = 120s. Use 10 minutes ago.
    const tenMinAgo = new Date(Date.now() - 10 * 60 * 1000).toISOString();
    const status = makeStatus(
      makeSync({
        last_success_at: tenMinAgo,
        last_attempt_at: tenMinAgo,
      }),
    );
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} status={status} />);
    expect(await screen.findByLabelText(/Sync settings/i)).toBeInTheDocument();
    expect(within(getRemoteSyncHealth()).getByText(/^STALE$/)).toBeInTheDocument();
  });

  it('shows PAUSED when the sync-paused event fires', async () => {
    let pausedHandler: ((e: unknown) => void) | undefined;
    (listen as ReturnType<typeof vi.fn>).mockImplementation((event, handler) => {
      if (event === 'sync-paused') pausedHandler = handler;
      return Promise.resolve(() => undefined);
    });

    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        enabled: true,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    const status = makeStatus(makeSync({
      last_success_at: new Date().toISOString(),
      last_attempt_at: new Date().toISOString(),
    }));
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} status={status} />);

    // Before the event: OK (recent activity).
    expect(within(getRemoteSyncHealth()).getByText(/^OK$/)).toBeInTheDocument();

    await act(async () => {
      pausedHandler?.({});
    });

    // After the event: PAUSED takes precedence over OK.
    expect(within(getRemoteSyncHealth()).getByText(/^PAUSED$/)).toBeInTheDocument();
    expect(
      await screen.findByText(/server rejected this uplink's token/i),
    ).toBeInTheDocument();
  });

  it('shows ERR when sync.last_error is set', async () => {
    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        enabled: true,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    const status = makeStatus(
      makeSync({
        last_attempt_at: '2026-05-21T12:00:00Z',
        last_error: 'connection refused',
      }),
    );
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} status={status} />);
    expect(await screen.findByLabelText(/Sync settings/i)).toBeInTheDocument();
    expect(within(getRemoteSyncHealth()).getByText(/^ERR$/)).toBeInTheDocument();
  });

  it('shows IDLE when paired + enabled but no traffic yet', async () => {
    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        enabled: true,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    const status = makeStatus(makeSync());
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} status={status} />);
    expect(await screen.findByLabelText(/Sync settings/i)).toBeInTheDocument();
    expect(within(getRemoteSyncHealth()).getByText(/^IDLE$/)).toBeInTheDocument();
  });

  it('shows OFF when remote_sync is disabled in config', async () => {
    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        enabled: false,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    const status = makeStatus(
      makeSync({ last_success_at: '2026-05-21T12:00:00Z' }),
    );
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} status={status} />);
    expect(await screen.findByLabelText(/Sync settings/i)).toBeInTheDocument();
    expect(within(getRemoteSyncHealth()).getByText(/^OFF$/)).toBeInTheDocument();
  });

  it('shows OFF when not paired', async () => {
    const cfg = makeConfig({
      remote_sync: { ...makeConfig().remote_sync, enabled: true },
    });
    const status = makeStatus(makeSync());
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} status={status} />);
    expect(await screen.findByLabelText(/Sync settings/i)).toBeInTheDocument();
    expect(within(getRemoteSyncHealth()).getByText(/^OFF$/)).toBeInTheDocument();
  });

  it('falls back to IDLE when status prop is omitted', async () => {
    const cfg = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        enabled: true,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);
    expect(await screen.findByLabelText(/Sync settings/i)).toBeInTheDocument();
    expect(within(getRemoteSyncHealth()).getByText(/^IDLE$/)).toBeInTheDocument();
  });
});

describe('SettingsPane unsaved-draft guard', () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    stubInvoke();
  });

  it('shows reload/keep notice when the config prop changes during an edit', async () => {
    const original = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    const { rerender } = wrap(<SettingsPane config={original} onSave={vi.fn()} />);

    // User edits theme locally — creates an unsaved draft.
    const pyroRadio = await screen.findByRole('radio', { name: /pyro/i });
    fireEvent.click(pyroRadio);

    // Parent receives a remote config-changed and re-renders with new prop.
    const remote: Config = { ...original, theme: 'nyx' };
    rerender(
      <FieldFocusProvider>
        <SettingsPane config={remote} onSave={vi.fn()} />
      </FieldFocusProvider>,
    );

    // Notice should appear with Reload + Keep my changes buttons.
    expect(
      await screen.findByText(/cloud settings changed while you were editing/i),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /reload/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /keep my changes/i })).toBeInTheDocument();

    // The user's draft must NOT be clobbered — theme picker still shows pyro selected.
    expect(pyroRadio.getAttribute('aria-checked')).toBe('true');
  });

  it('reload swaps to the incoming config', async () => {
    const original = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    const { rerender } = wrap(<SettingsPane config={original} onSave={vi.fn()} />);

    const pyroRadio = await screen.findByRole('radio', { name: /pyro/i });
    fireEvent.click(pyroRadio);

    const remote: Config = { ...original, theme: 'nyx' };
    rerender(
      <FieldFocusProvider>
        <SettingsPane config={remote} onSave={vi.fn()} />
      </FieldFocusProvider>,
    );

    fireEvent.click(screen.getByRole('button', { name: /reload/i }));

    // Notice gone; theme is now nyx.
    expect(screen.queryByText(/cloud settings changed/i)).toBeNull();
    const nyxRadio = screen.getByRole('radio', { name: /nyx/i });
    expect(nyxRadio.getAttribute('aria-checked')).toBe('true');
  });

  it('applies the new config prop silently when there is no unsaved draft', async () => {
    const original = makeConfig({
      remote_sync: {
        ...makeConfig().remote_sync,
        access_token: 'tok',
        claimed_handle: 'U',
      },
    });
    const { rerender } = wrap(<SettingsPane config={original} onSave={vi.fn()} />);

    // Make sure the form is fully mounted before the parent rerenders.
    await screen.findByRole('radio', { name: /stanton/i });

    const remote: Config = { ...original, theme: 'nyx' };
    rerender(
      <FieldFocusProvider>
        <SettingsPane config={remote} onSave={vi.fn()} />
      </FieldFocusProvider>,
    );

    // No notice (no unsaved draft).
    expect(screen.queryByText(/cloud settings changed/i)).toBeNull();
    // Theme is updated.
    const nyxRadio = await screen.findByRole('radio', { name: /nyx/i });
    expect(nyxRadio.getAttribute('aria-checked')).toBe('true');
  });
});

describe('SettingsPane org platform connector card', () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    (listen as ReturnType<typeof vi.fn>).mockImplementation(() =>
      Promise.resolve(() => undefined)
    );
    stubInvoke();
  });

  it('renders the enable toggle, URL and token inputs', async () => {
    const cfg = makeConfig();
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);

    const toggle = await screen.findByLabelText(
      /enable org platform connector/i
    );
    expect(toggle).toBeInTheDocument();
    expect((toggle as HTMLInputElement).checked).toBe(false);

    expect(
      screen.getByLabelText(/org connector bearer token/i)
    ).toBeInTheDocument();
    // Platform URL field renders too.
    expect(screen.getByPlaceholderText(/orgs\.example/i)).toBeInTheDocument();
  });

  it('renders the bearer token write-only: shows configured preview, never the stored value, no reveal toggle', async () => {
    stubInvoke({ bearer: { configured: true, preview: '…cret' } });
    const cfg = makeConfig({
      org_connector: {
        enabled: true,
        platform_url: 'https://orgs.example',
        // Backend now sends this as null (keychain-held); the form must
        // never bind or display it regardless.
        bearer_token: null,
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);

    const tokenInput = (await screen.findByLabelText(
      /org connector bearer token/i
    )) as HTMLInputElement;
    // Scope status assertions to the org card — the RSI cookie card also
    // renders a "…configured…" status line.
    const card = tokenInput.closest('section') as HTMLElement;

    // Status line reflects the keychain probe, showing only the redacted
    // preview tail — never the secret itself. The status renders after the
    // async on-mount get_org_bearer_status probe resolves, so AWAIT it
    // (findByText) — a synchronous getByText here raced the probe and flaked
    // on the slower CI runner.
    expect(await within(card).findByText(/configured/i)).toBeInTheDocument();
    expect(within(card).getByText('…cret')).toBeInTheDocument();

    // Always a masked password field, and the paste box starts empty —
    // the stored value is never round-tripped into the input.
    expect(tokenInput.type).toBe('password');
    expect(tokenInput.value).toBe('');

    // No reveal/eye toggle exists any more.
    expect(
      screen.queryByRole('button', { name: /show token/i })
    ).toBeNull();
    expect(
      screen.queryByRole('button', { name: /hide token/i })
    ).toBeNull();
  });

  it('disables the URL/token inputs when the connector is off', async () => {
    const cfg = makeConfig();
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);

    const urlInput = await screen.findByPlaceholderText(/orgs\.example/i);
    const tokenInput = screen.getByLabelText(/org connector bearer token/i);
    expect(urlInput).toBeDisabled();
    expect(tokenInput).toBeDisabled();
  });

  it('persists org_connector enabled + platform_url via onSave, but NOT the bearer token', async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const cfg = makeConfig();
    wrap(<SettingsPane config={cfg} onSave={onSave} />);

    // Enable, then fill the URL. The bearer token is managed out-of-band
    // (keychain commands), so it must NOT ride the config save path.
    const toggle = await screen.findByLabelText(
      /enable org platform connector/i
    );
    fireEvent.click(toggle);

    const urlInput = screen.getByPlaceholderText(/orgs\.example/i);
    fireEvent.change(urlInput, {
      target: { value: 'https://orgs.example' },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /save settings/i }));
    });

    expect(onSave).toHaveBeenCalledTimes(1);
    const saved = onSave.mock.calls[0][0] as Config;
    // enabled + platform_url persist; bearer_token stays null (keychain
    // is the source of truth, config never carries the secret).
    expect(saved.org_connector).toEqual({
      enabled: true,
      platform_url: 'https://orgs.example',
      bearer_token: null,
    });
    // The onSave path never touches the bearer keychain commands.
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      'set_org_bearer',
      expect.anything()
    );
  });

  it('shows "Not set", then "Configured" after Save calls set_org_bearer with { bearer_token }', async () => {
    const cfg = makeConfig({
      org_connector: {
        enabled: true,
        platform_url: 'https://orgs.example',
        bearer_token: null,
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);

    // On mount the keychain probe reports the token is absent.
    expect(await screen.findByText(/not set/i)).toBeInTheDocument();

    const tokenInput = screen.getByLabelText(/org connector bearer token/i);
    const card = tokenInput.closest('section') as HTMLElement;
    fireEvent.change(tokenInput, { target: { value: 'desk-token-123' } });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /save token/i }));
    });

    // Save routes through the dedicated command with the byte-exact
    // snake_case IPC arg — never camelCase, never the config save path.
    expect(mockedInvoke).toHaveBeenCalledWith('set_org_bearer', {
      bearer_token: 'desk-token-123',
    });

    // Status flips to Configured with the redacted preview from the
    // command response; the paste box is cleared. Scoped to the org card
    // since the RSI cookie card also carries a "…configured…" line.
    expect(await within(card).findByText(/configured/i)).toBeInTheDocument();
    expect(within(card).getByText('…3456')).toBeInTheDocument();
    expect((tokenInput as HTMLInputElement).value).toBe('');
  });

  it('Clear calls clear_org_bearer and returns the field to "Not set"', async () => {
    const confirmSpy = vi
      .spyOn(window, 'confirm')
      .mockReturnValue(true);
    stubInvoke({ bearer: { configured: true, preview: '…cret' } });
    const cfg = makeConfig({
      org_connector: {
        enabled: true,
        platform_url: 'https://orgs.example',
        bearer_token: null,
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);

    const tokenInput = await screen.findByLabelText(
      /org connector bearer token/i
    );
    // Scope to the org card — the RSI cookie card also renders a Clear
    // button + a "…configured…" status line.
    const card = tokenInput.closest('section') as HTMLElement;

    // Starts configured — await the async on-mount probe (findByText) so this
    // doesn't race it on slower CI.
    expect(await within(card).findByText(/configured/i)).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(within(card).getByRole('button', { name: /^clear$/i }));
    });

    expect(mockedInvoke).toHaveBeenCalledWith('clear_org_bearer');
    expect(await screen.findByText(/not set/i)).toBeInTheDocument();

    confirmSpy.mockRestore();
  });

  it('flags a non-localhost plaintext URL and clears once it is TLS', async () => {
    const cfg = makeConfig();
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);

    fireEvent.click(
      await screen.findByLabelText(/enable org platform connector/i),
    );

    const urlInput = screen.getByPlaceholderText(/orgs\.example/i);
    fireEvent.change(urlInput, { target: { value: 'http://orgs.example' } });
    expect(
      screen.getByText(/non-localhost urls must use https/i),
    ).toBeInTheDocument();
    expect(urlInput).toHaveAttribute('aria-invalid', 'true');

    // Plaintext localhost is allowed — no error.
    fireEvent.change(urlInput, { target: { value: 'http://localhost:8080' } });
    expect(
      screen.queryByText(/non-localhost urls must use https/i),
    ).not.toBeInTheDocument();

    // TLS clears any URL error too.
    fireEvent.change(urlInput, { target: { value: 'https://orgs.example' } });
    expect(urlInput).not.toHaveAttribute('aria-invalid');
  });

  it('requires a bearer token while the connector is enabled, and clears once one is saved', async () => {
    const cfg = makeConfig({
      org_connector: {
        enabled: true,
        platform_url: 'https://orgs.example',
        bearer_token: null,
      },
    });
    wrap(<SettingsPane config={cfg} onSave={vi.fn()} />);

    // Error is derived from the keychain status probe (not `draft`).
    expect(
      await screen.findByText(/a bearer token is required/i),
    ).toBeInTheDocument();

    // Saving a token flips the status to configured, which clears the
    // required-error — typing alone no longer clears it since the value
    // isn't bound to the draft.
    const tokenInput = screen.getByLabelText(/org connector bearer token/i);
    fireEvent.change(tokenInput, { target: { value: 'desk-token' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /save token/i }));
    });
    expect(
      screen.queryByText(/a bearer token is required/i),
    ).not.toBeInTheDocument();
  });
});
