# AGENTS.md — GloriaSetup

Per-user (no admin) Windows installer for projectGloria apps: Rust Win32/Direct2D stub (`gloria-stub`), packager CLI (`gloria-pack`), shared core (`gloria-core`), and GitHub Action (`action.yml`).

## Commands
- Build (debug, warning-free): `cargo build --workspace` (CI sets `RUSTFLAGS=-D warnings`)
- Test (all, unit + HKCU e2e): `cargo test --workspace`
- Test (single): `cargo test --workspace <test_name>`
- Build (release, stub < 5 MB): `cargo build --release -p gloria-stub -p gloria-pack`
- Preview / Screenshots: `target\release\gloria-stub.exe --preview [--page <id>] [--lang en|fr|es|tr] [--manifest gloria.toml] [--icon x.ico]` and `--screenshot out\shots`
- Dummy pack / Sandbox: `powershell -ExecutionPolicy Bypass -File examples\dummy\pack.ps1` and `powershell -ExecutionPolicy Bypass -File sandbox\run.ps1`

## Architecture
- `crates/gloria-core/` — No UI. Manifest schema/validation, payload format, SemVer compare, NSIS CLI parser, install-manifest, commit journal + recovery (`journal.rs`), legacy lookup (`legacy.rs`), path/data-folder guardrails (`paths.rs`), registry model, exit codes.
- `crates/gloria-pack/` — Packs stub + app folder + `gloria.toml` into `<Name>_<version>_x64-setup.exe`, builds stripped `uninstall.exe`, handles icon/resource patching and Authenticode signing. `tests/e2e.rs` drives real installs.
- `crates/gloria-stub/` — Win32 loop (`window.rs`) -> state machine (`app.rs`) -> Direct2D UI (`ui.rs`, `gfx.rs`); `real.rs` + `engine.rs` run install/uninstall/trampoline; `shot.rs` renders PNGs; `i18n/{en,es,fr,tr}.toml` (all four must have every key).
- `action.yml` & `.github/workflows/{ci,release}.yml` — Composite GitHub Action for app repos, CI (build, signing tests, stub size, dummy action), and `v*` release workflow.
- `examples/dummy/`, `sandbox/`, `tools/`, `docs/design.md`, `design/mockups/index.html` — Test app + icons, Windows Sandbox config, test-cert & social-preview scripts, full design doc, and approved UI mockups.
- **Version Manifests:** `Cargo.toml` (`[workspace.package].version`, inherited by all workspace crates), `npm/package.json`.

## Personal Rules (Brain)
- **Git Branching:** Never create new branches or git worktrees. Always work directly on `main` until explicitly told otherwise.
- **Versioning (Minimal Changes):** When runtime code or packaging changes, bump the alpha number by `+1` (e.g., `0.1.0-alpha.1` -> `0.1.0-alpha.2`) across all version manifests in the repo **at most once per uncommitted change set** (check `git diff HEAD` first—if the working tree version is already ahead of `HEAD`, do not bump again until committed). Skip version bumps on docs-only or read-only tasks.
- **Versioning (Major/Minor Changes):** Before changing minor/major versions or dropping `-alpha` on breaking or major feature work, stop and ask for the exact target version.
- **Commit Format:** Write one plain sentence stating what changed and why; do not use `feat:`/`fix:`/`chore:` conventional-commit prefixes.
- **Commit & Push Policy:** Never run `git commit`, `git tag`, or `git push` unless explicitly asked; never force-push to `main`, and push only after the build and tests pass.
- **Path Privacy:** Always use `C:\Users\you` in documentation, comments, and examples—never hardcode the local machine's real username.
- **Window Chrome:** Disable native OS titlebars; build an integrated frameless titlebar with drag region, double-click maximize, and custom min/max/close controls.
- **Desktop Shell Lock:** Lock the app shell to `100vw x 100vh` (`overflow: hidden`) with internal scroll panes and `user-select: none` on UI chrome.
- **Scrollbars:** Ban raw OS scrollbars; use thin (4–6px) themed scrollbars with transparent tracks and zero layout shift.
- **Interactive Controls:** Custom-style all form controls; define explicit `hover`, `active`, `focus-visible`, and `disabled` states with snappy 120–180ms transitions.

## Playbooks (Read Before Task)
- **Building & Packaging:** Read `.brain/building-the-project.md` before packaging, compiling release binaries, or building an installer.
- **GitHub Actions & Releases:** Read `.brain/github-actions.md` before creating or modifying `.github/workflows/*.yml` or release automation.

## Repo Guardrails & Gotchas
- **Platform & Privilege:** Rust 1.80+, x64, `windows` crate 0.62; strictly per-user (never write HKLM or require admin, except elevating an old all-users uninstaller during legacy migration after asking).
- **GloriaLauncher Contract:** Never edit sibling `GloriaLauncher` repo; write `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\projectGloria.<id>` with exact `DisplayName` (app name only), `DisplayVersion` (no `v`), unquoted `InstallLocation`, bare `DisplayIcon` (no quotes or `,0`), `UninstallString` (`"<dir>\uninstall.exe"`), `QuietUninstallString` (`+ /S`), `Publisher = projectGloria`, `EstimatedSize` DWORD KB, `NoModify`/`NoRepair = 1`, `GloriaSetup` = stub version.
- **Icon Rules:** `gloria-pack` deletes every `RT_GROUP_ICON`/`RT_ICON` (all languages) before embedding the app's icon; `assets/brand/GloriaSetup.ico` appears only on bare stub, previews, `gloria-pack.exe`, and `--allow-default-icon` (TEST BUILD badge); `assets/brand/*` stay byte-for-byte unchanged (`GloriaSetup.png` is docs-only).
- **Footer & Credit:** Every page footer starts with embedded `assets/brand/GloriaRed.ico` + `github.com/ProjectGloria` (`Act::OpenGithub`); left footer buttons follow it; done-screen credit sits just above the footer.
- **Payload & Signing Order:** PE + 64-byte `GLSETUP\0` header + manifest JSON + tar/zstd padded to 8, read forward from section end bounded by security directory (`uninstall.exe` has `archive_len == 0`). Fixed order: patch resources -> append payload -> sign (`--sign-pfx`, password via `GLORIA_SIGN_PASSWORD` only; `uninstall.exe` signed before archiving, setup last; signing e2e needs `GLORIA_TEST_SIGN_PFX`).
- **User Data & Uninstall Safety:** Never delete user data unless `/PURGE`/`/WIPE` or user ticks an off-by-default checkbox (updates/reinstalls never touch it); `data_folders` must be `%APPDATA%\`, `%LOCALAPPDATA%\`, or `%USERPROFILE%\` + concrete non-shared subfolder not overlapping install dir; never follow reparse points; uninstall removes only files in `<dir>\install-manifest.json`, except "remove everything" (`/WIPE`), which empties the install folder only if setup created it and clears `%TEMP%` leftovers matched by `paths::is_temp_leftover`. Default dir is `%LOCALAPPDATA%\Programs\Project Gloria Softwares\<Name>`; uninstall removes that parent once empty.
- **Install Paths & CLI:** Refuse `%TEMP%`, Program Files, Windows, etc. (exit 6; tests use `%LOCALAPPDATA%\GloriaSetupTests`). Run setups with flags from PowerShell (`Start-Process x.exe -ArgumentList '/S' -Wait -PassThru`)—Git Bash/MSYS rewrites `/S` and exits 6. Real runs log to `%TEMP%\GloriaSetup-<id>-<timestamp>.log`; `--dry-run` logs without mutating.
- **Commit Journal & Trampoline:** Stage into `<dir>\.gloria\staging`, write `.gloria\journal`, rename-commit with `.gloria\backup`, write registry, then delete journal; later runs roll back any found journal (`.gloria` without journal is deleted). `uninstall.exe` copies to `%TEMP%\GloriaSetup-<id>-<rand>.exe`, runs `--cleanup "<dir>" --wait-pid <pid>`, and self-deletes via hidden `cmd ping & del`.
- **Exit Codes & Process Closing:** `0` ok, `1` error, `2` downgrade refused, `3` locked/won't close, `4` UAC declined, `5` WebView2, `6` bad args/path. Match running apps by both given and resolved final path (packaged-app children see redirected `%LOCALAPPDATA%`); silent mode force-closes after grace.
- **Legacy Migration & WebView2:** Search HKCU+HKLM (both views) for old installs only when `projectGloria.<id>` is absent; back up `data_folders` to `%TEMP%\GloriaSetup-backup-<id>` (unless purging), run old uninstallers (elevate HKLM only), restore missing data without overwriting. Bundle user-supplied WebView2 bootstrapper under `.gloria-webview2\` (`--webview2-bootstrapper` with `needs_webview2`), run `/silent /install` only when missing, never download in-session.
- **Debug Test Hooks:** Debug stubs honor `GLORIA_TEST_CRASH_AT=<n>` (exits 99 at commit step `n`), `GLORIA_TEST_WEBVIEW2=<file>`, and `GLORIA_TEST_NO_HKLM=1` (skips HKLM legacy scan). Dummy app supports `--hold`, `--stubborn`, `--quiet`/`GLORIA_DUMMY_QUIET=1`, `GLORIA_DUMMY_DATA`, `GLORIA_DUMMY_LEGACY_KEY`, `GLORIA_DUMMY_LEGACY_WIPE`, `GLORIA_DUMMY_WEBVIEW2_FAIL=1`.

## Done Criteria
Run `cargo build --workspace` (warning-free), `cargo test --workspace`, and `cargo build --release -p gloria-stub -p gloria-pack` (confirm `target\release\gloria-stub.exe` < 5 MB). Verify `Cargo.toml` workspace version is synced and bumped only when required by the versioning rules.

<!-- brain-sync: 4401790b0df9 -->
