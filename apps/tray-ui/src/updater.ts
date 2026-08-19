import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { relaunch } from '@tauri-apps/plugin-process';
import type { ReleaseChannel } from './api';

/**
 * Updater check result. Unlike the plugin-updater 2.10.x JS API,
 * this carries no opaque handle — the install step re-runs `check`
 * server-side so we don't have to plumb a non-Serializable `Update`
 * across the IPC boundary. The race window between check and
 * install is acceptable: a fresher release between the two would
 * simply install the newer one.
 */
export interface UpdateInfo {
  available: true;
  version: string;
  notes: string | null;
  date: string | null;
}

export interface NoUpdate {
  available: false;
}

export type UpdateCheckResult = UpdateInfo | NoUpdate;

interface RustCheckOutcome {
  available: boolean;
  version: string | null;
  notes: string | null;
  date: string | null;
}

/**
 * Asks the Rust side to check the given channel's manifest. The
 * Rust command builds a fresh `Updater` with `endpoints(...)`
 * overridden to the channel URL, so flipping the channel in
 * Settings takes effect on the next check without an app restart.
 *
 * The JS plugin-updater's `check()` is bypassed because its
 * `CheckOptions` doesn't expose the `endpoints` field in
 * 2.10.1 — only the Rust `UpdaterBuilder` supports
 * per-call endpoint override.
 */
export async function checkForUpdate(
  channel: ReleaseChannel,
): Promise<UpdateCheckResult> {
  const result = await invoke<RustCheckOutcome>(
    'check_for_update_for_channel',
    { channel },
  );
  if (!result.available || !result.version) {
    return { available: false };
  }
  return {
    available: true,
    version: result.version,
    notes: result.notes,
    date: result.date,
  };
}

/**
 * Downloads + installs the latest release on the given channel,
 * then relaunches. Progress events are emitted from Rust on the
 * `update-progress` Tauri event channel; we subscribe before
 * invoking the install command and unsubscribe in `finally`.
 *
 * Returns when install completes — the relaunch typically kills
 * this process, so callers shouldn't expect to run code after.
 */
export async function applyUpdate(
  channel: ReleaseChannel,
  onProgress?: (downloaded: number, total: number | null) => void,
): Promise<void> {
  let unlisten: UnlistenFn | null = null;
  if (onProgress) {
    unlisten = await listen<{ downloaded: number; total: number | null }>(
      'update-progress',
      (event) => {
        onProgress(event.payload.downloaded, event.payload.total);
      },
    );
  }
  try {
    const installed = await invoke<boolean>('install_update_for_channel', {
      channel,
    });
    if (installed) {
      await relaunch();
    }
  } finally {
    unlisten?.();
  }
}
