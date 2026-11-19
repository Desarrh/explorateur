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
