# GloriaSetup: agent notes

A per-user (no admin) Windows installer for projectGloria apps. Rust stub + `gloria-pack` CLI + GitHub Action.
Full design: `docs/design.md`. Approved UI: `design/mockups/index.html`. Original brief: `INITIAL_PLAN.md`.

## Commands

```powershell
cargo build --workspace                       # debug; must stay warning-free
cargo test --workspace                        # unit + e2e (e2e installs real dummy apps under HKCU, then cleans up)
cargo build --release -p gloria-stub          # stub must stay < 5 MB before the payload (now ~1.7 MB)
target\release\gloria-stub.exe --preview [--page <id>] [--lang en|fr|es|tr] [--manifest gloria.toml] [--icon x.ico]
target\release\gloria-stub.exe --screenshot out\shots     # every page x language as PNG, no window
powershell -ExecutionPolicy Bypass -File examples\dummy\pack.ps1   # dist\dummy\GloriaSetupDummy_{0.1.0,0.2.0}_x64-setup.exe
powershell -ExecutionPolicy Bypass -File sandbox\run.ps1           # Windows Sandbox, dist mapped read-only
```

- Run setups with flags from PowerShell (`Start-Process x.exe -ArgumentList '/S' -Wait -PassThru`). Git Bash/MSYS
  rewrites `/S` into a path and the setup exits 6.
- Install folders under `%TEMP%`, Program Files, Windows etc. are refused on purpose (exit 6). Tests use
  `%LOCALAPPDATA%\GloriaSetupTests`.
- Every real run logs to `%TEMP%\GloriaSetup-<id>-<timestamp>.log`. `--dry-run` logs every action and changes nothing.

## Layout

- `crates/gloria-core`: no UI. Manifest schema + validation, payload format, SemVer compare, NSIS-style CLI parser,
  install-manifest model, commit journal + crash recovery (`journal.rs`), path/data-folder guardrails, registry
  value model, exit codes.
- `crates/gloria-pack`: stub + app folder + `gloria.toml` → `<Name>_<version>_x64-setup.exe` (also builds the
  stripped `uninstall.exe` and puts it in the archive). `tests/e2e.rs` drives real installs.
- `crates/gloria-stub`: the setup/uninstaller exe. `window.rs` (Win32 loop) → `app.rs` (state machine) →
  `ui.rs` (immediate-mode Direct2D drawing via `gfx.rs`). `real.rs` wires the UI and silent mode to `engine.rs`
  (install/uninstall/trampoline). `shot.rs` renders PNGs. Strings in `i18n/*.toml` (all four must have every key).
- `action.yml`: composite action for app repos (tag → download release binaries, else build from source).
  `.github/workflows/ci.yml` (build, tests incl. signing, stub size, action on the dummy), `release.yml` (`v*` tag).
  `tools/make-test-cert.ps1`: throwaway self-signed code-signing PFX for tests. `tools/make-social-preview.ps1`
  renders `assets/social-preview.png` (upload it by hand in the repo settings).
- `examples/dummy`: test app + two icons + `pack.ps1`. Test modes: `--hold` (window, closes when asked),
  `--stubborn` (ignores close, must be force-closed), `--quiet` / `GLORIA_DUMMY_QUIET=1`, `GLORIA_DUMMY_DATA=<dir>`,
  fake old uninstaller (`/S` + `GLORIA_DUMMY_LEGACY_KEY=<HKCU key>`, optional `GLORIA_DUMMY_LEGACY_WIPE=<dir>`),
  fake WebView2 bootstrapper (`/install` + `GLORIA_TEST_WEBVIEW2`; `GLORIA_DUMMY_WEBVIEW2_FAIL=1` makes it fail).
  `sandbox/`: Windows Sandbox config.

## GloriaLauncher contract (do not change values)

GloriaLauncher (sibling repo, read only, never edit it) finds apps through
`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\projectGloria.<id>`:
`DisplayName` = exact app name (nothing appended), `DisplayVersion` without `v`, `InstallLocation` unquoted,
`DisplayIcon` = bare full path to the main exe (no quotes, no `,0`), `UninstallString` = `"<dir>\uninstall.exe"`,
`QuietUninstallString` = that + ` /S`, `Publisher` = `projectGloria` (not configurable), `EstimatedSize` DWORD KB,
`NoModify`/`NoRepair` = 1, `GloriaSetup` = stub version. Only HKCU; never write HKLM.

## Icon rules

- **Per-app icon:** a packed setup exe, its window, its taskbar entry and its `uninstall.exe` always carry the app's
  own icon. `gloria-pack` deletes **every** `RT_GROUP_ICON` and `RT_ICON` (all languages) before writing the app's.
  A test checks no GloriaSetup icon image survives.
- **GloriaSetup brand icon** (`assets/brand/GloriaSetup.ico`) appears only on the bare stub, previews,
  `gloria-pack.exe`, and `--allow-default-icon` test builds (which show a TEST BUILD badge). Never on a real
  installer. The small credit image on the done screens is embedded data, not an icon resource, and is fine.
- **Footer link:** every page's footer starts with `assets/brand/GloriaRed.ico` (embedded data, copied from
  GloriaLauncher) + `github.com/ProjectGloria`, which opens https://github.com/ProjectGloria (`Act::OpenGithub`).
  Left footer buttons follow it; the done screens' credit sits just above the footer.
- `assets/brand/*` stay byte-for-byte unchanged. The PNG is for docs only, never a source for icons.

## Payload and signing order

PE + payload appended after the last section: 64-byte header (`GLSETUP\0`, version, flags, offsets, SHA-256) +
manifest JSON + tar/zstd archive + padding to 8. Read forward from the section end, never from EOF, and bounded by the
security directory, so Authenticode signing doesn't break it. The uninstaller payload has `archive_len == 0`.
**Order is fixed:** patch resources (icon purge, icon, version info) → append payload → sign (optional, last).
`UpdateResource` drops overlays and anything after signing breaks the signature. Signing lives in `gloria-pack`
(`--sign-pfx`, password only via `GLORIA_SIGN_PASSWORD`): `uninstall.exe` is signed before archiving, the setup last.
The signing e2e test runs only with `GLORIA_TEST_SIGN_PFX` set (see `tools/make-test-cert.ps1`).

## User data rules

- Never delete user data unless the user ticks "Also remove my settings and data" (off by default) or passes
  `/PURGE`. Updates and reinstalls never touch it.
- `data_folders` must be `%APPDATA%\`, `%LOCALAPPDATA%\` or `%USERPROFILE%\` + a concrete subfolder, not a shared
  folder (deny-list in `gloria-core::paths`), not overlapping the install dir. Deletion never follows reparse points.
- Uninstall removes only files listed in `<dir>\install-manifest.json`; anything else in the folder stays.

## Behaviour notes

- Uninstall trampoline: `uninstall.exe` copies itself to `%TEMP%\GloriaSetup-<id>-<rand>.exe`, starts it with
  `--cleanup "<dir>" --wait-pid <pid>` (+ `/S`, `/PURGE`, `--dry-run`, `--lang`), exits 0. The copy deletes itself
  with a hidden `cmd ping & del` (MoveFileEx delay-until-reboot needs admin).
- Exit codes: 0 ok, 1 error, 2 downgrade refused, 3 files locked / app won't close, 4 UAC (legacy migration),
  5 WebView2, 6 bad args or path.
- Installs stage into `<dir>\.gloria\staging`, write `.gloria\journal`, rename-commit with backups in
  `.gloria\backup`, write the registry, then delete the journal (the commit point). Any later setup or uninstall run
  rolls back a journal it finds before doing anything else. `.gloria` without a journal is just deleted.
- Debug stubs only: `GLORIA_TEST_CRASH_AT=<n>` exits 99 at commit step n (renames, then registry, then journal delete);
  `GLORIA_TEST_WEBVIEW2=<file>` makes "WebView2 present" mean "that file exists"; `GLORIA_TEST_NO_HKLM=1` skips HKLM
  when looking for old installs, so tests never migrate a real install on the dev machine.
- Old installs (`gloria-core::legacy`): looked for only when our own `projectGloria.<id>` key is absent, in HKCU and
  HKLM, both views. Migration backs up the data folders to `%TEMP%\GloriaSetup-backup-<id>` (unless purging), runs
  each old uninstaller (only HKLM ones elevated; UAC declined = exit 4), puts back missing data without overwriting,
  then installs normally. `/D` at the old folder installs in place.
- WebView2: the bootstrapper is packed under `.gloria-webview2\` (`gloria-pack --webview2-bootstrapper`, only with
  `needs_webview2`), staged, run with `/silent /install` only when the runtime is missing, never installed. Never
  download it in a session; the user supplies it.
- Running apps are found by image path, compared with both the given folder and its resolved final path (children of
  packaged apps such as this desktop app see a redirected `%LOCALAPPDATA%`). Silent mode force-closes after a grace.

## Conventions

- Rust 1.80+, x64, `windows` crate 0.62. Strictly per-user; nothing may need admin (except running an old all-users
  uninstaller during migration, after asking).
- Commit messages: one plain sentence saying what changed and why. No `feat:`/`fix:` prefixes.
- Examples and docs use `C:\Users\you`, never a real user name.
