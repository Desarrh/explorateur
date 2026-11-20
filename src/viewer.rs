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

/// Formats de conversion proposés pour une extension (mêmes noms que `--convert`).
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
/// Une version du lecteur sans ce mode ignore l'argument et ouvre le fichier normalement.
/// Si le lancement échoue, ou si le lecteur sort en erreur dans la seconde, on envoie le fichier
/// sur `fallback` : l'explorateur affiche alors son propre aperçu, sans message d'erreur.
/// Rien ne bloque l'interface : l'attente se fait sur un thread.
pub fn quick_look(viewer: &Path, file: &Path, fallback: crossbeam_channel::Sender<PathBuf>, ctx: egui::Context) {
    quick_look_via(VIEWER_PIPE, viewer, file, fallback, ctx);
}

/// Même chose avec un nom de tube donné (les tests utilisent un tube qui n'existe pas).
pub fn quick_look_via(pipe: &'static str, viewer: &Path, file: &Path, fallback: crossbeam_channel::Sender<PathBuf>, ctx: egui::Context) {
    let (viewer, file) = (viewer.to_path_buf(), file.to_path_buf());
    std::thread::spawn(move || {
        // Voie rapide : le lecteur tourne déjà (fenêtre d'aperçu en veille) → on lui écrit
        // directement dans son tube nommé, sans lancer de processus (~20–30 ms de gagnés).
        if send_to_pipe(pipe, &file) {
            return;
        }
        let child = std::process::Command::new(&viewer).arg("--apercu").arg(&file).spawn();
        let failed = match child {
            Err(_) => true,
            Ok(mut c) => {
                let t0 = std::time::Instant::now();
                loop {
                    match c.try_wait() {
                        // Sortie rapide normale = fichier transmis à une instance déjà ouverte.
                        Ok(Some(status)) => break !status.success(),
                        Ok(None) if t0.elapsed() < std::time::Duration::from_millis(1200) => {
                            std::thread::sleep(std::time::Duration::from_millis(40));
                        }
                        Ok(None) => break false, // fenêtre ouverte : tout va bien
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

/// Tube de l'instance d'aperçu du lecteur (séparée de sa fenêtre de travail).
/// Protocole : le chemin en UTF-8, une ligne, sans préfixe. Renvoie `false` si aucune instance
/// d'aperçu n'écoute : on lancera alors l'exe avec `--apercu`.
pub const VIEWER_PIPE: &str = r"\\.\pipe\lecteur-universel-apercu-v1";

pub fn send_to_pipe(pipe: &str, file: &Path) -> bool {
    use std::io::Write;
    let t0 = std::time::Instant::now();
    loop {
        match std::fs::OpenOptions::new().write(true).open(pipe) {
            Ok(mut f) => {
                // Le lecteur doit pouvoir passer au premier plan : on lui cède ce droit.
                #[cfg(windows)]
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(u32::MAX); // ASFW_ANY
                }
                return f.write_all(file.to_string_lossy().as_bytes()).is_ok();
            }
            // Tube occupé (le serveur recrée son point d'écoute) : on réessaie un court instant.
            Err(e) if e.raw_os_error() == Some(231) && t0.elapsed() < std::time::Duration::from_millis(60) => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(_) => return false,
        }
    }
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
