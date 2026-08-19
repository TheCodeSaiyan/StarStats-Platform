# Windows installer upgrade UX — design note

> **Status:** v0.2.x ships ARP metadata + an upgrade-detection
> property (see `crates/starstats-client/wix/upgrade_metadata.wxs`).
> What's still deferred is the *visible* "Upgrading existing install"
> dialog text — that needs a full WiX UI template override and a
> Windows VM iteration loop.

## Current behaviour (today)

Tauri 2 derives the MSI `UpgradeCode` from `bundle.identifier`
(`app.starstats.tray`). On install, the Windows Installer engine:

1. Reads the candidate's UpgradeCode.
2. Searches for products with the same UpgradeCode.
3. If found and the candidate version > existing, it triggers a
   **major upgrade** — the existing install is removed and the new one
   takes its place, in one transaction.

This works **silently** with Tauri's default WiX UI. There is no UI
text telling the user "we're upgrading your existing install".

## What we want

When a user double-clicks the v0.2.0 MSI on a machine that already has
v0.1.0 installed:

- Welcome dialog header reads **"Upgrading StarStats"** instead of
  "Welcome to the StarStats Setup Wizard".
- A short body line: **"This installer will replace your existing
  StarStats v0.1.0 with v0.2.0. Your settings and event database
  will be preserved."**
- The progress dialog title shows **"Upgrading StarStats..."**
  instead of "Installing StarStats...".

## What's already in place

`crates/starstats-client/wix/upgrade_metadata.wxs` (registered via
`bundle.windows.wix.fragmentPaths`) ships:

- ARP entries (`ARPURLINFOABOUT`, `ARPHELPLINK`, `ARPNOMODIFY`,
  `ARPNOREPAIR`) so Settings → Apps & features shows project links.
- A `STARSTATS_UPGRADE_DETECTED` Property mirroring the built-in
  `WIX_UPGRADE_DETECTED` signal under our namespace. The visible-UI
  template work below keys conditional dialog text off this Property.

## Implementation path (visible UI text — not yet shipped)

Three pieces:

1. **WiX template override** — `tauri.conf.json` →
   `bundle.windows.wix.template` pointing at
   `crates/starstats-client/wix/main.wxs`. Copy Tauri 2's default
   template as the starting point (its `wix-default.wxs`), then
   customise.
2. **Custom WiX fragment** — `crates/starstats-client/wix/upgrade.wxs`
   with a `<UI>` element that conditions text on the
   `WIX_UPGRADE_DETECTED` property:

   ```xml
   <Property Id="WIXUI_INSTALLDIR" Value="INSTALLDIR" />
   <UI>
     <UIRef Id="WixUI_InstallDir" />
     <Publish Dialog="WelcomeDlg" Control="Next" Event="EndDialog"
              Value="Return" Order="2">WIX_UPGRADE_DETECTED</Publish>
   </UI>
   <CustomAction Id="SetUpgradeText" Property="WelcomeDlgTitle"
                 Value="Upgrading [ProductName]"
                 Execute="immediate" />
   <InstallUISequence>
     <Custom Action="SetUpgradeText" Before="LaunchConditions">
       WIX_UPGRADE_DETECTED
     </Custom>
   </InstallUISequence>
   ```

3. **`bundle.windows.wix.fragmentPaths`** — list the new fragment.

## Testing

This MUST be tested by:

1. Building the v0.x installer.
2. Running it on a clean VM — verify normal install UI text.
3. Building v0.x+1 with the same UpgradeCode.
4. Running it on the same VM — verify "Upgrading..." text appears.

Cannot be tested without a Windows VM and at least two consecutive
release builds. That's why this is deferred — landing a half-tested
WiX template that breaks the installer is far worse than waiting for
a focused pass with a proper test loop.

## Install-time auto-start checkbox

> **Status:** Deferred. Same blocker as the upgrade-text work above —
> needs a full WiX UI template override.

The tray now ships a runtime auto-start toggle (Settings → Updates →
"Launch at sign-in", default ON) via `tauri-plugin-autostart`; see
`docs/AUTOSTART.md`. The matching install-time affordance — a visible
checkbox in the WiX welcome dialog letting the user opt out *before*
the first launch — is not yet shipped.

Implementation depends on the WiX UI template override described above:

1. The custom `main.wxs` adds a checkbox bound to a Property (e.g.
   `STARSTATS_AUTOSTART`) on the WelcomeDlg / CustomizeDlg.
2. A `<CustomAction>` writes a marker file (or a registry value under
   `HKCU\Software\StarStats`) when the user unticks the box, before
   the first run of the binary.
3. The tray's setup-closure honours the marker: if present, the
   first-run default is OFF instead of ON, and the marker is consumed.

Cannot be tested without the same Windows VM + two-release loop as
the upgrade-text work, so it lands together with that pass.

## Settings / DB preservation note

The upgrade preserves user data because StarStats stores its DB +
config under `%APPDATA%\StarStats\`, not under `%PROGRAMFILES%`. The
MSI only manages files under `%PROGRAMFILES%\StarStats\` (the binary
+ resources), so the user's local store is untouched by uninstall +
reinstall.

The "Your settings and event database will be preserved." copy in
the upgrade dialog is only true while this layout holds. If we ever
move part of the DB into the install dir, the wording must change.
