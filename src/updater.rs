//! Mise à jour automatique depuis les releases GitHub (Desarrh/explorateur).
//! - vérification en arrière-plan au plus une fois par 24 h (curl.exe, fourni avec Windows 10/11) ;
//! - si une version plus récente existe : bandeau « Installer » ;
//! - installation : téléchargement du zip, l'exe en cours est renommé en `.old` (Windows l'autorise),
//!   le nouveau prend sa place, puis relance. Le `.old` est supprimé au démarrage suivant.
use std::path::{Path, PathBuf};

pub const REPO: &str = "Desarrh/explorateur";

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub asset_url: String,
    pub page: String,
}

fn curl() -> std::process::Command {
    let mut c = std::process::Command::new("curl.exe");
    c.args(["-sSL", "--fail", "--max-time", "120", "-H", "User-Agent: explorateur-updater"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim().trim_start_matches(['v', 'V']).split(['.', '-']);
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next().and_then(|x| x.parse().ok()).unwrap_or(0)))
}

pub fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_version(tag), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Dernière release publiée, si elle est plus récente que cette version.
pub fn check() -> Result<Option<Release>, String> {
    let out = curl()
        .arg(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .output()
        .map_err(|e| format!("curl introuvable : {e}"))?;
    if !out.status.success() {
        return Err("GitHub injoignable".into());
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().unwrap_or_default().to_string();
    if !is_newer(&tag, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let asset_url = v["assets"]
        .as_array()
        .and_then(|a| {
            a.iter().find_map(|x| {
                let name = x["name"].as_str()?;
                (name.to_lowercase().ends_with("windows-x64.zip")).then(|| x["browser_download_url"].as_str().map(String::from))?
            })
        })
        .ok_or("release sans zip Windows")?;
    let page = v["html_url"].as_str().unwrap_or_default().to_string();
    Ok(Some(Release { tag, asset_url, page }))
}

/// Télécharge, remplace l'exe, relance. En cas de succès, le processus courant doit s'arrêter.
pub fn install(r: &Release) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let tmp = std::env::temp_dir().join(format!("explorateur-maj-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let zip_path = tmp.join("maj.zip");
    let st = curl().arg("-o").arg(&zip_path).arg(&r.asset_url).status().map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("téléchargement échoué".into());
    }
    let new_exe = extract_exe(&zip_path, &tmp)?;
    // Remplacement : l'exe en cours ne peut pas être écrasé, mais il peut être renommé.
    let old = exe.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("impossible de renommer l’exe : {e}"))?;
    if let Err(e) = std::fs::copy(&new_exe, &exe) {
        let _ = std::fs::rename(&old, &exe); // on remet l'ancien
        return Err(format!("copie impossible : {e}"));
    }
    let _ = std::fs::remove_dir_all(&tmp);
    std::process::Command::new(&exe).arg("--after-update").spawn().map_err(|e| e.to_string())?;
    Ok(())
}

fn extract_exe(zip_path: &Path, dir: &Path) -> Result<PathBuf, String> {
    let f = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut a = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    for i in 0..a.len() {
        let mut entry = a.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = entry.enclosed_name() else { continue };
        if name.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false) {
            let out = dir.join(name.file_name().unwrap_or_default());
            let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut w).map_err(|e| e.to_string())?;
            return Ok(out);
        }
    }
    Err("pas d’exe dans le zip".into())
}

/// Au démarrage : supprime l'ancien exe laissé par une mise à jour.
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old"));
    }
}
