# EAC-Safety

This document explains, in detail, why running StarStats does not
violate Easy Anti-Cheat's tampering rules. If you've landed here from
the README and you'd rather not run code on faith, this is the file
that should answer your questions. If anything below is wrong,
unclear, or missing, raise it via the channels in
[`SECURITY.md`](SECURITY.md) and we'll correct it.

> **TL;DR.** StarStats opens text files the game already wrote to
> disk, and issues HTTPS requests to a website you're already logged
> into in your browser. It does not attach to the game process, read
> game memory, hook the game's APIs, modify game files, or drive
> in-game input. Easy Anti-Cheat's published threat model targets
> exactly the things StarStats refuses to do.

## What Easy Anti-Cheat actually watches for

EAC is a kernel-level anti-cheat that ships with Star Citizen. Per
its public documentation, it inspects the running game process and
its environment for signs of tampering. The classes of behaviour it
flags include:

1. **Process attachment.** Anything calling `OpenProcess` against
   `StarCitizen.exe`, `ReadProcessMemory`, `WriteProcessMemory`,
   `VirtualAllocEx` into the game's address space, or
   `CreateRemoteThread`.
2. **Module injection.** DLLs loaded into the game by
   `SetWindowsHookEx`, `LoadLibrary`-via-remote-thread, AppInit_DLLs,
   manual mapping, etc.
3. **API hooking.** Detours, IAT/EAT patches, vtable hooks, or
   trampolines placed inside game code or libraries it depends on.
4. **Driver-level tampering.** Unsigned drivers, vulnerable-driver
   loads, syscall hooks, hypervisor-style stealth.
5. **Game file modification.** Patching shipped binaries, shaders,
   or pak files; redirecting them via shims; replacing engine DLLs.
6. **Network manipulation against the game's traffic.** Intercepting,
   modifying, or injecting packets into the game's own connection.
7. **Input automation against the game window.** Synthetic input
   directed at the game (auto-fire, auto-aim, multibox replication).
8. **Overlay / frame interception.** D3D/Vulkan/DXGI hooks that
   render into the game window, capture the framebuffer, or read
   GPU resources from the game.

Cheats and bans are produced by combinations of those signatures.
**StarStats does none of them.**

## What StarStats actually does

StarStats has exactly two input planes. Both are explicitly outside
EAC's protected boundary.

### Plane 1 — the local `Game.log` file

The Star Citizen client writes a text log of its session to
`Game.log` inside its install directory (typical paths:
`%LOCALAPPDATA%\..\StarCitizen\LIVE\Game.log`,
`...\PTU\Game.log`, `...\TECH-PREVIEW\Game.log`). The file is:

- Written **by the game itself** during gameplay.
- A plain UTF-8 text file with normal NTFS ACLs that allow read
  access to the same OS user that runs the game.
- **Outside the EAC-protected boundary.** EAC's integrity checks
  cover the running process and the binaries/paks it loads, not
  side-channel files the engine flushes to disk for diagnostics.
- Already routinely opened by the player themselves (Notepad,
  Notepad++, VS Code, third-party log viewers).

StarStats opens this file the way every text-tailing tool does:

- Cross-platform filesystem watcher via the `notify` Rust crate
  (which sits on `ReadDirectoryChangesW` on Windows and `inotify` on
  Linux — the same APIs that File Explorer's auto-refresh, OneDrive
  sync, and every IDE you've ever used rely on).
- A `std::fs::File` opened in read-only mode with shared-read
  sharing flags so we never block the game from writing to it.
- Incremental reads tracked by byte offset; we re-read from offset 0
  if the file shrinks (rotation/restart) and resume from where we
  left off otherwise.

The file is parsed line-by-line into structured events
(`crates/starstats-core/src/events.rs`). The parser is regex-based
and runs entirely inside the StarStats process. The game has
already finished with each line by the time we see it.

### Plane 2 — the RSI website, with your own session

If — and only if — you opt into Hangar Sync and paste your own RSI
session cookie into the tray's settings, StarStats also issues
authenticated HTTPS requests to `robertsspaceindustries.com`:

- Requests are made via `reqwest` over `rustls-tls`. They look
  identical on the wire to any other HTTPS client (browser, `curl`,
  `Invoke-WebRequest`).
- The cookie is the same `RSI-` session cookie your browser already
  holds when you're logged in. StarStats does not log you in, does
  not know your password, does not read your browser's cookie store,
  and does not bypass any auth check — it presents a cookie *you*
  pasted in.
- Targets are limited to your own profile and hangar pages — the
  same pages your browser fetches when you click around the RSI
  site while logged in.
- The cookie is stored in your OS keychain (Windows Credential
  Manager / macOS Keychain / Linux Secret Service via the `keyring`
  crate, default features off, with the appropriate native backend
  enabled per platform). Only the same OS user that pasted the
  cookie can read it back.
- The hangar fetcher pauses while `StarCitizen.exe` is running — not
  for EAC reasons (the request never touches the game process) but
  to avoid overlapping authenticated HTTP from the same machine
  while the game is also reaching out to RSI for its own purposes.

This is the same mechanism a Greasemonkey script, an RSS reader, or
a homemade scraper would use against any other site you have a
session for. RSI's own terms allow accessing your own account data
via standard HTTP requests; there is nothing privileged or
undocumented being called.

## What StarStats explicitly will not do

The following are out of scope and will be rejected on review:

- `OpenProcess` against `StarCitizen.exe` or any related process.
- `ReadProcessMemory` / `WriteProcessMemory` against the game.
- `CreateRemoteThread`, manual DLL mapping, `SetWindowsHookEx`, or
  `AppInit_DLLs`.
- Drivers of any kind. StarStats is a userland Rust binary plus a
  webview; nothing it ships needs ring 0.
- D3D/DXGI/Vulkan/OpenGL hooks. There is no overlay, no FPS counter,
  no in-game UI. The tray UI lives in its own Tauri window.
- Frame capture, window capture, or OCR against the running game.
- Synthetic keyboard / mouse / gamepad input directed at the game
  window. No macros, no auto-clickers, no aim assistance, no
  multiboxing.
- Modifying anything inside the Star Citizen install directory.
  StarStats opens `Game.log` read-only and never writes to that
  tree.
- Sniffing or modifying the game's network traffic. We don't bind to
  raw sockets, we don't run a local proxy in front of the game, and
  we don't install network-filter drivers.
- Reading other players' RSI accounts, hangar contents, or profiles.
  Only the cookie-bearer's own account is fetched.

If a future feature would require any of the above, the answer is
that feature doesn't belong in StarStats. The whole point of the
project is that the answer is "no."

## How this compares to tools that *do* get bans

| Tool class | What it does | EAC-visible? | StarStats? |
|---|---|---|---|
| Memory readers (e.g. radar/ESP overlays) | `OpenProcess` + `ReadProcessMemory` against the game; reads UEC, position, vehicle state from process memory | **Yes** — exact pattern EAC was built to detect | **No** — never touches the game process |
| In-game overlays (FPS, kill feed, server-FPS HUDs that draw inside the game window) | D3D/DXGI hooks loaded into the game process | **Yes** — module injection + API hooking | **No** — Tauri tray UI is a separate window |
| Macro / aim-assist tools | Synthesise input or hook input APIs to alter mouse/keyboard behaviour while the game is in focus | **Yes** in the case of injection; even pure SendInput tools fall foul of CIG's EULA | **No** — StarStats sends no input anywhere |
| Game-file mods (texture packs, gameplay tweaks, EXE patches) | Replace files in the install directory or load alongside the game | **Yes** — integrity check failure | **No** — install directory is read-only to us, and only `Game.log` is opened |
| Packet inspectors (modified clients, MITM, network-filter drivers) | Hook into the game's TCP/UDP traffic | **Yes** | **No** — we don't see any of the game's network traffic |
| **`Game.log` tailers (StarStats and similar)** | **Read a text file the game flushed to disk** | **No** — same posture as Notepad reading the same file | **This is StarStats** |
| **Authenticated browsers / scrapers of RSI (StarStats Hangar Sync, your own browser, fan apps you log into)** | **Make HTTPS requests with your own session cookie to a public website** | **No** — never touches the game | **This is StarStats** |

The bright line is: did the tool *touch the game*? StarStats doesn't.
Tools that do get banned. Tools that work entirely off side-effects
the game wrote to disk, plus a website you're a logged-in user of,
have always been fine.

## The Windows APIs StarStats uses, in plain terms

So you can verify the surface area yourself:

| Capability | Crate | Underlying Windows API | Used elsewhere by |
|---|---|---|---|
| Watch a directory for file changes | `notify` | `ReadDirectoryChangesW` | OneDrive, Dropbox, every IDE, File Explorer |
| Read a text file | `std::fs::File` | `CreateFileW` (read-only, `FILE_SHARE_READ \| FILE_SHARE_WRITE`) | Notepad, every text editor |
| HTTPS requests | `reqwest` over `rustls-tls` + `tokio` | `WSAStartup`, `socket`, `connect`, `send`, `recv` | curl, Discord, Slack, every browser |
| OS keychain | `keyring` (windows-native backend) | Windows Credential Manager (`CredWrite`/`CredRead`) | Git Credential Manager, the Edge browser |
| Process enumeration (only to check whether the game is running) | `sysinfo` | `CreateToolhelp32Snapshot`, `Process32First/Next` | Task Manager, Process Explorer |
| System tray UI | `tauri` + `tray-icon` | `Shell_NotifyIconW` | Discord, Slack, OBS, Steam |

None of these APIs are restricted, none of them require elevation,
and none of them target the game process. They are the same APIs
your other desktop apps are using right now.

## A note on RSI

RSI's terms permit you to access your own account data via standard
HTTP requests. The Hangar Sync feature uses your existing session as
a normal authenticated client. We do not bypass authentication, we
do not attempt to access other accounts, and we rate-limit our own
requests so we don't behave like an abusive scraper. If RSI ever
asks us to change how the integration works, we will. (Get in touch
via [`SECURITY.md`](SECURITY.md) or the issue tracker.)

## Independent review

If you'd like to perform or coordinate a review of StarStats'
EAC-safety posture, please reach out at **<security@starstats.app>**.
We'll happily walk through the code paths above, share build
artifacts, and update this document with any findings. The whole
project is open source under MPL-2.0 — every claim above can be
verified by reading the source.

The summary, again: StarStats is built around the rule that EAC must
never see us touching the game. If anything in this repository ever
appears to violate that rule, treat it as a bug, file it via the
security channel, and we'll fix it.
