<p align="center"><img src="assets/brand/GloriaSetup.png" width="128" alt="GloriaSetup"></p>

<h1 align="center">GloriaSetup</h1>

<p align="center">A per-user Windows installer for projectGloria apps. No admin, one small exe, styled like GloriaLauncher.</p>

- **Stub** (`crates/gloria-stub`): the setup and uninstaller, a Win32 + Direct2D app in English, French, Spanish
  and Turkish.
- **`gloria-pack`** (`crates/gloria-pack`): turns the stub, an app folder and a `gloria.toml` into
  `<Name>_<version>_x64-setup.exe`, with the app's own icon.
- **GitHub Action**: packs and uploads the setup in an app's release job (milestone 3).

How it works: [docs/design.md](docs/design.md). Approved screens: [design/mockups/index.html](design/mockups/index.html).

## Build

Needs Rust 1.80+ (msvc, x64) on Windows 10/11.

```powershell
cargo build --release -p gloria-stub -p gloria-pack
cargo test --workspace
```

## Pack an app

```powershell
target\release\gloria-pack.exe --app path\to\built\app --manifest path\to\gloria.toml `
  --version 1.2.0 --stub target\release\gloria-stub.exe --out dist
```

`gloria.toml` is described in [docs/design.md § App manifest](docs/design.md#app-manifest-gloriatoml-in-the-app-repo).
`gloria-pack --inspect <setup.exe>` prints what a setup contains.

## Setup command line

| Flag | Meaning |
|---|---|
| `/S` | silent install or uninstall |
| `/D=<path>` | install folder (must be last; spaces need no quotes) |
| `/PURGE` | silent uninstall or clean reinstall also removes user data |
| `/ALLOWDOWNGRADE` | silent install may install an older version |
| `/RELAUNCH` | start the app after a silent install |
| `--dry-run` | log every action, change nothing |

Exit codes: 0 ok, 1 error, 2 downgrade refused, 3 app could not be closed or files locked, 4 elevation cancelled
(legacy migration), 5 WebView2 failed, 6 invalid arguments or install folder.
Each run logs to `%TEMP%\GloriaSetup-<id>-<timestamp>.log`.

Run setups with flags from PowerShell or cmd. Git Bash rewrites `/S` into a path.

## How to test

Work through these in order. Each step only needs the one before it.

### 1. Preview the screens (changes nothing)

```powershell
cargo build --release -p gloria-stub
target\release\gloria-stub.exe --preview
target\release\gloria-stub.exe --preview --page uninstall --lang tr
target\release\gloria-stub.exe --preview --manifest examples\dummy\gloria.toml --icon examples\dummy\icons\dummy-a.ico
target\release\gloria-stub.exe --screenshot out\shots
```

- Pages: `welcome`, `options`, `options-error`, `progress`, `done`, `update`, `update-running`, `reinstall`,
  `downgrade`, `migrate`, `uninstall`, `uninstall-progress`, `uninstall-done`, `error`.
- Buttons work and jobs are simulated. Nothing is installed.
- `--screenshot` writes every page in every language to `out\shots\<lang>\<page>.png`, plus `update-waiting` and
  `update-stuck`. Compare them with the mockups.
- Running the bare stub with no flags also opens the preview. It is the only exe that shows the GloriaSetup icon.

### 2. Install, update and uninstall the dummy app on your PC

```powershell
powershell -ExecutionPolicy Bypass -File examples\dummy\pack.ps1
```

This builds `dist\dummy\GloriaSetupDummy_0.1.0_x64-setup.exe` (packed with icon A) and `..._0.2.0_...` (icon B).
`dummy.exe` itself always carries icon A, the way a real app exe carries its own icon. The dummy app shows its version in a message box and counts its runs in `%APPDATA%\com.gloria.dummy\settings.json`.

**Install 0.1.0.** Run the 0.1.0 setup from Explorer, keep the default folder, tick "Launch when done". Check:

- **Explorer:** both setup exes show their own icons (A and B), not the GloriaSetup icon. File properties → Details
  show "GloriaSetup Dummy Setup", the version and "projectGloria".
- **Install folder:** `C:\Users\you\AppData\Local\Programs\GloriaSetup Dummy` has `dummy.exe`, `uninstall.exe` (with
  icon A), `version.txt`, `docs\`, `old-notes.txt` and `install-manifest.json`.
- **Start Menu:** "GloriaSetup Dummy" with icon A (from `dummy.exe`) starts the app. A desktop shortcut exists only if you ticked it.
- **Settings → Apps → Installed apps:** "GloriaSetup Dummy" with icon A, version 0.1.0, publisher projectGloria, a size, and
  Uninstall works.
- **regedit:** `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall\projectGloria.GloriaDummy` has
  `DisplayName` = `GloriaSetup Dummy`, `DisplayVersion` = `0.1.0` (no `v`), `InstallLocation` (no quotes),
  `DisplayIcon` = the bare path to `dummy.exe`, `UninstallString` = the quoted `uninstall.exe`, `QuietUninstallString`
  = the same + ` /S`, `Publisher` = `projectGloria`, `EstimatedSize`, `NoModify` = 1, `NoRepair` = 1, `GloriaSetup`.
  Nothing is written under HKLM.
- **Data folder:** run the app once, then `%APPDATA%\com.gloria.dummy\settings.json` exists.

**Update to 0.2.0.** Run the 0.2.0 setup. It opens on the update screen (0.1.0 → 0.2.0). Try it once with the dummy
still open to see the "close the app" prompt. After the update:

- The app says 0.2.0, `old-notes.txt` is gone, the run count in `settings.json` kept going.
- A file you put in the install folder yourself (say `mine.txt`) is still there.
- regedit and Settings show 0.2.0. `uninstall.exe` now shows icon B (the setup's
  icon). The Start Menu entry keeps icon A because it comes from `dummy.exe`.

**Reinstall and downgrade.** Run 0.2.0 again: the reinstall screen offers Repair or Uninstall. Run 0.1.0: the
downgrade screen warns first. Silent downgrades are refused:

```powershell
(Start-Process dist\dummy\GloriaSetupDummy_0.1.0_x64-setup.exe -ArgumentList '/S' -Wait -PassThru).ExitCode   # 2
```

**Uninstall.** Use Settings → Apps → Uninstall (or the Start Menu, or `uninstall.exe`).

- Leave "Also remove my settings and data" unticked. Afterwards the install folder holds only your `mine.txt`, the
  Start Menu entry and registry key are gone, and `%APPDATA%\com.gloria.dummy` is still there.
- After a few seconds no `GloriaSetup-GloriaDummy-*.exe` is left in `%TEMP%`. That is the uninstaller's temp copy,
  which deletes itself.
- Install again and uninstall with the box ticked: the data folder is gone too.

**Silent run and dry run:**

```powershell
$s = 'dist\dummy\GloriaSetupDummy_0.2.0_x64-setup.exe'
(Start-Process $s -ArgumentList '/S --dry-run' -Wait -PassThru).ExitCode                       # 0, nothing changes
(Start-Process $s -ArgumentList '/S /D=C:\Users\you\Apps\Gloria Dummy' -Wait -PassThru).ExitCode
$u = 'C:\Users\you\Apps\Gloria Dummy\uninstall.exe'
(Start-Process $u -ArgumentList '/S' -Wait -PassThru).ExitCode                                 # 0, keeps data
```

Install silently again, then run `uninstall.exe` with `/S /PURGE` to remove the data folder too. Check the
`%TEMP%\GloriaSetup-GloriaDummy-*.log` files: the dry run lists every file, shortcut and registry value it would write.

`cargo test --workspace` runs the same install, update, reinstall, downgrade and uninstall flow automatically, with
unique app ids, and cleans up after itself.

### 3. Clean machine and software rendering: Windows Sandbox

Windows Sandbox is a throwaway Windows desktop. It starts clean each time and has no GPU here, so Direct2D renders
with WARP, the software renderer that VMs and remote desktops get.

1. **Turn it on (once):** Windows 10/11 Pro, Enterprise or Education, with virtualization on in the BIOS. Open
   "Turn Windows features on or off", tick **Windows Sandbox**, then reboot. This needs admin. Or, in an admin
   PowerShell: `Enable-WindowsOptionalFeature -Online -FeatureName Containers-DisposableClientVM -All`.
2. **Start it:** build the dummy setups (step 2), then run:

   ```powershell
   powershell -ExecutionPolicy Bypass -File sandbox\run.ps1
   ```

   The script copies the release stub to `dist\GloriaSetup-preview.exe` and fills this checkout's `dist` path into
   [sandbox/GloriaSetup.wsb](sandbox/GloriaSetup.wsb). Windows Sandbox needs absolute paths, so open the `.wsb`
   through the script. The sandbox opens with `Desktop\dist` in Explorer. It is read-only, has no network and has no
   vGPU.
3. **Check in the sandbox:**
   - `GloriaSetup-preview.exe` renders every screen correctly: text, glow, icons and animations, no black or empty
     areas. Resize, drag and close the window.
   - `dummy\GloriaSetupDummy_0.1.0_x64-setup.exe` installs, then repeat the step 2 checks. Microsoft Defender may
     scan the unsigned setup first. Without network, SmartScreen can't block it.
   - Update to 0.2.0, then uninstall with and without data.
4. Close the sandbox window to throw everything away.

To move a log out, copy it from `%TEMP%` in the sandbox and paste it on your desktop. Clipboard sharing is on.

### 4. A real app

Pack a real projectGloria app, for example OmniConvert. Put the files the app needs at run time in one folder (for a
Tauri app, the release exe plus any resources), and write a `gloria.toml` for it like the example in
[docs/design.md](docs/design.md#app-manifest-gloriatoml-in-the-app-repo). Then:

```powershell
target\release\gloria-pack.exe --app C:\Users\you\staging\OmniConvert --manifest C:\Users\you\staging\gloria.toml `
  --version 0.2.0 --stub target\release\gloria-stub.exe --out dist
```

- Try it in the Sandbox first, then on your PC.
- Check the same things as in step 2, with the app's own name, icon, accent colour and data folders.
- **Migration from an old installer:** install the app's previous NSIS, MSI or Inno build first, then run the new
  setup. It opens on the migrate screen, names the old install (and says "all users" when it needs admin), backs up
  the data folders, runs the old uninstaller (a UAC prompt only for all-users installs; declining stops with code 4)
  and installs. The app's settings survive unless you tick the data box. Try an all-users (HKLM) install in the
  Sandbox: the automated tests don't cover that path.
- **WebView2:** for `needs_webview2` apps, pass `--webview2-bootstrapper <MicrosoftEdgeWebview2Setup.exe>` (download it
  from Microsoft yourself). Where the runtime is missing, the setup runs it first. Without network it fails, and the
  setup exits with code 5 and leaves nothing installed.
- **GloriaLauncher detection:** start GloriaLauncher after installing. The app shows as installed with the right
  version, and Launch and Uninstall work from there. GloriaLauncher only lists apps in its catalogue, so the dummy
  never shows up there. The dummy only proves the registry fields.
