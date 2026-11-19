//! Intégration Windows : presse-papiers de fichiers (compatible Explorateur),
//! boîte « Propriétés », « Ouvrir avec », touches.
use std::path::{Path, PathBuf};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::ptr::{copy_nonoverlapping, null_mut};
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::*;
    use windows_sys::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};
    use windows_sys::Win32::UI::Shell::*;

    unsafe fn put(fmt: u32, bytes: &[u8]) {
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len());
            if h.is_null() {
                return;
            }
            let p = GlobalLock(h) as *mut u8;
            if !p.is_null() {
                copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
                GlobalUnlock(h);
                SetClipboardData(fmt, h);
            }
        }
    }

    /// Met des fichiers dans le presse-papiers comme l'Explorateur (CF_HDROP),
    /// plus leur chemin en texte.
    pub fn set_files(paths: &[PathBuf], cut: bool) -> bool {
        unsafe {
            if OpenClipboard(null_mut()) == 0 {
                return false;
            }
            EmptyClipboard();
            let mut list: Vec<u16> = Vec::new();
            for p in paths {
                list.extend(p.as_os_str().encode_wide());
                list.push(0);
            }
            list.push(0);
            let header = std::mem::size_of::<DROPFILES>();
            let mut buf = vec![0u8; header + list.len() * 2];
            let df = DROPFILES { pFiles: header as u32, pt: std::mem::zeroed(), fNC: 0, fWide: 1 };
            copy_nonoverlapping(&df as *const DROPFILES as *const u8, buf.as_mut_ptr(), header);
            copy_nonoverlapping(list.as_ptr() as *const u8, buf.as_mut_ptr().add(header), list.len() * 2);
            put(CF_HDROP as u32, &buf);

            let effect: u32 = if cut { 2 } else { 5 }; // DROPEFFECT_MOVE / COPY|LINK
            let fmt = RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr());
            put(fmt, &effect.to_le_bytes());

            let text: Vec<u16> = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("\r\n")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let bytes: Vec<u8> = text.iter().flat_map(|c| c.to_le_bytes()).collect();
            put(CF_UNICODETEXT as u32, &bytes);
            CloseClipboard();
            true
        }
    }

    /// Lit les fichiers du presse-papiers (copiés ici ou dans l'Explorateur).
    pub fn get_files() -> Option<(Vec<PathBuf>, bool)> {
        unsafe {
            if OpenClipboard(null_mut()) == 0 {
                return None;
            }
            let h = GetClipboardData(CF_HDROP as u32);
            if h.is_null() {
                CloseClipboard();
                return None;
            }
            let n = DragQueryFileW(h as HDROP, u32::MAX, null_mut(), 0);
            let mut out = Vec::new();
            for i in 0..n {
                let len = DragQueryFileW(h as HDROP, i, null_mut(), 0) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(h as HDROP, i, buf.as_mut_ptr(), (len + 1) as u32);
                out.push(PathBuf::from(OsString::from_wide(&buf[..len])));
            }
            let fmt = RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr());
            let hd = GetClipboardData(fmt);
            let mut cut = false;
            if !hd.is_null() {
                let p = GlobalLock(hd) as *const u32;
                if !p.is_null() {
                    cut = (*p & 2) != 0;
                    GlobalUnlock(hd);
                }
            }
            CloseClipboard();
            Some((out, cut))
        }
    }

    pub fn properties(path: &Path) {
        unsafe {
            let file: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            let verb = wide("properties");
            let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            info.fMask = SEE_MASK_INVOKEIDLIST;
            info.lpVerb = verb.as_ptr();
            info.lpFile = file.as_ptr();
            info.nShow = 5; // SW_SHOW
            ShellExecuteExW(&mut info);
        }
    }

    pub fn key_down(vk: i32) -> bool {
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk) < 0 }
    }
}

#[cfg(windows)]
pub use imp::*;

// ---------- Explorateur par défaut ----------
//
// Tout se passe dans HKEY_CURRENT_USER (rien pour les autres comptes, pas besoin d'admin) :
// - Directory\shell et Drive\shell : un verbe « explorateur » par défaut → ouvrir un dossier
//   ou un disque depuis le Bureau, un raccourci, une autre appli ouvre cette app ;
// - le CLSID de Win+E → Win+E ouvre cette app.
// Désactiver supprime exactement ces clés : l'Explorateur Windows reprend la main.
#[cfg(windows)]
pub mod default_explorer {
    use super::wide;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW, RRF_RT_REG_SZ,
    };

    const WIN_E: &str = r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";
    const VERB: &str = "explorateur";

    fn set(sub: &str, name: Option<&str>, value: &str) -> bool {
        let sub = wide(sub);
        let name_w = name.map(wide);
        let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                name_w.as_ref().map(|n| n.as_ptr()).unwrap_or(std::ptr::null()),
                REG_SZ,
                data.as_ptr() as *const _,
                (data.len() * 2) as u32,
            ) == 0
        }
    }

    fn get(sub: &str) -> Option<String> {
        let sub = wide(sub);
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, sub.as_ptr(), std::ptr::null(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut len)
        };
        if r != 0 {
            return None;
        }
        let n = (len as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..n]))
    }

    pub fn is_enabled() -> bool {
        get(&format!(r"Software\Classes\Directory\shell\{VERB}\command")).is_some()
    }

    pub fn enable() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.to_string_lossy();
        let open = format!("\"{exe}\" \"%1\"");
        let mut ok = true;
        for class in ["Directory", "Drive"] {
            ok &= set(&format!(r"Software\Classes\{class}\shell\{VERB}"), None, "Ouvrir dans Explorateur");
            ok &= set(&format!(r"Software\Classes\{class}\shell\{VERB}\command"), None, &open);
            ok &= set(&format!(r"Software\Classes\{class}\shell"), None, VERB);
        }
        ok &= set(&format!(r"{WIN_E}\shell\opennewwindow\command"), None, &format!("\"{exe}\""));
        ok &= set(&format!(r"{WIN_E}\shell\opennewwindow\command"), Some("DelegateExecute"), "");
        notify_shell();
        if ok { Ok(()) } else { Err("écriture du registre refusée".into()) }
    }

    pub fn disable() {
        unsafe {
            for class in ["Directory", "Drive"] {
                let verb = wide(&format!(r"Software\Classes\{class}\shell\{VERB}"));
                RegDeleteTreeW(HKEY_CURRENT_USER, verb.as_ptr());
                // On ne retire la valeur par défaut que si c'est la nôtre.
                if get(&format!(r"Software\Classes\{class}\shell")).as_deref() == Some(VERB) {
                    let shell = wide(&format!(r"Software\Classes\{class}\shell"));
                    RegDeleteKeyValueW(HKEY_CURRENT_USER, shell.as_ptr(), std::ptr::null());
                }
            }
            let k = wide(WIN_E);
            RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr());
        }
        notify_shell();
    }

    fn notify_shell() {
        use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
    }
}

// ---------- Une seule fenêtre ----------
//
// Si l'app tourne déjà, une nouvelle instance dépose le dossier demandé dans une « boîte aux
// lettres » (%APPDATA%\Explorateur\inbox) et se ferme ; la fenêtre ouverte l'ouvre dans un onglet.
pub mod single {
    use std::path::PathBuf;

    pub fn inbox() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("Explorateur").join("inbox"))
    }

    /// `true` si une autre instance tourne déjà (mutex nommé, gardé jusqu'à la fin du processus).
    #[cfg(windows)]
    pub fn another_running() -> bool {
        use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let name = super::wide("Local\\Explorateur.SingleInstance");
        unsafe {
            let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            !h.is_null() && GetLastError() == ERROR_ALREADY_EXISTS
        }
    }

    /// Transmet le dossier à l'instance déjà ouverte et l'autorise à passer au premier plan.
    pub fn forward(path: Option<PathBuf>) -> bool {
        let Some(dir) = inbox() else { return false };
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let tmp = dir.join(format!("{stamp}.tmp"));
        let body = path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        if std::fs::write(&tmp, body).is_err() || std::fs::rename(&tmp, dir.join(format!("{stamp}.req"))).is_err() {
            return false;
        }
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(u32::MAX); // ASFW_ANY
        }
        true
    }

    /// Lit et vide les demandes en attente. Chaîne vide = « nouvel onglet sur le dossier perso ».
    pub fn take_requests() -> Vec<String> {
        let Some(dir) = inbox() else { return vec![] };
        let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
        let mut files: Vec<PathBuf> =
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "req").unwrap_or(false)).collect();
        files.sort();
        files
            .into_iter()
            .filter_map(|f| {
                let s = std::fs::read_to_string(&f).ok();
                let _ = std::fs::remove_file(&f);
                s
            })
            .collect()
    }
}

/// Le vrai menu clic droit de Windows (7-Zip, Git, « Envoyer vers », extensions…).
/// Bloquant tant que le menu est ouvert, comme dans l'Explorateur.
#[cfg(windows)]
pub fn native_context_menu(hwnd: isize, paths: &[std::path::PathBuf]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        BHID_SFUIObject, CMF_NORMAL, CMINVOKECOMMANDINFO, IContextMenu, ILFree, SHCreateShellItemArrayFromIDLists,
        SHParseDisplayName,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreatePopupMenu, DestroyMenu, GetCursorPos, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
    };
    use windows::core::PCWSTR;
    if paths.is_empty() {
        return Ok(());
    }
    unsafe {
        let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
        for p in paths {
            let w: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            if SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
                pidls.push(pidl);
            }
        }
        let result = (|| -> windows::core::Result<()> {
            let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
            let arr = SHCreateShellItemArrayFromIDLists(&consts)?;
            let cm: IContextMenu = arr.BindToHandler(None, &BHID_SFUIObject)?;
            let menu = CreatePopupMenu()?;
            cm.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL)?;
            let mut pt = POINT::default();
            GetCursorPos(&mut pt)?;
            let hwnd = HWND(hwnd as *mut _);
            let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None);
            if cmd.0 > 0 {
                let info = CMINVOKECOMMANDINFO {
                    cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
                    hwnd,
                    lpVerb: windows::core::PCSTR((cmd.0 - 1) as usize as *const u8),
                    nShow: 1, // SW_SHOWNORMAL
                    ..Default::default()
                };
                let _ = cm.InvokeCommand(&info);
            }
            let _ = DestroyMenu(menu);
            Ok(())
        })();
        for p in pidls {
            ILFree(Some(p as *const _));
        }
        result.map_err(|e| e.message().to_string())
    }
}

/// Ouvre une nouvelle session Claude Code dans l'app Claude de bureau, sur ce dossier
/// (lien `claude://code/new?folder=…` que l'app enregistre sous Windows).
/// Si l'app n'est pas installée, on se rabat sur `claude` dans un terminal.
pub fn open_claude_code(dir: &Path) -> bool {
    let url = format!("claude://code/new?folder={}", percent_encode(&dir.to_string_lossy()));
    if open::that_detached(&url).is_ok() {
        return true;
    }
    let wt = std::process::Command::new("wt.exe")
        .arg("-d")
        .arg(dir)
        .args(["powershell.exe", "-NoExit", "-Command", "claude"])
        .spawn();
    if wt.is_err() {
        let _ = std::process::Command::new("powershell.exe").args(["-NoExit", "-Command", "claude"]).current_dir(dir).spawn();
    }
    false
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn hidden_cmd(prog: &str) -> std::process::Command {
    let mut c = std::process::Command::new(prog);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

pub fn open_with(path: &Path) {
    let _ = std::process::Command::new("rundll32.exe")
        .arg("shell32.dll,OpenAs_RunDLL")
        .arg(path)
        .spawn();
}

pub fn reveal_in_explorer(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn();
}

pub fn open_terminal(dir: &Path) {
    // Windows Terminal si présent, sinon PowerShell.
    let wt = std::process::Command::new("wt.exe").arg("-d").arg(dir).spawn();
    if wt.is_err() {
        let _ = std::process::Command::new("powershell.exe")
            .arg("-NoExit")
            .current_dir(dir)
            .spawn();
    }
}

pub fn open_in_code(path: &Path) -> bool {
    hidden_cmd("cmd").args(["/C", "code"]).arg(path).spawn().is_ok()
}
