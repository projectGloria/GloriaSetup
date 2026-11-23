//! Thin Win32 helpers for the engine: HKCU registry, shortcuts, processes, launching, self-delete.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gloria_core::legacy;
use gloria_core::paths::{is_within, norm};
use gloria_core::registry::RegValue;
use windows::core::{Interface, GUID, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Programs, IShellLinkW, SHChangeNotify, SHGetKnownFolderPath, ShellExecuteW, ShellLink,
    KF_FLAG_DEFAULT, SHCNE_ASSOCCHANGED, SHCNF_IDLIST,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SW_SHOWNORMAL, WM_CLOSE};


const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ───────────────────────────── registry ─────────────────────────────

fn subkey(key: &str) -> HSTRING {
    HSTRING::from(format!(r"{}\{key}", gloria_core::registry::UNINSTALL_ROOT))
}

/// Writes `HKCU\…\Uninstall\<key>` with exactly `values`.
pub fn reg_write(key: &str, values: &[(&str, RegValue)]) -> Result<(), String> {
    reg_write_at(&subkey(key), values)
}

/// Writes `values` into `HKCU\<full>` (created if missing). A value named `""` is the key's default value.
pub fn reg_write_hkcu(full: &str, values: &[(&str, RegValue)]) -> Result<(), String> {
    reg_write_at(&HSTRING::from(full), values)
}

fn reg_write_at(sk: &HSTRING, values: &[(&str, RegValue)]) -> Result<(), String> {
    unsafe {
        let mut h = HKEY::default();
        let e = RegCreateKeyExW(HKEY_CURRENT_USER, sk, None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut h, None);
        if e != ERROR_SUCCESS {
            return Err(format!("RegCreateKeyExW: {e:?}"));
        }
        let mut res = Ok(());
        for (name, v) in values {
            let (ty, bytes) = match v {
                RegValue::Sz(s) => {
                    let w: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
                    (REG_SZ, w.iter().flat_map(|c| c.to_le_bytes()).collect::<Vec<u8>>())
                }
                RegValue::Dword(d) => (REG_DWORD, d.to_le_bytes().to_vec()),
            };
            let e = RegSetValueExW(h, &HSTRING::from(*name), None, ty, Some(&bytes));
            if e != ERROR_SUCCESS {
                res = Err(format!("RegSetValueExW {name}: {e:?}"));
                break;
            }
        }
        let _ = RegCloseKey(h);
        res
    }
}

/// Reads a REG_SZ from `HKCU\…\Uninstall\<key>`.
pub fn reg_read(key: &str, value: &str) -> Option<String> {
    unsafe {
        let sk = subkey(key);
        let name = HSTRING::from(value);
        let mut len = 0u32;
        if RegGetValueW(HKEY_CURRENT_USER, &sk, &name, RRF_RT_REG_SZ, None, None, Some(&mut len)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
        let mut len = (buf.len() * 2) as u32;
        if RegGetValueW(HKEY_CURRENT_USER, &sk, &name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len))
            != ERROR_SUCCESS
        {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

pub fn reg_delete(key: &str) -> Result<(), String> {
    reg_delete_at(&subkey(key))
}

/// Deletes `HKCU\<full>` and everything under it. A missing key is fine.
pub fn reg_delete_hkcu(full: &str) -> Result<(), String> {
    reg_delete_at(&HSTRING::from(full))
}

fn reg_delete_at(sk: &HSTRING) -> Result<(), String> {
    let e = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, sk) };
    if e == ERROR_SUCCESS || e == ERROR_FILE_NOT_FOUND {
        // RegDeleteTreeW empties the key; remove the key itself too.
        let e2 = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, sk) };
        if e2 == ERROR_SUCCESS || e2 == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        return Err(format!("RegDeleteKeyW: {e2:?}"));
    }
    Err(format!("RegDeleteTreeW: {e:?}"))
}

// ───────────────────────── read-only registry (any hive) ─────────────────────────
// Legacy detection and the WebView2 check only read. Nothing here writes, and nothing anywhere writes HKLM.

fn open_read(root: HKEY, path: &str, view: REG_SAM_FLAGS) -> Option<HKEY> {
    let mut h = HKEY::default();
    let e = unsafe { RegOpenKeyExW(root, &HSTRING::from(path), Some(0), KEY_READ | view, &mut h) };
    (e == ERROR_SUCCESS).then_some(h)
}

/// Every value of an open key: strings as is, DWORDs as decimal. Other types are skipped.
fn read_values(h: HKEY) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for i in 0.. {
        let mut name = vec![0u16; 16384];
        let mut name_len = name.len() as u32;
        let mut ty = 0u32;
        let mut data = vec![0u8; 65536];
        let mut data_len = data.len() as u32;
        let e = unsafe {
            RegEnumValueW(h, i, Some(PWSTR(name.as_mut_ptr())), &mut name_len, None, Some(&mut ty), Some(data.as_mut_ptr()), Some(&mut data_len))
        };
        if e != ERROR_SUCCESS {
            // ERROR_NO_MORE_ITEMS ends the list; ERROR_MORE_DATA (a value over 64 KB) is not something we need.
            if e.0 == 234 {
                continue;
            }
            break;
        }
        let name = String::from_utf16_lossy(&name[..name_len as usize]);
        let data = &data[..data_len as usize];
        let v = match REG_VALUE_TYPE(ty) {
            REG_SZ | REG_EXPAND_SZ => {
                let w: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
                String::from_utf16_lossy(&w[..n])
            }
            REG_DWORD if data.len() >= 4 => u32::from_le_bytes([data[0], data[1], data[2], data[3]]).to_string(),
            _ => continue,
        };
        out.insert(name, v);
    }
    out
}

fn subkeys(h: HKEY) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0.. {
        let mut name = vec![0u16; 256];
        let mut len = name.len() as u32;
        let e = unsafe { RegEnumKeyExW(h, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) };
        if e != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    out
}

fn hive_root(h: legacy::Hive) -> HKEY {
    match h {
        legacy::Hive::User => HKEY_CURRENT_USER,
        legacy::Hive::Machine => HKEY_LOCAL_MACHINE,
    }
}

fn view_flag(v: legacy::View) -> REG_SAM_FLAGS {
    match v {
        legacy::View::Native => KEY_WOW64_64KEY,
        legacy::View::Wow32 => KEY_WOW64_32KEY,
    }
}

/// Every Uninstall entry in HKCU and HKLM, both registry views. Read only.
pub fn uninstall_entries() -> Vec<legacy::Entry> {
    use legacy::{Hive, View};
    let mut out = Vec::new();
    let mut hives = vec![Hive::User];
    // Debug builds: tests can keep the machine's own HKLM entries out of the picture.
    if !(cfg!(debug_assertions) && std::env::var_os("GLORIA_TEST_NO_HKLM").is_some()) {
        hives.push(Hive::Machine);
    }
    for hive in hives {
        for view in [View::Native, View::Wow32] {
            let Some(root) = open_read(hive_root(hive), gloria_core::registry::UNINSTALL_ROOT, view_flag(view)) else { continue };
            for key in subkeys(root) {
                if let Some(h) = open_read(root, &key, view_flag(view)) {
                    out.push(legacy::Entry { hive, view, key, values: read_values(h) });
                    let _ = unsafe { RegCloseKey(h) };
                }
            }
            let _ = unsafe { RegCloseKey(root) };
        }
    }
    out
}

/// Is this Uninstall entry still there?
pub fn uninstall_entry_exists(e: &legacy::Entry) -> bool {
    let path = format!(r"{}\{}", gloria_core::registry::UNINSTALL_ROOT, e.key);
    match open_read(hive_root(e.hive), &path, view_flag(e.view)) {
        Some(h) => {
            let _ = unsafe { RegCloseKey(h) };
            true
        }
        None => false,
    }
}

const WEBVIEW2_CLIENT: &str = r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// Is the WebView2 Evergreen Runtime installed (machine-wide or for this user)? `pv` set and not `0.0.0.0`.
pub fn webview2_present() -> bool {
    // Debug builds: `GLORIA_TEST_WEBVIEW2=<file>` makes "present" mean "that file exists".
    #[cfg(debug_assertions)]
    if let Some(f) = std::env::var_os("GLORIA_TEST_WEBVIEW2") {
        return Path::new(&f).exists();
    }
    let probe = |root: HKEY, path: String, view: REG_SAM_FLAGS| -> bool {
        let Some(h) = open_read(root, &path, view) else { return false };
        let v = read_values(h);
        let _ = unsafe { RegCloseKey(h) };
        v.iter().any(|(k, v)| k.eq_ignore_ascii_case("pv") && !v.is_empty() && v != "0.0.0.0")
    };
    let r = probe(HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\{WEBVIEW2_CLIENT}"), KEY_WOW64_32KEY)
        || probe(HKEY_CURRENT_USER, format!(r"Software\{WEBVIEW2_CLIENT}"), REG_SAM_FLAGS(0));
    log!("WebView2 runtime present: {r}");
    r
}

// ───────────────────────────── running other installers ─────────────────────────────

pub enum RunError {
    /// The user said no to the UAC prompt.
    Cancelled,
    Other(String),
}

/// Runs `exe args` and waits for it. With `elevate`, Windows asks for admin first (only the old all-users
/// uninstaller ever runs that way). Returns the exit code, or `None` if it was still running after `timeout`.
pub fn run_wait(exe: &Path, args: &str, elevate: bool, timeout: Duration) -> Result<Option<u32>, RunError> {
    use windows::Win32::System::Threading::GetExitCodeProcess;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    if !elevate {
        let mut child = std::process::Command::new(exe).raw_arg(args).creation_flags(CREATE_NO_WINDOW).spawn().map_err(|e| RunError::Other(e.to_string()))?;
        let end = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(s)) => return Ok(Some(s.code().unwrap_or(1) as u32)),
                Ok(None) if Instant::now() >= end => return Ok(None),
                Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                Err(e) => return Err(RunError::Other(e.to_string())),
            }
        }
    }
    let verb = HSTRING::from("runas");
    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        // ERROR_CANCELLED: the UAC prompt was declined.
        if e.code() == windows::Win32::Foundation::ERROR_CANCELLED.to_hresult() {
            return Err(RunError::Cancelled);
        }
        return Err(RunError::Other(e.to_string()));
    }
    let h = info.hProcess;
    if h.is_invalid() {
        return Ok(Some(0));
    }
    let r = unsafe { WaitForSingleObject(h, timeout.as_millis().min(u32::MAX as u128 - 1) as u32) };
    let mut code = 1u32;
    let done = r == WAIT_OBJECT_0 && unsafe { GetExitCodeProcess(h, &mut code) }.is_ok();
    let _ = unsafe { CloseHandle(h) };
    Ok(done.then_some(code))
}

/// `%SystemRoot%\System32\<exe>` for a bare system tool name such as `msiexec.exe`.
pub fn system_exe(name: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join(name)
}

// ───────────────────────────── shortcuts ─────────────────────────────

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p: PWSTR = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs`.
pub fn start_menu_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_Programs)
}

pub fn desktop_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_Desktop)
}

const PKEY_APP_USER_MODEL_ID: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3), pid: 5 };

/// Initializes COM on the calling thread for as long as it lives (the engine runs on worker threads, where nobody
/// else does); uninitializes only if this call did the initializing.
struct ComScope(bool);

impl ComScope {
    fn enter() -> ComScope {
        // S_OK / S_FALSE need a matching CoUninitialize; RPC_E_CHANGED_MODE means COM is already up in another mode.
        ComScope(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok())
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Writes a .lnk to `target` (its icon comes from the exe), with an optional AppUserModelID. Safe on any thread.
pub fn create_shortcut(lnk: &Path, target: &Path, aumid: Option<&str>, description: &str) -> windows::core::Result<()> {
    let _com = ComScope::enter();
    unsafe {
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        sl.SetPath(&HSTRING::from(target.as_os_str()))?;
        if let Some(dir) = target.parent() {
            sl.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        sl.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)?;
        sl.SetDescription(&HSTRING::from(description))?;
        if let Some(id) = aumid {
            let store: IPropertyStore = sl.cast()?;
            let mut w: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
            // A borrowed VT_LPWSTR: SetValue copies it, and we never call PropVariantClear on it.
            let pv = PROPVARIANT {
                Anonymous: PROPVARIANT_0 {
                    Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                        vt: VT_LPWSTR,
                        wReserved1: 0,
                        wReserved2: 0,
                        wReserved3: 0,
                        Anonymous: PROPVARIANT_0_0_0 { pwszVal: PWSTR(w.as_mut_ptr()) },
                    }),
                },
            };
            store.SetValue(&PKEY_APP_USER_MODEL_ID, &pv)?;
            store.Commit()?;
        }
        if let Some(dir) = lnk.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let pf: IPersistFile = sl.cast()?;
        pf.Save(&HSTRING::from(lnk.as_os_str()), true)?;
        Ok(())
    }
}

pub fn notify_shell() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

// ───────────────────────────── processes ─────────────────────────────

pub fn image_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        ok.ok()?;
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// `dir` as the file system resolves it, without the `\\?\` prefix: junctions, 8.3 names and the AppData
/// redirection Windows applies to children of packaged (MSIX) apps are undone. Process image paths come back in
/// this form, so a plain string compare with the folder we were given can miss a running app.
fn final_path(dir: &Path) -> Option<PathBuf> {
    let p = std::fs::canonicalize(dir).ok()?;
    let s = p.to_string_lossy();
    Some(match s.strip_prefix(r"\\?\UNC\") {
        Some(r) => PathBuf::from(format!(r"\\{r}")),
        None => PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s)),
    })
}

/// Processes whose image is inside `dir` (never this process). `skip` drops extra pids (the uninstaller we wait for).
pub fn processes_in(dir: &Path, skip: Option<u32>) -> Vec<(u32, PathBuf)> {
    let me = unsafe { GetCurrentProcessId() };
    let dirs: Vec<PathBuf> = std::iter::once(dir.to_path_buf()).chain(final_path(dir)).collect();
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let pid = e.th32ProcessID;
            if pid != me && Some(pid) != skip && pid != 0 {
                if let Some(p) = image_path(pid) {
                    if dirs.iter().any(|d| is_within(&p, d) && norm(&p) != norm(d)) {
                        out.push((pid, p));
                    }
                }
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Posts WM_CLOSE to every visible top-level window of `pids`.
pub fn ask_close(pids: &[u32]) {
    struct Ctx<'a>(&'a [u32]);
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> windows_core::BOOL {
        let ctx = &*(l.0 as *const Ctx);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if ctx.0.contains(&pid) && IsWindowVisible(h).as_bool() {
            let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        windows_core::BOOL(1)
    }
    let ctx = Ctx(pids);
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&ctx as *const Ctx as isize));
    }
}

pub fn terminate(pid: u32) {
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) {
            let _ = TerminateProcess(h, 1);
            let _ = WaitForSingleObject(h, 3000);
            let _ = CloseHandle(h);
        }
    }
}

/// Waits for a process to exit. True if it has (or was never there).
pub fn wait_pid(pid: u32, timeout: Duration) -> bool {
    unsafe {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(h) => {
                let r = WaitForSingleObject(h, timeout.as_millis() as u32);
                let _ = CloseHandle(h);
                r == WAIT_OBJECT_0
            }
            Err(_) => true,
        }
    }
}

/// Asks the app's processes in `dir` to close, waits up to `grace`, then force-closes what is left if `force`.
/// Returns the processes still running.
pub fn close_apps(dir: &Path, skip: Option<u32>, grace: Duration, force: bool) -> Vec<(u32, PathBuf)> {
    let procs = processes_in(dir, skip);
    if procs.is_empty() {
        return procs;
    }
    for (pid, p) in &procs {
        log!("running: {pid} {}", p.display());
    }
    ask_close(&procs.iter().map(|p| p.0).collect::<Vec<_>>());
    let end = Instant::now() + grace;
    let mut left = procs;
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(200));
        left = processes_in(dir, skip);
        if left.is_empty() {
            return left;
        }
    }
    if force {
        for (pid, _) in &left {
            log!("force-closing {pid}");
            terminate(*pid);
        }
        std::thread::sleep(Duration::from_millis(300));
        left = processes_in(dir, skip);
    }
    left
}

// ───────────────────────────── launching ─────────────────────────────

/// Starts the app the way Explorer would (not elevated, its own folder as working directory).
pub fn launch(exe: &Path) -> bool {
    let dir = exe.parent().map(|d| HSTRING::from(d.as_os_str())).unwrap_or_default();
    let r = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(exe.as_os_str()), PCWSTR::null(), &dir, SW_SHOWNORMAL) };
    r.0 as isize > 32
}

pub fn open_file(p: &Path) {
    let _ = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(p.as_os_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
}

/// Opens a web page in the default browser.
pub fn open_url(url: &str) {
    let _ = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
}

/// Starts `exe` with a raw argument string (our own NSIS-style parser reads it back).
pub fn spawn(exe: &Path, args: &str) -> std::io::Result<u32> {
    let child = std::process::Command::new(exe).raw_arg(args).spawn()?;
    Ok(child.id())
}

/// Deletes this (temporary) exe after it exits, and `also` (e.g. this run's log): a hidden `cmd` waits a moment,
/// then removes them.
pub fn delete_self_later(also: Option<&Path>) {
    let Ok(me) = std::env::current_exe() else { return };
    let mut cmd = format!(r#"/d /c ping 127.0.0.1 -n 3 >nul & del /f /q "{}""#, me.display());
    if let Some(p) = also {
        cmd += &format!(r#" & del /f /q "{}" 2>nul"#, p.display());
    }
    let _ = std::process::Command::new("cmd.exe").raw_arg(cmd).creation_flags(CREATE_NO_WINDOW).spawn();
}

pub fn current_pid() -> u32 {
    unsafe { GetCurrentProcessId() }
}

/// A random-enough suffix for temp names.
pub fn rand_suffix() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{:08x}", (t.as_nanos() as u64 ^ (current_pid() as u64).rotate_left(17)) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_and_closes_processes_in_a_folder() {
        let dir = std::env::temp_dir().join(format!("gloria-sys-procs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("pinger.exe");
        let sys32 = std::env::var("SystemRoot").unwrap() + r"\System32\PING.EXE";
        std::fs::copy(sys32, &exe).unwrap();
        let mut child = std::process::Command::new(&exe).args(["-n", "60", "127.0.0.1"]).creation_flags(CREATE_NO_WINDOW).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        eprintln!("image of child: {:?}", image_path(child.id()));
        let found = processes_in(&dir, None);
        assert!(found.iter().any(|(pid, _)| *pid == child.id()), "{found:?}");
        // No window to ask: force-closing ends it.
        let left = close_apps(&dir, None, Duration::from_millis(400), true);
        assert!(left.is_empty(), "{left:?}");
        assert!(child.wait().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
