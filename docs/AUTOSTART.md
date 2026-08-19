# Auto-start at system sign-in

StarStats can launch automatically when you sign in. **The default is
ON for fresh installs.** The toggle lives at Settings → Updates →
"Launch at sign-in".

## How it works

Implemented via [`tauri-plugin-autostart`]. On startup the tray reads
the user preference (`autostart_enabled` in `config.toml`) and
reconciles it against the per-OS autostart entry:

- First run (field is unset) → enable + persist `true`.
- Field is `true` and the OS entry is missing → enable.
- Field is `false` and the OS entry is present → disable.
- Already in the target state → no-op.

When the OS starts StarStats via the entry, the binary is invoked with
`--autostart`. The setup-closure suppresses the first-launch main
window in that case; the tray icon is the only visible affordance
until the user clicks it.

## Where the state lives

| OS      | Where the entry is written                                              |
|---------|-------------------------------------------------------------------------|
| Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (value `StarStats`) |
| Linux   | `$XDG_CONFIG_HOME/autostart/StarStats.desktop` (user-scope `.desktop`)   |
| macOS   | `~/Library/LaunchAgents/app.starstats.tray.plist` (LaunchAgent)         |

The user preference is mirrored in
`%APPDATA%\StarStats\config.toml` (or the XDG/macOS equivalent) as
`autostart_enabled = true | false`.

## Disabling manually

If the app is gone but the entry remains, delete the file/key listed
above. The toggle in Settings is the supported path for active
installs.

[`tauri-plugin-autostart`]: https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/autostart
