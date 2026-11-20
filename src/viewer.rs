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
