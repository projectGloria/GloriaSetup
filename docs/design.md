# GloriaSetup design

GloriaSetup is one branded, per-user Windows installer shared by every projectGloria app. Its output is a single
`<AppName>_<version>_x64-setup.exe`, built from an app folder and a small manifest.

This document records the decisions made before any installer code exists. The visual source of truth is
`design/mockups/index.html`, which must be approved before the stub UI is written.

## 1. Architecture

```
GloriaSetup/
├─ crates/
│  ├─ gloria-core/     shared library: manifest schema + validation, payload format, version compare,
│  │                   NSIS-style CLI parser, install-manifest model, path guardrails (no Win32 UI)
│  ├─ gloria-stub/     the installer/uninstaller exe (Win32 + Direct2D UI, install engine)
│  └─ gloria-pack/     CLI: stub + app folder + manifest → setup.exe
├─ examples/dummy/     tiny test app + two icons + pack script (0.1.0 and 0.2.0)
├─ assets/brand/       GloriaSetup.ico / GloriaSetup.png (unchanged originals)
├─ assets/fonts/       Archivo, Inter, JetBrains Mono (OFL 1.1), Latin + Latin Extended subsets
├─ design/mockups/     clickable HTML mockups (approved before UI code)
├─ sandbox/            GloriaSetup.wsb (Windows Sandbox, build output mapped read-only)
├─ action.yml          composite GitHub Action used by app repos in their release job
└─ docs/design.md
```

`gloria-core` has no UI and no Win32 calls that need a desktop session, so almost all logic is unit-testable on any
Windows runner. The stub is a thin shell over it: UI, registry, shell links, processes, file system.

### App manifest (`gloria.toml`, in the app repo)

```toml
id            = "OmniConvert"          # stable key; registry subkey and file names derive from it
name          = "OmniConvert"          # DisplayName, exactly as the GitHub repo name
version       = "0.2.0"                # may be overridden by `gloria-pack --version` from the release tag
main_exe      = "omniconvert.exe"      # relative to the app folder
icon          = "src-tauri/icons/icon.ico"   # .ico, or .png ≥ 256×256
accent        = "#f59e0b"
description   = "Universal offline media & document converter"   # optional, shown on welcome
needs_webview2 = true
data_folders  = ["%APPDATA%\\com.omniconvert.app", "%LOCALAPPDATA%\\com.omniconvert.app"]
legacy_names  = ["OmniConvert"]        # optional extra DisplayNames to match old installers
app_user_model_id = "com.omniconvert.app"   # optional, written on shortcuts (Tauri notifications)
context_menu  = "Convert with OmniConvert"   # optional right-click label for folders, backgrounds, drives
```

`publisher` is always `projectGloria` and is not configurable (GloriaLauncher contract).

## 2. PE payload format (Authenticode-safe)

```
┌──────────────────────────────┐ 0
│ PE headers + sections        │  stub code, resources (icon + version info patched by gloria-pack)
├──────────────────────────────┤ overlay_start = max(PointerToRawData + SizeOfRawData) over all sections
│ Payload header (64 bytes)    │  magic "GLSETUP\0", format version, flags, offsets/lengths, SHA-256
│ manifest.json (utf-8)        │  the validated runtime manifest (+ pack metadata)
│ archive (tar, zstd)          │  app files (+ uninstall.exe, + WebView2 bootstrapper when needed)
│ zero padding to 8 bytes      │
├──────────────────────────────┤ payload_end
│ WIN_CERTIFICATE (optional)   │  added by signtool; located by DataDirectory[SECURITY]
└──────────────────────────────┘ EOF
```

Reading at run time (`gloria-core::payload`):

1. Open our own image (`GetModuleFileNameW`), parse DOS/NT headers and the section table.
2. `overlay_start` = end of the last section's raw data. The header is read **forward from there**, never from EOF.
3. If `DataDirectory[IMAGE_DIRECTORY_ENTRY_SECURITY]` is set, its file offset is the hard upper bound. The payload must
   end at or before it; otherwise the file is rejected as corrupt.
4. Verify magic, format version, bounds and the SHA-256 over manifest + archive, then stream-decompress the archive.

Why forward from the section end: signtool appends the certificate table after the overlay and may insert alignment
padding, so "read the footer at EOF" breaks once a file is signed. Our own 8-byte padding keeps signtool from having
to pad. The test suite appends fake `WIN_CERTIFICATE` bytes and patches the security directory to prove the round-trip.

**Pack order (strict):** `patch resources (icon purge + new icon + version info)` → `append payload` → `sign` (optional,
last). `UpdateResource` rewrites the whole PE and would drop an overlay, and any change after signing invalidates the
signature, so the order is enforced in code, not just documented.

## 3. UI library: direct Direct2D + DirectWrite via `windows-rs`

Candidates were egui, Slint and hand-rolled Direct2D/DirectWrite. Choice: **Direct2D/DirectWrite**, with a small
retained widget layer (~15 widgets: text, button, checkbox, radio card, progress bar, path field, icon, badge, modal).

| Requirement | egui | Slint | Direct2D/DirectWrite |
|---|---|---|---|
| Windows Sandbox / WARP | `glow` needs OpenGL 2+ (Sandbox has none); `wgpu` DX12 WARP works but adds MBs | Software renderer works everywhere | D2D falls back to WARP automatically; it is the OS compositor's own path |
| Glow / blur / gradients | No real blur; glow faked with stacked shapes | Drop-shadow only, no Gaussian effect on arbitrary content | Native Gaussian blur, shadow and radial-gradient effects (`ID2D1Effect`) |
| Text quality at 100–200 % DPI | Own rasterizer, slightly soft | Good | ClearType/grayscale DirectWrite, the same text Windows itself draws |
| Embedded fonts | Yes | Yes | `IDWriteInMemoryFontFileLoader` (Win10 1703+) |
| Licensing | MIT/Apache | GPLv3, or Royalty-free 2.0 **requiring attribution** (AboutSlint or a badge on the app's page) for every app in the suite, incl. closed-source ones | MIT/Apache (`windows-rs`); fonts OFL |
| Headless `--screenshot` | Needs a GPU context | Software renderer → buffer | `ID2D1RenderTarget` on a WIC bitmap: CPU-only, deterministic |
| Zero flicker | Possible | Possible | Full control: hidden + DWM-cloaked window, first `Present`, then show |
| Stub size | ~3–6 MB with wgpu | ~2–3 MB | ~1 MB code + ~0.4 MB fonts |

Costs we accept: more code for layout and input, and only basic accessibility. Mitigations: keyboard navigation
(Tab / Shift+Tab / Enter / Esc / Space) on every control, high-contrast-safe colors, and the folder path uses a real
Win32 `EDIT` child control (styled with `WM_CTLCOLOREDIT`) so IME, clipboard, selection and screen readers work there.

How each mockup effect maps:

- Accent glow behind icon and on progress bar → `D2D1_GAUSSIANBLUR` over the accent-tinted shape. Exact.
- Radial accent wash in the background → radial gradient brush. Exact.
- Page transitions (fade + 8 px slide, 180 ms ease-out), button hover/press, progress shimmer → own animation clock;
  rendering runs at the monitor rate only while something animates, idle otherwise. Exact.
- Backdrop blur of the **desktop** behind the window (Mica/Acrylic) is not used: it varies with wallpaper and
  is unavailable in Sandbox. The window is opaque obsidian, as in the mockups.
- Modal dimming over a page → blur of our own previous frame + 60 % obsidian overlay. Exact.

**WARP / Sandbox:** the device is created with `D3D_DRIVER_TYPE_HARDWARE` and falls back to `D3D_DRIVER_TYPE_WARP`;
if both fail (no D3D at all), we render with the WIC software target and blit via `UpdateLayeredWindow`/GDI.
Milestone 1 is not complete until the UI has been checked inside Windows Sandbox.

**Zero flicker:** window class has no background brush; the window is created without `WS_VISIBLE` and cloaked
(`DWMWA_CLOAK`); the first frame is presented, then it is uncloaked and shown. No white flash, no resize jump.

**Window:** fixed 640 × 440 DIP, centered on the monitor that has the cursor, per-monitor DPI v2 (manifest), custom
title bar drawn by us (`WM_NCCALCSIZE` removes the frame, `WM_NCHITTEST` gives drag + snap, DWM keeps the shadow).
Corner style (Win11 rounded vs square) is chosen in the mockups.

## 4. Icons

- The stub is built with `assets/brand/GloriaSetup.ico` as icon group 1 (all 11 sizes), so a bare stub and preview
  runs show GloriaSetup's icon. `gloria-pack.exe` embeds the same icon.
- `gloria-pack` loads the app icon: `.ico` as is, or a PNG ≥ 256×256 (anything smaller is an error). Missing standard
  sizes (16, 24, 32, 48, 64, 256) are generated from the largest image with Lanczos3. Entries < 256 are stored as
  32-bit BMP, 256 as PNG (Explorer compatibility).
- **Full purge:** `gloria-pack` enumerates the stub's resources (`EnumResourceNamesW` on a datafile-loaded copy) and
  deletes **every** `RT_GROUP_ICON` and `RT_ICON` entry (all languages) in the same `BeginUpdateResource` session that
  writes the new group 1 and its `RT_ICON` ids. A test asserts that no GloriaSetup icon image remains in the output.
- Version info (`RT_VERSION`) is rewritten: `ProductName` = name, `FileDescription` = "`<Name>` Setup",
  `ProductVersion`/`FileVersion` = version (numeric `FIXEDFILEINFO` from the core of the SemVer), `CompanyName` =
  "projectGloria".
- The stub loads its window icons and the large welcome icon **from its own resources**, so whatever `gloria-pack`
  wrote is what the window, taskbar and Explorer show. `--preview --icon <file>` overrides this for previews only.
- No icon in the manifest → pack error. `--allow-default-icon` keeps the GloriaSetup icon (local testing only; the
  manifest gets a flag and the UI shows a small "TEST BUILD" badge so it can never be mistaken for a release).
- `UpdateResource` needs a Windows host, so packing runs on `windows-latest` in CI.

## 5. Install engine

### Paths
- Default folder: `%LOCALAPPDATA%\Programs\Project Gloria Softwares\<Name>`, so every projectGloria app sits under
  one per-user parent. Uninstall removes that parent once it is empty. Registry: `HKCU` only.
- Rejected folders (exit code 6 in silent mode, inline error in UI): drive roots, UNC paths, anything under
  `%ProgramFiles%`, `%ProgramFiles(x86)%`, `%ProgramW6432%`, `%WINDIR%`, `%ProgramData%`, `%TEMP%`, any declared data
  folder, and any folder where a write probe fails.
- **Safe custom folders:** if the chosen folder exists, is not empty, and has no `install-manifest.json` with our app
  id, `\<Name>` is appended. The UI shows the final path live under the field.

### Tracked file manifest
`<install>\install-manifest.json` records schema version, app id, version, every installed file (relative path,
size, SHA-256), every directory we created, shortcuts we created, and the user's options (desktop shortcut, …).
Update and uninstall touch only what it lists; `remove_dir_all(InstallLocation)` is never called. Directories are
removed only if empty after their tracked files are gone.

### Atomic replacement
All work happens in `<install>\.gloria\` (hidden, same volume, so renames are atomic):

1. **Stage:** extract the whole archive to `.gloria\staging`, plus the new `install-manifest.json`. Disk-full / AV
   lock here → delete `.gloria`, nothing changed.
2. **Journal:** write `.gloria\journal` (flushed to disk) before the first rename. It is a small text file: a
   `GLORIA-JOURNAL 1` header, the app id (`A`), whether the install folder is new (`N`), new parent folders (`D`),
   every file to place (`P`, including `install-manifest.json`) and every obsolete file (`O`). Code:
   `gloria-core::journal`.
3. **Commit:** for each file to place, rename the old one (if any) into `.gloria\backup`, then rename the staged one
   into place. Then rename each obsolete file into `.gloria\backup`. Then write the registry.
4. **Finalize:** delete the journal (this is the commit point), then `.gloria`, then shortcuts and the data purge.
5. **Rollback** on any failure before the commit point, in this run or the next: in reverse, an obsolete file whose
   original is missing comes back from backup; a placed file goes back to staging and its backup goes back into
   place; new folders are removed. The journal is deleted last, so a crash during rollback just rolls back again.
   Which step a file reached is read from which files exist, so the journal never needs rewriting.

Every setup and uninstaller run checks for a journal before doing anything else (after closing the app). A journal
names its app id, so a half-done fresh install is still recognised as "ours" and not treated as a foreign folder.
`.gloria` without a journal is leftover work and is deleted, never restored from. Known gap: a crash between the
registry write and deleting the journal rolls the files back but leaves the registry one version ahead until the
next setup run rewrites it.

Debug builds of the stub exit with code 99 just before rename `n` when `GLORIA_TEST_CRASH_AT=n` is set (`n = renames`
is the registry write, `renames + 1` deleting the journal). The e2e tests use it to kill an update at each step.

### Shortcuts
Start Menu (`%APPDATA%\Microsoft\Windows\Start Menu\Programs\<Name>.lnk`) optional and on by default; desktop optional
and off by default. Silent installs, updates and reinstalls keep the choices stored in the install manifest (an install
manifest without `start_menu_shortcut` counts as having one). Unticking a shortcut on update removes it. Both target the
main exe (so they show the app's icon) and carry `app_user_model_id` when the manifest has one.

### Context menu
With `context_menu` set, the install writes a classic shell verb under three HKCU roots:
`Software\Classes\Directory\shell`, `Directory\Background\shell` and `Drive\shell`, each as `projectGloria.<id>`
with `(Default)` = the label, `Icon` = the main exe and `command\(Default)` = `"<main exe>" "%V"`. A later version
without `context_menu` deletes the keys, and uninstall always deletes them. Failures are logged, not rolled back. On
Windows 11 classic verbs show under **Show more options**. Explorer passes a drive root as `"C:\"`, so the app
receives `C:"` and has to strip the quote itself.

### WebView2
If `needs_webview2`: present when `pv` exists and is not `0.0.0.0` under either
`HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` or
`HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-…}`. If missing, run the Evergreen bootstrapper bundled in the
payload (only for such apps) with `/silent /install`, then re-check. Failure → exit code 5. The stub has no HTTP/TLS.

### Closing running processes
Enumerate processes, read `QueryFullProcessImageNameW`, and match by path prefix inside `InstallLocation` (case
-insensitive, normalized, trailing separator). Interactive: ask first (modal), send `WM_CLOSE` to their top-level
windows, wait up to 8 s, re-check locks with brief retries (WebView2 children exit after their host), then offer
**Force close** (`TerminateProcess`) or cancel. Silent: the same graceful attempt, then force-close; still locked →
exit code 3, nothing changed.

## 6. Registry (GloriaLauncher contract)

Subkey: `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\projectGloria.<id>`. The prefix keeps it from
colliding with Tauri-NSIS subkeys (which use the bare product name) during migration.

| Value | Type | Content |
|---|---|---|
| `DisplayName` | SZ | `name`, nothing appended |
| `DisplayVersion` | SZ | version, no `v` |
| `InstallLocation` | SZ | install folder, unquoted |
| `DisplayIcon` | SZ | `C:\…\omniconvert.exe`, bare full path |
| `UninstallString` | SZ | `"C:\…\uninstall.exe"` |
| `QuietUninstallString` | SZ | `"C:\…\uninstall.exe" /S` |
| `Publisher` | SZ | `projectGloria` |
| `EstimatedSize` | DWORD | total installed bytes / 1024 (KB) |
| `NoModify`, `NoRepair` | DWORD | 1 |
| `GloriaSetup` | SZ | stub version (marks our own keys) |

GloriaLauncher (`src-tauri/src/lib.rs`, `system_installed`) was read to confirm: `parse_icon_path` strips quotes and a
trailing `,N`, then requires an existing `.exe` that isn't a helper (`unins*`, `uninstall*`, …). A bare path is the
simplest value it accepts. It matches `normalize(strip_version(DisplayName))`, scans HKCU plus HKLM in both views,
and runs `UninstallString` via `cmd /C`, which handles the quoted path.

## 7. Uninstaller

- `gloria-pack` builds `uninstall.exe` itself: the resource-patched stub (app icon; `FileDescription` = "`<Name>`
  Uninstaller") + a payload with the manifest and **no archive** (flag `UNINSTALLER`). It is signed like the setup
  when a certificate exists, then stored as a file inside the setup's archive. So the installed uninstaller is never a
  copy of the full setup and never needs PE surgery on the user's machine.
- Started normally, `uninstall.exe` opens the uninstall screen. On confirm it copies itself to
  `%TEMP%\GloriaSetup-<id>-<rand>.exe`, starts that with `--cleanup "<install>" --wait-pid <pid>` (plus the user's
  choices), and exits.
- The temp copy waits for the PID, closes running app processes, removes tracked files, shortcuts, the registry key,
  user data only if chosen, then empty directories, and finally deletes itself through a hidden
  `cmd /c ping -n 3 127.0.0.1 >nul & del "<self>"` started just before it exits. (Changed in M1: the planned
  `MoveFileExW(…, MOVEFILE_DELAY_UNTIL_REBOOT)` writes `PendingFileRenameOperations` under HKLM and needs admin, so it
  fails for a per-user installer. If the `del` ever misses, the leftover is one small file in `%TEMP%`.) Progress and the result are shown by the temp copy, so the user sees
  one continuous window.

## 8. Existing installs

Detection order on start: our own key (`projectGloria.<id>`), then legacy keys.

| Found | Mode | Data question | Silent behaviour |
|---|---|---|---|
| nothing | Fresh install | — | install |
| ours, older | Update | never (always kept) | update |
| ours, same | Reinstall / Uninstall | "clean reinstall", default keep | reinstall |
| ours, newer | Downgrade warning (explicit confirm) | yes, default keep | exit 2 unless `/ALLOWDOWNGRADE` |
| legacy key | Upgrade from previous installer | yes, default keep | migrate; exit 4 if elevation fails |

Update reuses the install folder and stored options without asking.

### Legacy migration
Scan `Uninstall` under HKCU and HKLM with both `KEY_WOW64_64KEY` and `KEY_WOW64_32KEY`, match
`normalize(strip_version(DisplayName))` against `name` + `legacy_names`, skip our own key. Uninstall command by type:

| Type (detected by, in this order) | Command |
|---|---|
| MSI (`WindowsInstaller=1` / `MsiExec` + GUID) | `msiexec.exe /X{GUID} /qn /norestart` |
| Inno (`*_is1` subkey / `Inno Setup:` values) | `"unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART` |
| NSIS (Tauri; `NullsoftInst` in the uninstaller exe) | `"<dir>\uninstall.exe" /S _?=<dir>`, then delete that leftover uninstaller |
| anything else with an absolute `QuietUninstallString` | that string (none: the UI runs `UninstallString`, silent mode stops with exit 1) |

After the uninstaller exits, wait up to 60 s for its Uninstall key to disappear (Inno and some others hand off to a
`%TEMP%` copy and return early).

`_?=` keeps NSIS synchronous (no `%TEMP%` copy racing with us). HKLM entries need elevation: the UI says so and asks;
only the old uninstaller runs elevated (`ShellExecuteExW` "runas"); cancel → stop with exit/UI code 4.

**Keeping data:** if the user keeps data, declared data folders are copied to `%TEMP%\GloriaSetup-backup-<id>` before
the old uninstaller runs and restored afterwards if anything is missing. (Tauri's NSIS uninstaller only deletes app data
when its own checkbox is ticked, and `/S` leaves it unticked, but we don't rely on that.)

## 9. User data guardrails

`data_folders` are validated at pack time and again at run time (same code in `gloria-core`):

- Must start with `%APPDATA%\`, `%LOCALAPPDATA%\` or `%USERPROFILE%\` followed by at least one concrete component.
- Rejected: `..`, `.`, empty components, wildcards, drive letters or `:` after the root, UNC, bare roots, and a
  deny-list of shared folders (e.g. `%LOCALAPPDATA%\Programs`, `\Temp`, `\Microsoft`, `\Packages`, and under
  `%USERPROFILE%`: `Documents`, `Desktop`, `Downloads`, `Pictures`, `Music`, `Videos`, `AppData`, `OneDrive`).
- Must not overlap the install folder.
- Deletion never follows junctions or symlinks (reparse points are unlinked, not traversed).

Data is removed only when the user ticks "Also remove my settings and data" (unchecked by default, lists each folder
with its size), or with `/PURGE` in silent mode. A normal update never asks and never deletes.

The uninstall page has a second, off-by-default box, "Don't leave anything behind: remove everything" (`/WIPE` in
silent mode). It ticks the data box with it and, on top of `/PURGE`:
- empties and removes the whole install folder, including files the app wrote there itself (logs, caches), but only
  when setup created that folder and it passes the same shared/system-folder guards as legacy migration; a folder
  that existed before the install keeps everything setup did not put there;
- deletes what GloriaSetup left in `%TEMP%` for this app id (`GloriaSetup-<id>-<timestamp>.log`, trampoline copies
  `GloriaSetup-<id>-<8 hex>.exe`, the migration backup `GloriaSetup-backup-<id>`), matched strictly so another app
  id never matches; the trampoline deletes its own log with its exe after a successful run.

## 10. Version comparison

`gloria-core::version` implements SemVer 2.0 precedence with two extensions:

- A leading `v`/`V` is ignored; build metadata (`+…`) is ignored.
- The core has 1 to 4 numeric parts; missing parts are `0` (`1.2` = `1.2.0` = `1.2.0.0`). Each part is compared as an
  integer (`0.10.0 > 0.2.0`).
- A pre-release sorts before its release (`1.0.0-beta.1 < 1.0.0`); identifiers compare per SemVer (numeric
  numerically, alphanumeric by ASCII, numeric < alphanumeric, shorter set first when equal prefix).
- An unparseable installed version is treated as older (so the user gets an update, never a downgrade warning).

GloriaLauncher ignores pre-release tags, so it won't offer `1.0.0` over `1.0.0-beta.1`; GloriaSetup itself treats that
as an update. That difference is harmless.

## 11. Command line and exit codes

The raw `GetCommandLineW()` is parsed NSIS-style: `/D=` must be last and takes the rest of the line verbatim (quotes
optional), so `/D=C:\My Apps\Foo` works. Flags are case-insensitive.

| Flag | Meaning |
|---|---|
| `/S` | silent install / uninstall |
| `/D=<path>` | install folder |
| `/PURGE` | remove user data (silent) |
| `/WIPE` | uninstall leaves nothing behind: `/PURGE` plus leftover files in the install folder and `%TEMP%` logs |
| `/ALLOWDOWNGRADE` | permit downgrade (silent) |
| `/RELAUNCH` | start the app after a silent install/update |
| `--preview`, `--page <name>`, `--lang <code>`, `--manifest <file>`, `--icon <file>`, `--screenshot <dir>` | preview |
| `--dry-run` | log every action without doing it |
| `--cleanup <dir> --wait-pid <pid>` | internal (uninstall trampoline) |

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | generic / unexpected error |
| 2 | downgrade refused (no `/ALLOWDOWNGRADE`) |
| 3 | running app could not be closed / files locked |
| 4 | UAC elevation cancelled or failed (legacy HKLM migration) |
| 5 | WebView2 installation failed |
| 6 | invalid arguments or non-writable install path |

Every run writes a log to `%TEMP%\GloriaSetup-<id>-<timestamp>.log`; the error screen links to it.

## 12. Languages

`en`, `fr`, `es`, `tr`, chosen from `GetUserDefaultUILanguage()` with an English fallback; `--lang` overrides.
Strings live in `crates/gloria-stub/i18n/*.toml` and a test checks every key exists in all four. Fonts are subset to
Latin + Latin Extended-A so Turkish `ğ ş ı İ` render with the embedded fonts.

## 13. CI and signing

`action.yml` (composite, `windows-latest`): at a version tag (`uses: projectGloria/GloriaSetup@v0.1.0`) it downloads
that release's `gloria-pack.exe` + `gloria-stub.exe` and checks them against `SHA256SUMS.txt`; at a branch or commit
it builds them from source. It runs `gloria-pack`, which signs with `signtool` only when the `certificate` input (base64
PFX from secrets) is non-empty: the PFX goes to `RUNNER_TEMP` for that step only and the password reaches signtool
through `GLORIA_SIGN_PASSWORD`, never the command line of a logged step. Signing happens inside `gloria-pack` because
`uninstall.exe` must be signed before it is archived; the signed setup is re-read to prove its payload survived. Then
the action uploads `<Name>_<version>_x64-setup.exe` to the release.

This repo's CI (`.github/workflows/ci.yml`) builds warning-free, runs all tests with a throwaway self-signed
certificate (`tools/make-test-cert.ps1`) so the signing test runs too, checks the stub stays under 5 MB, and runs the
action on the dummy app. `release.yml` publishes the binaries for a `v*` tag that matches the workspace version. Unsigned builds may trigger SmartScreen until a certificate
exists; the stub stays "clean" (no self-modification, no network, no packed/encrypted code) to reduce Defender noise.

## 14. Milestones

1. **Preview + end-to-end install/uninstall.** Direct2D UI matching the approved mockups (all screens, 4 languages,
   `--preview`, `--page`, `--lang`, `--screenshot`), verified in Windows Sandbox. `gloria-pack` with icon purge,
   version info and payload. Fresh install of the dummy app, registry contract, shortcuts, tracked manifest,
   stripped self-deleting `uninstall.exe` with the data question, `/S` + `/PURGE`, exit codes, `--dry-run`,
   `sandbox/GloriaSetup.wsb`, `AGENTS.md`, automated tests.
2. **Existing installs.** Update (obsolete-file removal), reinstall, downgrade, process closing, full rollback journal,
   remaining silent-mode rules and tests. Done: crash recovery at every commit step, closing a running app (asked
   first, forced in silent mode), locked files (exit 3, old version kept), `/RELAUNCH`.
3. **Migration + WebView2.** Legacy NSIS / MSI / Inno detection in both registry views, elevation, data backup and
   restore, bundled WebView2 bootstrapper. Done: automated tests cover per-user migrations (quiet string and NSIS
   `_?=`, data restored even when the old uninstaller wipes it, `/PURGE`) and the bootstrapper (runs only when the
   runtime is missing, failure exits 5 with nothing installed). The elevated HKLM path (UAC accepted, and declined =
   exit 4 with nothing changed and no backup left) was checked by hand with a fake all-users install.
4. **CI + polish.** Composite action, optional signing, README "How to test", social preview image. Done: signing in
   `gloria-pack` (tested with a self-signed certificate), the action (tested by CI on the dummy), CI and release
   workflows, `assets/social-preview.png`.
5. **Pilot.** OmniConvert migrated in its own repo (separate session).
