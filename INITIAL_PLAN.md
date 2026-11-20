I want to build "GloriaSetup", one custom Windows installer used by every app in my projectGloria suite. This repo is a new, standalone project. My other apps live next to it in `C:\Users\Nihil\Desktop\projectGloria\` (about 60 repos). 12 are Tauri v2 apps (they ship Tauri's default NSIS/MSI installers today), 8 use separate Inno Setup scripts (Ada, DefaultFast, Inscriptio, Mutatio, Salvus, WindowsTweaks, WingetUI, FakeGameLauncher), and the rest have no installer. I want one installer with one look for all of them, styled like my launcher, GloriaLauncher (`C:\Users\Nihil\Desktop\projectGloria\GloriaLauncher`).

## Goal
A Rust project that produces a branded, single-file setup `.exe` for any app from:
- the app's built files (a folder),
- a small manifest (id, display name, version, publisher, main exe, icon, accent color, data folders, flags such as "needs WebView2").

## Suggested architecture (challenge it if you have a better idea)
- `stub`: the installer and uninstaller program, a single self-contained `.exe`. At run time it locates its appended payload (zstd/zip archive + manifest + icon) by parsing its own PE headers (starting after the last PE section and stopping before any PE Certificate Table / Authenticode signature) rather than reading blindly from EOF, so Authenticode signing never breaks payload extraction.
- `gloria-pack`: a CLI that takes the stub, the app folder, and the manifest, and writes `<AppName>_<version>_x64-setup.exe`. Packing must strictly follow this order: patch PE resources (icon + version info) → append payload → optional code signing (`signtool`) last.
- A reusable GitHub Actions workflow or composite action (running on `windows-latest` if Win32 `UpdateResource` is used) that any app repo can call in its release job to build the setup and attach it to the GitHub release.
- UI: prefer a native Rust UI (`egui`, `Slint`, or direct Direct2D/DirectWrite via `windows-rs`) over a WebView so the installer itself has no WebView2 dependency and stays small. Pick the one that best reproduces the approved visual design (see below) and justify it in `docs/design.md`, explicitly addressing:
  - **Windows Sandbox / software GPU compatibility** (must render cleanly under WARP / software rendering — verify inside Windows Sandbox before finishing Milestone 1),
  - **Licensing** (e.g., Slint's GPLv3 / Royalty-Free attribution badge requirements vs. closed-source apps in the suite and our optional credit),
  - **Headless / deterministic screenshot capture** for `--screenshot <dir>`,
  - **Zero startup flicker** (create the window hidden and only show it after the first frame is rendered).

## Visual design comes first (high priority)
The visual quality of the installer matters most to me. Work in this order:
1. Before writing the stub UI, make clickable HTML mockups of every screen in `design/mockups/`: welcome, options, progress, done, update, reinstall, downgrade warning, upgrade from previous installer, uninstall, and error — each with its user-data option where it applies. Use two real apps' icons and accent colors (e.g. OmniConvert amber `#f59e0b`, GloriaTheme crimson `#dc143c`) so I can see how per-app theming looks. Show them to me and wait for my approval or changes.
2. Match GloriaLauncher's style: dark obsidian background, neon accent glow, clean typography, smooth transitions. Read GloriaLauncher (styles, components, and registry/launch logic) for the colors, fonts, feel, and exact `DisplayIcon` expectations — and only read it; don't change anything there.
3. Then build the real UI to match the approved mockups as closely as the UI library allows. If the library can't do something from the mockup (blur, glow, animation), tell me and propose the closest alternative. Don't quietly simplify it.
4. Details count: crisp icons at every DPI (100–200% scaling), no flicker on start, a custom title bar, the window centered at a fixed size, and a polished done screen with a "Launch `<App>`" button.

## Installer behavior
- **Strictly per-user, no admin**: install to `%LOCALAPPDATA%\Programs\<AppName>` by default (`HKCU`). Allow changing the folder, but reject non-writable/system paths (such as `C:\Program Files\...`) with a clear error message rather than introducing a dual HKCU/HKLM install mode. UAC elevation is only ever requested when removing an old HKLM installer during migration.
- **Safe custom folders**: if the user selects an existing non-empty folder that does not already belong to this app, automatically append `\<AppName>` (or require an empty subfolder) so installing and later uninstalling can never touch unrelated user files.
- **Tracked file manifest**: write a small manifest (e.g. `install-manifest.json`) in the install folder recording every installed file and created directory. Updates and uninstalls must only remove tracked files (and empty directories) rather than blindly deleting `InstallLocation`. This prevents accidental data loss and ensures files removed in newer versions don't stay orphaned on disk.
- **Atomic / safe file replacement**: stage extraction or move old files aside before committing new ones so a mid-install or mid-update failure (locked file, antivirus scan lock, full disk) rolls back cleanly instead of leaving a broken half-installed app.
- Flow for a fresh install: welcome (app icon, name, version, accent color) → folder and options (desktop shortcut, launch after install) → progress → done. Keep it to 2–3 screens; it should feel modern, not like a wizard.
- Start Menu shortcut always, desktop shortcut optional.
- **WebView2 handling**: if the manifest says the app needs WebView2 (Tauri apps), check both `HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` and `HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` for a `pv` value that exists and is not `"0.0.0.0"`. If missing, run Microsoft's Evergreen WebView2 bootstrapper bundled conditionally inside the app's payload (only when `needs_webview2 = true`, keeping the stub offline-capable and free of an HTTP/TLS stack).
- UI languages: `en`, `fr`, `es`, `tr` (my suite's languages), following the Windows language with an English fallback.

## User data
- The manifest lists the app's data folders (e.g. `%APPDATA%\<App>`, `%LOCALAPPDATA%\<App>`).
- **Path validation guardrails**: validate every `data_folders` entry at both pack time (`gloria-pack`) and runtime (`stub`). Only allow known user-scoped roots (`%APPDATA%\<subdir>`, `%LOCALAPPDATA%\<subdir>`, `%USERPROFILE%\<subdir>`) with at least one concrete subfolder; reject `..` traversal, drive roots, or bare `%USERPROFILE%` / `%APPDATA%` / `%LOCALAPPDATA%`.
- Whenever removing data is a real choice, ask with a clear option such as "Also remove my settings and data" that lists those folders and their total size, unchecked by default (keep is the default).
- Never delete user data without the user choosing it. Never ask during a normal update.

| Situation | Asks about user data? | Default |
|---|---|---|
| Uninstall | Yes | Keep |
| Reinstall (same version) | Yes, as a "clean reinstall" option | Keep |
| Upgrade from old installer | Yes | Keep |
| Downgrade | Yes (older versions may not read newer data) | Keep |
| Normal update | No, data is always kept | Keep |
| Silent mode (`/S`) | Never asks, never deletes unless `/PURGE` is passed | Keep |

## Uninstaller
- Write a **stripped** copy of the stub into the install folder as `uninstall.exe` — containing only the PE stub plus manifest/icon metadata, **without** the compressed app archive (never literally copy the full setup `.exe` into the install folder). Launching `uninstall.exe` starts directly in uninstall mode.
- **Self-deletion trampoline**: because a running Windows `.exe` cannot delete itself or its parent folder, `uninstall.exe` must copy itself to `%TEMP%`, spawn the temp copy with `--cleanup <install_dir> --wait-pid <pid>`, and exit. The temp copy waits for the parent PID to exit, removes tracked files, shortcuts, registry entries, optional user data, and the empty install directory, and schedules its own deletion via `MoveFileExW(..., MOVEFILE_DELAY_UNTIL_REBOOT)`.

## Existing installs
On start, check the app's registry key and compare `DisplayVersion` using **numeric version / SemVer comparison** (e.g. `0.10.0 > 0.2.0`, never lexicographical string comparison; define pre-release and 4-part version handling in `docs/design.md`), then pick a mode:
- **Older version installed** → "Update" screen showing installed → new version. Reuse the existing install folder and the user's earlier choices (desktop shortcut etc.) without asking again, close running app processes (see below), replace files via the tracked manifest (removing obsolete files), always keep user data (no data question), and update `DisplayVersion` and `EstimatedSize`.
- **Same version installed** → offer "Reinstall" or "Uninstall". Reinstall asks about user data ("clean reinstall").
- **Newer version installed** → warn that this is a downgrade and that the older version may not read newer data; only continue if the user confirms, and ask about user data. Silent mode refuses unless `/ALLOWDOWNGRADE` is passed.
- **Not installed by GloriaSetup, but installed by an old installer** (Tauri NSIS/MSI or my Inno Setup scripts) → "Upgrade from previous installer" screen:
  - Scan `HKCU` and `HKLM` Uninstall keys across **both 64-bit and 32-bit registry views** (`KEY_WOW64_64KEY` and `KEY_WOW64_32KEY`) to match `DisplayName`.
  - Ask about user data. If the user chooses **Keep**, ensure the old uninstaller cannot silently wipe declared data folders (verify old installer behavior or back up and restore declared data folders around the old uninstaller run if needed).
  - Run the old uninstaller silently using type-appropriate flags (prefer `QuietUninstallString` where valid, otherwise handle Inno `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`, WiX/MSI rewriting `MsiExec /I{GUID}` to `/X{GUID} /qn /norestart`, and NSIS `/S _?=<old_install_dir>` so NSIS runs synchronously without spawning an async `%TEMP%` copy that races with the new install — then delete the leftover NSIS uninstaller `.exe`).
  - If the old install is in `HKLM` and needs admin rights to remove, explain that clearly and ask before elevating (only the old uninstaller runs elevated; the new GloriaSetup install stays per-user).
  - Then install normally so only one entry remains in Settings → Apps.

### Closing running processes (update / reinstall / downgrade / uninstall)
- Identify running processes by **full executable path inside `InstallLocation`** (`QueryFullProcessImageNameW`), never by `.exe` name alone.
- In interactive mode, ask before closing. Send `WM_CLOSE` first and wait with a timeout; account for lingering child processes (such as `msedgewebview2.exe` in Tauri apps) with brief retries before offering force-close or failing gracefully.

## Silent mode & CLI parsing
For automation and GloriaLauncher updates:
- Supported flags: `/S` (silent install/uninstall), `/D=<path>` (install folder), `/PURGE` (remove user data), `/ALLOWDOWNGRADE` (permit downgrade), `/RELAUNCH` (launch the app after a silent install/update — needed when an app like GloriaLauncher self-updates and exits).
- **NSIS-compatible command-line parsing**: parse raw `GetCommandLineW()` directly (not a standard POSIX-style argument parser alone) so NSIS-style trailing unquoted `/D=C:\My Apps\Foo` (as well as quoted `/D="..."`) and `/S` flags parse reliably.
- Silent mode follows all the rules above without showing UI screens, never removes user data unless `/PURGE` is passed, and refuses downgrades unless `/ALLOWDOWNGRADE` is passed.
- **Defined exit codes** (so GloriaLauncher can report failures accurately):
  - `0` — Success
  - `1` — Generic / unexpected error
  - `2` — Downgrade refused (missing `/ALLOWDOWNGRADE`)
  - `3` — Running app could not be closed / files locked
  - `4` — UAC elevation cancelled or failed (old HKLM uninstaller migration)
  - `5` — WebView2 runtime installation failed
  - `6` — Invalid arguments or non-writable install path

## GloriaSetup's own icon
I already added GloriaSetup's own brand files at the repo root:
- `GloriaSetup.ico`: multi-size (16, 20, 24, 32, 40, 48, 60, 64, 96, 128, 256). Use this for anything Windows displays as an icon.
- `GloriaSetup.png`: 1254×1254, no transparency (no alpha channel). Use it only for docs and images, never to generate icons.

Move them into `assets/brand/` (use `git mv`; keep the files unchanged — don't resize, recolor or regenerate them) and use them wherever GloriaSetup represents itself:
- The icon embedded in the stub. It is the placeholder that `gloria-pack` replaces with the app's icon, so the icon resource already exists to replace.
- Bare preview runs without a manifest, and test builds made with `--allow-default-icon`.
- The `gloria-pack.exe` CLI's own icon.
- The README header/logo and a GitHub social preview image (built from the PNG).
- A small, subtle "GloriaSetup" credit on the done screen if it fits the approved design (optional, and I decide in the mockups).
GloriaSetup's icon must never appear as the icon of a real packed installer, its window, or its `uninstall.exe`. Those always use the app's own icon (next section).

## Per-app icons (required)
Every app's installer must carry that app's own icon. That applies to:
- The setup `.exe` file itself (what you see in Explorer, Downloads, and the taskbar): `gloria-pack` must replace the stub's embedded GloriaSetup icon resource with the app's icon at pack time. Editing the resources of the prebuilt stub (Win32 `UpdateResource` or a PE resource crate) is preferred over recompiling the stub for each app, so packing stays fast and works on CI. If you choose recompiling, explain why in the design doc.
  - **Full `RT_ICON` purge**: because `GloriaSetup.ico` contains 11 sizes while a generated app `.ico` may have fewer, `gloria-pack` must enumerate and **delete all existing `RT_GROUP_ICON` and `RT_ICON` entries** in the stub before writing the app's icon entries, guaranteeing no leftover GloriaSetup icon sizes remain in the PE.
- The installer window: the title bar/taskbar icon and the large icon on the welcome screen.
- `uninstall.exe` in the install folder: it uses the same app icon.
- Shortcuts and the Apps & Features entry point to the app's main `.exe`, so they show the app's own icon.

Icon input:
- The manifest gives the icon path. Accept a `.ico`, or a PNG (at least 256×256) that `gloria-pack` converts into a multi-size `.ico` (16, 24, 32, 48, 64, 256).
- If a `.ico` is missing standard sizes (16, 24, 32, 48, 64, 256), generate them from the largest available image so it stays sharp at every Explorer view size.
- Conventions for my repos: Tauri apps already have `src-tauri/icons/icon.ico`. Other apps declare their icon path in the manifest.
- With no icon, packing fails with a clear error. `--allow-default-icon` (local testing only) packs with the GloriaSetup icon instead.
- Also set the setup `.exe`'s PE version info (`ProductName`, `FileDescription` `"<AppName> Setup"`, `ProductVersion`, `FileVersion`, `CompanyName` `"projectGloria"`).

## Hard contract with GloriaLauncher (do not break)
GloriaLauncher detects installed apps by reading `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\<subkey>` (and the HKLM equivalents), and offers updates by comparing `DisplayVersion` with my latest GitHub release. For each app write:
- `DisplayName` = the app name exactly as on GitHub (e.g. `"OmniConvert"`). The launcher strips a trailing version and compares only lowercase letters and digits, so `"OmniConvert"` or `"OmniConvert 1.2.0"` both work, but nothing else may be added.
- `DisplayVersion` = the version without a `"v"`.
- `InstallLocation` = the install folder.
- `DisplayIcon` = the full path to the main app `.exe` (verify in GloriaLauncher's source whether it expects a bare path or strips quotes / `,0`, and match its exact expectation).
- `UninstallString` = `"<install>\uninstall.exe"` (quoted so spaces in custom paths work in Windows Settings → Apps).
- `QuietUninstallString` = `"<install>\uninstall.exe" /S`.
- `Publisher` = `"projectGloria"`.
- `EstimatedSize` = `REG_DWORD` in **kilobytes (KB)** (not bytes).
- `NoModify` = `REG_DWORD(1)`, `NoRepair` = `REG_DWORD(1)`.
- The uninstaller's file name must start with `"unins"` (the launcher skips those when looking for the main `.exe`).
- The release asset must end in `-setup.exe` (the launcher's self-update looks for that).

## Testing and preview (required)
- `--preview` mode on the stub: shows the full installer UI with a given manifest and icon, simulates progress, and writes nothing to disk or the registry. Also allow `--lang <code>` to view each language and `--page <name>` to jump straight to any screen (including update, reinstall, downgrade, upgrade-from-previous-installer, uninstall, and error). Without a manifest, it uses the GloriaSetup icon and a sample app name.
- Add a `--screenshot <dir>` option to preview mode that saves a PNG of every screen in every language, so I can review all of it at once and compare it against the mockups after each change.
- A tiny dummy app in `examples/dummy` (a small `.exe` that opens a window showing its version and writes a small settings file into its data folder) plus two dummy icons, and a script that packs it into real setup exes for versions `0.1.0` and `0.2.0` in one command.
- Add `--dry-run` to the real installer: it logs every file, shortcut, registry write, and deletion it would make, without making them.
- Add a `sandbox/GloriaSetup.wsb` Windows Sandbox config that maps the build output folder read-only, so I can test a clean-machine install (and verify UI rendering under software GPU). Document how to use it in the README.
- README section "How to test": preview → dummy install/update/uninstall on my PC → Sandbox → a real app, with the exact commands and what to check at each step (Explorer icon, Start Menu, Settings → Apps, regedit fields, the data folder, GloriaLauncher detection).
- Automated tests:
  - registry fields against the GloriaLauncher contract (including quoting and `EstimatedSize` in KB),
  - PE payload round-trip, including with simulated trailing Authenticode signature bytes after the payload,
  - icons: the bare stub carries the GloriaSetup icon; packing the dummy app with two different icons gives each output `.exe` its own icon resource and version info, and **zero** leftover GloriaSetup `RT_ICON` entries remain,
  - uninstaller stripping: `uninstall.exe` does not contain the app archive payload and only deletes manifest-tracked files (leaving unrelated sibling files untouched),
  - version comparison (`0.10.0 > 0.2.0`, equal, downgrade),
  - data-folder path validation (rejects `..`, drive roots, bare `%APPDATA%`/`%LOCALAPPDATA%`/`%USERPROFILE%`),
  - NSIS-style CLI parsing (`/S`, `/PURGE`, `/ALLOWDOWNGRADE`, `/RELAUNCH`, and unquoted `/D=C:\Path With Spaces`),
  - existing-install modes: `0.1.0` → `0.2.0` update (including removal of an obsolete file from `0.1.0`), same-version reinstall, `0.2.0` → `0.1.0` downgrade, and upgrade from a fake old-installer registry entry (both 64-bit and 32-bit views),
  - user data: kept by default in every mode, removed only when chosen, never removed by a normal update, and removed in silent mode only with `/PURGE`,
  - silent mode exit codes (including downgrade refusal returning code `2`).

## Constraints
- Rust 1.80+, Windows-first, x64. Keep the stub small: aim for under 5 MB before the payload.
- Code signing: add an optional signing step (`signtool`, certificate from secrets) that is skipped when no certificate is configured. I don't have a certificate yet (note: unsigned self-extracting Rust installers may trigger Windows Defender / SmartScreen warnings until signed, so keep stub behavior clean and test in Windows Sandbox).
- Don't touch my other repos in this session. A later step will migrate them one by one, starting with a pilot app (OmniConvert).
- Commit style: a plain sentence saying what changed and why. No `feat:`/`fix:`/`chore:` prefixes.

## How to work
1. Write a short design doc (`docs/design.md`): architecture, PE payload format (Authenticode-safe), UI library choice (including Sandbox/WARP rendering and licensing), icon handling (with full `RT_ICON` purge), tracked file manifest & uninstaller trampoline, existing-install modes & legacy uninstaller migration, user-data guardrails, version comparison, silent exit codes, and a milestone list. At the same time, make the HTML mockups in `design/mockups/`. Show me both and wait for my OK before writing installer code.
2. Then build it milestone by milestone. First milestone: preview mode matching the approved mockups (verified in Windows Sandbox), plus a working end-to-end install + stripped self-deleting uninstall of the dummy app with per-app icons, registry entries that satisfy the contract, tracked file manifest, and the user-data question on uninstall, with the automated tests passing. Update/reinstall/downgrade/migration can follow in the next milestone.
3. After each milestone, tell me what to run to see it (preview command, screenshot folder, test installer path).
4. Create `AGENTS.md` (with a `CLAUDE.md` that just says `@AGENTS.md`) listing the commands, architecture, the GloriaLauncher contract, the per-app icon rule, the GloriaSetup brand-icon rule, the PE payload/signing order, and the user-data rules, so later sessions keep them intact.