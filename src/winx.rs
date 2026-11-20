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
/// Lancé depuis une app empaquetée (MSIX, p. ex. l'app Claude) ? Ses nouveaux fichiers d'AppData
/// (et ses écritures dans HKCU) partent alors dans le conteneur de cette app, invisibles pour
/// Windows et pour l'utilisateur. Ces processus n'ont pas d'identité de paquet : on le vérifie
/// directement avec un fichier sonde (seulement quand on en a besoin, quelques ms).
pub fn virtualized() -> bool {
    let (Some(roaming), Some(local)) = (dirs::config_dir(), dirs::data_local_dir()) else { return false };
    let name = format!("explorateur-sonde-{}.tmp", std::process::id());
    let probe = roaming.join(&name);
    if std::fs::write(&probe, b"").is_err() {
        return false;
    }
    let found = std::fs::read_dir(local.join("Packages"))
        .map(|rd| rd.flatten().any(|e| e.path().join("LocalCache").join("Roaming").join(&name).exists()))
        .unwrap_or(false);
    let _ = std::fs::remove_file(&probe);
    found
}

pub mod default_explorer {
    //! Explorateur par défaut, **par utilisateur** (HKCU uniquement, réversible) : verbe par défaut
    //! des dossiers et des disques, et Win+E. L'ancienne valeur par défaut est gardée et remise.
    use super::wide;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
    };

    /// Racine réelle ; les tests en utilisent une autre.
    pub const CLASSES: &str = r"Software\Classes";
    const WIN_E: &str = r"CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command";
    const VERB: &str = "explorateur";
    /// Valeurs où l'on garde ce qu'il y avait avant nous.
    const OLD_DEFAULT: &str = "AncienDefaut";
    const OLD_WIN_E: &str = "AncienWinE";

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

    fn get_value(sub: &str, name: Option<&str>) -> Option<String> {
        let sub = wide(sub);
        let name_w = name.map(wide);
        let mut buf = vec![0u16; 2048];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                name_w.as_ref().map(|n| n.as_ptr()).unwrap_or(std::ptr::null()),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut _,
                &mut len,
            )
        };
        if r != 0 {
            return None;
        }
        let n = (len as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..n]))
    }

    fn get(sub: &str) -> Option<String> {
        get_value(sub, None)
    }

    fn delete_value(sub: &str, name: Option<&str>) {
        let sub = wide(sub);
        let name_w = name.map(wide);
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, sub.as_ptr(), name_w.as_ref().map(|n| n.as_ptr()).unwrap_or(std::ptr::null())) };
    }

    fn delete_tree(sub: &str) {
        let sub = wide(sub);
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, sub.as_ptr()) };
    }

    /// Commande d'ouverture. Pour un disque, `%1` vaut « C:\ » : entre guillemets, le « \" » final
    /// serait lu comme un guillemet échappé (l'exe recevrait « C:" »), donc pas de guillemets.
    fn command(exe: &str, class: &str) -> String {
        if class == "Drive" { format!("\"{exe}\" %1") } else { format!("\"{exe}\" \"%1\"") }
    }

    pub fn is_enabled_at(base: &str) -> bool {
        get(&format!(r"{base}\Directory\shell\{VERB}\command")).is_some()
    }

    /// Exe inscrit dans le registre (entre les premiers guillemets de la commande).
    pub fn registered_exe_at(base: &str) -> Option<String> {
        let c = get(&format!(r"{base}\Directory\shell\{VERB}\command"))?;
        c.strip_prefix('"')?.split('"').next().map(String::from)
    }

    pub fn enable_at(base: &str, exe: &str) -> Result<(), String> {
        let mut ok = true;
        for class in ["Directory", "Drive"] {
            let shell = format!(r"{base}\{class}\shell");
            let verb = format!(r"{shell}\{VERB}");
            // Verbe par défaut d'avant (ex. un autre explorateur) : gardé pour le remettre.
            if let Some(old) = get(&shell).filter(|v| !v.is_empty() && v != VERB) {
                ok &= set(&verb, Some(OLD_DEFAULT), &old);
            }
            ok &= set(&verb, None, "Ouvrir dans Explorateur");
            ok &= set(&format!(r"{verb}\command"), None, &command(exe, class));
            ok &= set(&shell, None, VERB);
        }
        let win_e = format!(r"{base}\{WIN_E}");
        let mine = format!("\"{exe}\"");
        if let Some(old) = get(&win_e).filter(|v| !v.is_empty() && !v.contains("Explorateur.exe") && v != &mine) {
            ok &= set(&format!(r"{base}\Directory\shell\{VERB}"), Some(OLD_WIN_E), &old);
        }
        ok &= set(&win_e, None, &mine);
        ok &= set(&win_e, Some("DelegateExecute"), "");
        if ok { Ok(()) } else { Err("écriture du registre refusée".into()) }
    }

    pub fn disable_at(base: &str) {
        let old_win_e = get_value(&format!(r"{base}\Directory\shell\{VERB}"), Some(OLD_WIN_E));
        for class in ["Directory", "Drive"] {
            let shell = format!(r"{base}\{class}\shell");
            let verb = format!(r"{shell}\{VERB}");
            let old = get_value(&verb, Some(OLD_DEFAULT));
            delete_tree(&verb);
            // On ne touche à la valeur par défaut que si c'est la nôtre.
            if get(&shell).as_deref() == Some(VERB) {
                match old {
                    Some(v) => {
                        set(&shell, None, &v);
                    }
                    None => delete_value(&shell, None),
                }
            }
        }
        let win_e = format!(r"{base}\{WIN_E}");
        match old_win_e {
            Some(v) => {
                set(&win_e, None, &v);
            }
            None => delete_tree(&format!(r"{base}\CLSID\{{52205fd8-5dfb-447d-801a-d0b52f2e83e1}}")),
        }
    }

    pub fn is_enabled() -> bool {
        is_enabled_at(CLASSES)
    }

    /// Exe de développement (`target\…`) : jamais inscrit comme explorateur par défaut.
    fn is_dev_build(exe: &std::path::Path) -> bool {
        exe.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target"))
    }

    pub fn enable() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        if is_dev_build(&exe) {
            return Err("version de développement (target\\) : lance l'exe installé pour ce réglage".into());
        }
        enable_at(CLASSES, &exe.to_string_lossy())?;
        notify_shell();
        Ok(())
    }

    pub fn disable() {
        disable_at(CLASSES);
        notify_shell();
    }

    /// Au démarrage : l'exe a changé de place (dossier déplacé, nouvelle version ailleurs) alors que
    /// le réglage est actif → on réinscrit le bon chemin. Rien pour une version de développement
    /// ni depuis un conteneur d'app (registre virtualisé). Renvoie `true` s'il a été réécrit.
    pub fn refresh_if_moved() -> bool {
        let Ok(exe) = std::env::current_exe() else { return false };
        let Some(reg) = registered_exe_at(CLASSES) else { return false };
        if reg.eq_ignore_ascii_case(&exe.to_string_lossy()) || is_dev_build(&exe) || super::virtualized() {
            return false;
        }
        enable().is_ok()
    }

    fn notify_shell() {
        use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
    }
}

/// Le vrai menu clic droit de Windows (7-Zip, Git, « Envoyer vers », extensions…).
/// Chemins → tableau d'éléments du Shell (les PIDL intermédiaires sont libérés ici : le tableau
/// garde ses propres copies). Utilisé par le menu natif et le glisser vers l'extérieur.
#[cfg(windows)]
fn shell_items(paths: &[PathBuf]) -> windows::core::Result<windows::Win32::UI::Shell::IShellItemArray> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{ILFree, SHCreateShellItemArrayFromIDLists, SHParseDisplayName};
    use windows::core::PCWSTR;
    unsafe {
        let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
        for p in paths {
            let w: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            if SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
                pidls.push(pidl);
            }
        }
        let res = if pidls.is_empty() {
            Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_INVALIDARG))
        } else {
            let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
            SHCreateShellItemArrayFromIDLists(&consts)
        };
        for p in pidls {
            ILFree(Some(p as *const _));
        }
        res
    }
}

/// Objet de données du Shell pour ces fichiers (CF_HDROP, noms, PIDL…), exactement celui que
/// l'Explorateur fournit quand on glisse des fichiers. Accepte des fichiers de dossiers différents.
#[cfg(windows)]
pub fn shell_data_object(paths: &[PathBuf]) -> Result<windows::Win32::System::Com::IDataObject, String> {
    use windows::Win32::UI::Shell::BHID_DataObject;
    unsafe { shell_items(paths).and_then(|arr| arr.BindToHandler(None, &BHID_DataObject)).map_err(|e| e.message()) }
}

/// Glisser des fichiers hors de la fenêtre (Bureau, Discord, navigateur…) : glisser-déposer OLE du
/// Shell, avec sa source et son image de glissement par défaut. Bloquant jusqu'au dépôt.
/// Remplace la crate `drag`, qui tirait d'anciennes versions de `windows`.
#[cfg(windows)]
pub fn drag_files(hwnd: isize, paths: &[PathBuf]) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, IDropSource};
    use windows::Win32::UI::Shell::SHDoDragDrop;
    let data = shell_data_object(paths)?;
    unsafe {
        SHDoDragDrop(Some(HWND(hwnd as *mut _)), &data, None::<&IDropSource>, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK)
            .map(|_| ())
            .map_err(|e| e.message())
    }
}

/// Bloquant tant que le menu est ouvert, comme dans l'Explorateur.
#[cfg(windows)]
pub fn native_context_menu(hwnd: isize, paths: &[std::path::PathBuf]) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::UI::Shell::{BHID_SFUIObject, CMF_NORMAL, CMINVOKECOMMANDINFO, IContextMenu};
    use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx};
    if paths.is_empty() {
        return Ok(());
    }
    /// Détruit le menu dans tous les cas, y compris sur un chemin d'erreur.
    struct MenuGuard(HMENU);
    impl Drop for MenuGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyMenu(self.0);
            }
        }
    }
    unsafe {
        let result = (|| -> windows::core::Result<()> {
            let arr = shell_items(paths)?;
            let cm: IContextMenu = arr.BindToHandler(None, &BHID_SFUIObject)?;
            let guard = MenuGuard(CreatePopupMenu()?);
            let menu = guard.0;
            cm.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL).ok()?;
            let mut pt = POINT::default();
            GetCursorPos(&mut pt)?;
            let hwnd = HWND(hwnd as *mut _);
            // Sous-menus dynamiques (« Envoyer vers », « Ouvrir avec », icônes d'extensions) :
            // Windows les remplit via des messages qu'on doit transmettre à IContextMenu2/3
            // pendant que le menu est ouvert. On sous-classe la fenêtre le temps du menu.
            use windows::core::Interface;
            use windows::Win32::UI::Shell::{IContextMenu2, IContextMenu3, RemoveWindowSubclass, SetWindowSubclass};
            MENU_HANDLER.with(|h| {
                *h.borrow_mut() = cm.cast::<IContextMenu3>().map(MenuHandler::V3).or_else(|_| cm.cast::<IContextMenu2>().map(MenuHandler::V2)).ok();
            });
            let subclassed = SetWindowSubclass(hwnd, Some(menu_subclass_proc), 0xE4F1, 0).as_bool();
            let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None);
            if subclassed {
                let _ = RemoveWindowSubclass(hwnd, Some(menu_subclass_proc), 0xE4F1);
            }
            MENU_HANDLER.with(|h| *h.borrow_mut() = None);
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
            Ok(())
        })();
        result.map_err(|e| e.message().to_string())
    }
}

#[cfg(windows)]
enum MenuHandler {
    V3(windows::Win32::UI::Shell::IContextMenu3),
    V2(windows::Win32::UI::Shell::IContextMenu2),
}

#[cfg(windows)]
thread_local! {
    static MENU_HANDLER: std::cell::RefCell<Option<MenuHandler>> = const { std::cell::RefCell::new(None) };
}

/// Transmet au menu Windows les messages qui servent à dessiner et remplir ses sous-menus.
#[cfg(windows)]
unsafe extern "system" fn menu_subclass_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::Shell::DefSubclassProc;
    const WM_DRAWITEM: u32 = 0x002B;
    const WM_MEASUREITEM: u32 = 0x002C;
    const WM_INITMENUPOPUP: u32 = 0x0117;
    const WM_MENUCHAR: u32 = 0x0120;
    if matches!(msg, WM_DRAWITEM | WM_MEASUREITEM | WM_INITMENUPOPUP | WM_MENUCHAR) {
        let handled = MENU_HANDLER.with(|h| {
            let mut result = LRESULT(0);
            let ok = match h.borrow().as_ref() {
                Some(MenuHandler::V3(c)) => unsafe { c.HandleMenuMsg2(msg, wparam, lparam, Some(&mut result)) }.is_ok(),
                Some(MenuHandler::V2(c)) => unsafe { c.HandleMenuMsg(msg, wparam, lparam) }.is_ok(),
                None => false,
            };
            ok.then_some(result)
        });
        if let Some(r) = handled {
            // WM_MENUCHAR renvoie une valeur ; les autres messages : traités = 0 (ou TRUE pour dessin)
            return if msg == WM_MENUCHAR { r } else if matches!(msg, WM_DRAWITEM | WM_MEASUREITEM) { LRESULT(1) } else { LRESULT(0) };
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
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
    // L'API du Shell plutôt que « explorer /select,<chemin> », qui casse si le chemin contient
    // une virgule.
    #[cfg(windows)]
    unsafe {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::UI::Shell::Common::ITEMIDLIST;
        use windows::Win32::UI::Shell::{ILFree, SHOpenFolderAndSelectItems, SHParseDisplayName};
        use windows::core::PCWSTR;
        let w: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
            let ok = SHOpenFolderAndSelectItems(pidl, None, 0).is_ok();
            ILFree(Some(pidl as *const _));
            if ok {
                return;
            }
        }
    }
    let _ = std::process::Command::new("explorer.exe").arg(format!("/select,\"{}\"", path.display())).spawn();
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
    // code.cmd directement (pas « cmd /C code <chemin> » : un « & » dans le chemin serait exécuté).
    // La bibliothèque standard échappe les arguments d'un .cmd, ou refuse s'ils sont dangereux.
    hidden_cmd("code.cmd").arg(path).spawn().is_ok()
}
