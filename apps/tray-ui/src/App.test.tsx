import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, waitFor, act } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Config } from './api';
import App from './App';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);

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
    ...overrides,
  } as Config;
}

// App mounts on the 'status' tab which polls `get_status`, fetches
// `get_app_version`, and polls `count_unknown_lines`. SettingsPane
// (with its own RSI cookie / autostart fetches) is NOT mounted on the
// default tab — only `get_config` + the always-on side-effects need
// stubbing.
function stubInvoke(initialConfig: Config) {
  mockedInvoke.mockImplementation((cmd: string) => {
    if (cmd === 'get_config') return Promise.resolve(initialConfig);
    if (cmd === 'get_app_version') return Promise.resolve('0.0.0-test');
    if (cmd === 'count_unknown_lines') return Promise.resolve(0);
    if (cmd === 'get_status') return Promise.reject(new Error('status not relevant'));
    return Promise.reject(new Error(`unexpected invoke: ${cmd}`));
  });
}

describe('App-level config-changed listener', () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    (listen as ReturnType<typeof vi.fn>).mockReset();
    // Reset document.theme attribute between tests so assertions are
    // independent of the starting state set by `index.html`.
    document.documentElement.dataset.theme = 'stanton';
  });

  it('updates document theme when a remote config-changed arrives', async () => {
    let configHandler: ((e: { payload: Config }) => void) | undefined;
    (listen as ReturnType<typeof vi.fn>).mockImplementation((event, handler) => {
      if (event === 'config-changed') configHandler = handler;
      return Promise.resolve(() => undefined);
    });
    stubInvoke(makeConfig({ theme: 'stanton' }));

    render(<App />);

    // Wait for initial getConfig() to resolve and the theme effect to
    // apply 'stanton' to the document root.
    await waitFor(() => {
      expect(document.documentElement.dataset.theme).toBe('stanton');
    });

    // Remote sync downloads a new theme from another device.
    const incoming = makeConfig({ theme: 'nyx' });
    act(() => {
      configHandler?.({ payload: incoming });
    });

    // The document theme attribute must reflect the remote value
    // immediately — without the user having to open the Settings tab
    // or refresh the app.
    await waitFor(() => {
      expect(document.documentElement.dataset.theme).toBe('nyx');
    });
  });
});
