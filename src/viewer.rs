//! Lien avec le lecteur universel (`C:\Dev\lecteur-universel`) : ouvrir un fichier dedans,
//! et « Convertir en… » sans fenêtre (`lecteur-universel --convert <format> <fichiers…>`).
use std::path::{Path, PathBuf};

/// Cherche l'exe du lecteur : à côté de l'explorateur, puis dans son projet.
pub fn find() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            cands.push(dir.join("lecteur-universel.exe"));
            cands.push(dir.join("Lecteur.exe"));
        }
    }
    for root in [r"C:\Dev\lecteur-universel"] {
        let r = Path::new(root);
        cands.push(r.join("dist").join("Lecteur.exe"));
        cands.push(r.join("dist").join("lecteur-universel.exe"));
        cands.push(r.join("target").join("release").join("lecteur-universel.exe"));
    }
    cands.into_iter().find(|p| p.is_file())
}

/// Le lecteur sait-il ouvrir ce type ?
pub fn handles(ext: &str) -> bool {
    !targets(ext).is_empty()
}

/// Une cible de conversion (`--convert <ext>`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Target {
    pub ext: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(serde::Deserialize)]
struct FormatsAnswer {
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    targets: Vec<Target>,
}

/// Le lecteur annonce ses options de ligne de commande dans sa ressource de version
/// (`LecteurCli` = « formats apercu … »). Absente : lecteur ancien, on ne lui envoie rien de neuf
/// (un ancien lecteur prendrait « --formats pdf » pour un fichier à ouvrir).
#[cfg(windows)]
pub fn cli_caps(exe: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    let w: Vec<u16> = exe.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let size = GetFileVersionInfoSizeW(w.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(w.as_ptr(), 0, size, buf.as_mut_ptr() as *mut _) == 0 {
            return None;
        }
        let query = |key: &str| -> Option<(*const u8, u32)> {
            let k: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
            let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
            let mut len = 0u32;
            (VerQueryValueW(buf.as_ptr() as *const _, k.as_ptr(), &mut ptr, &mut len) != 0 && !ptr.is_null()).then_some((ptr as *const u8, len))
        };
        // Langues déclarées, puis les plus courantes.
        let mut langs: Vec<(u16, u16)> = Vec::new();
        if let Some((p, len)) = query(r"\VarFileInfo\Translation") {
            let pairs = std::slice::from_raw_parts(p as *const u16, (len / 2) as usize);
            langs.extend(pairs.chunks_exact(2).map(|c| (c[0], c[1])));
        }
        langs.extend([(0x0000, 0x04B0), (0x0409, 0x04B0), (0x040C, 0x04B0)]);
        for (l, c) in langs {
            if let Some((p, len)) = query(&format!(r"\StringFileInfo\{l:04x}{c:04x}\LecteurCli")) {
                let chars = std::slice::from_raw_parts(p as *const u16, len as usize);
                let s = String::from_utf16_lossy(chars);
                return Some(s.trim_end_matches('\0').to_string());
            }
        }
        None
    }
}

/// `lecteur --formats <ext>` : les conversions que ce lecteur sait faire (≈ 100 ms, à appeler sur
/// un thread et à mettre en cache). `None` si la réponse est absente ou illisible.
pub fn query_formats(exe: &Path, ext: &str) -> Option<Vec<Target>> {
    let mut c = std::process::Command::new(exe);
    c.arg("--formats").arg(ext);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let o = c.output().ok()?;
    if !o.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&o.stdout);
    let a: FormatsAnswer = serde_json::from_str(line.trim()).ok()?;
    Some(if a.supported { a.targets } else { vec![] })
}

/// Liste interne (repli quand le lecteur ne sait pas répondre à `--formats`).
pub fn fallback_targets(ext: &str) -> Vec<Target> {
    targets(ext).iter().map(|e| Target { ext: e.to_string(), label: e.to_uppercase(), detail: String::new() }).collect()
}

/// Formats de conversion de la liste interne (mêmes noms que `--convert`).
pub fn targets(ext: &str) -> &'static [&'static str] {
    match ext {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico" | "tif" | "tiff" => &["png", "jpg", "webp", "bmp", "tiff", "ico", "gif"],
        "pdf" => &["png", "jpg", "webp"],
        "docx" => &["pdf", "txt", "md", "html"],
        "mp3" | "wav" | "m4a" | "flac" | "ogg" | "aac" | "wma" => &["wav", "mp3"],
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "wmv" | "m4v" => &["mp4", "gif", "mp3", "wav", "png"],
        e if crate::fsx::is_text_ext(e) && e != "svg" => &["pdf", "html", "txt"],
        _ => &[],
    }
}

/// Aperçu rapide (Espace) : `lecteur-universel --apercu <fichier>` (vue épurée, Échap/Espace ferment).
/// Si l'aperçu du lecteur est déjà ouvert, ce 2ᵉ processus lui transmet le fichier par l'instance
/// unique du socle et sort aussitôt : on ne parle jamais directement à son tube (son nom et son
/// format changent avec la version du socle du lecteur ; la ligne de commande, elle, reste stable).
/// Si le lancement échoue, ou si le lecteur sort en erreur dans la seconde, on envoie le fichier
/// sur `fallback` : l'explorateur affiche alors son propre aperçu, sans message d'erreur.
/// Rien ne bloque l'interface : l'attente se fait sur un thread.
pub fn quick_look(viewer: &Path, file: &Path, fallback: crossbeam_channel::Sender<PathBuf>, ctx: egui::Context) {
    quick_look_within(viewer, file, fallback, ctx, std::time::Duration::from_millis(1200));
}

/// Même chose avec le délai pendant lequel une sortie en erreur déclenche le repli (tests).
pub fn quick_look_within(
    viewer: &Path,
    file: &Path,
    fallback: crossbeam_channel::Sender<PathBuf>,
    ctx: egui::Context,
    wait: std::time::Duration,
) {
    let (viewer, file) = (viewer.to_path_buf(), file.to_path_buf());
    std::thread::spawn(move || {
        let child = std::process::Command::new(&viewer).arg("--apercu").arg(&file).spawn();
        let failed = match child {
            Err(_) => true,
            Ok(mut c) => {
                let t0 = std::time::Instant::now();
                loop {
                    match c.try_wait() {
                        // Sortie rapide normale = fichier transmis à l'aperçu déjà ouvert.
                        Ok(Some(status)) => break !status.success(),
                        Ok(None) if t0.elapsed() < wait => std::thread::sleep(std::time::Duration::from_millis(40)),
                        Ok(None) => break false, // fenêtre d'aperçu ouverte : tout va bien
                        Err(_) => break true,
                    }
                }
            }
        };
        if failed {
            let _ = fallback.send(file);
            ctx.request_repaint();
        }
    });
}

pub fn open(viewer: &Path, files: &[PathBuf]) -> bool {
    std::process::Command::new(viewer).args(files).spawn().is_ok()
}

/// Lance une conversion et attend la fin. Renvoie (fichiers créés, erreurs).
pub fn convert(viewer: &Path, fmt: &str, files: &[PathBuf]) -> (Vec<PathBuf>, Vec<String>) {
    let mut c = std::process::Command::new(viewer);
    c.arg("--convert").arg(fmt).args(files);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    match c.output() {
        Ok(o) => {
            let created: Vec<PathBuf> =
                String::from_utf8_lossy(&o.stdout).lines().map(|l| PathBuf::from(l.trim())).filter(|p| p.exists()).collect();
            let mut errors: Vec<String> =
                String::from_utf8_lossy(&o.stderr).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
            if !o.status.success() && errors.is_empty() {
                errors.push(format!("le lecteur a échoué (code {})", o.status.code().unwrap_or(-1)));
            }
            (created, errors)
        }
        Err(e) => (vec![], vec![format!("lecteur introuvable : {e}")]),
    }
}
