//! Lecture des dossiers, métadonnées, formatage.
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub lower: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub ext: String,
    pub hidden: bool,
    /// Pour les résultats de recherche récursive : chemin affiché relatif.
    pub display: Option<String>,
}

impl Entry {
    pub fn from_meta(path: PathBuf, meta: &std::fs::Metadata) -> Entry {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let is_dir = meta.is_dir();
        let ext = if is_dir {
            String::new()
        } else {
            path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
        };
        Entry {
            lower: name.to_lowercase(),
            name,
            path,
            is_dir,
            size: if is_dir { 0 } else { meta.len() },
            modified: meta.modified().ok(),
            ext,
            hidden: is_hidden(meta),
            display: None,
        }
    }

    pub fn stat(path: &Path) -> Option<Entry> {
        std::fs::metadata(path).ok().map(|m| Entry::from_meta(path.to_path_buf(), &m))
    }

    /// A-t-il une miniature (image, vidéo, PDF, Office…) ?
    pub fn has_thumb(&self) -> bool {
        !self.is_dir && (is_image_ext(&self.ext) || is_shell_thumb_ext(&self.ext))
    }

    pub fn is_video(&self) -> bool {
        is_video_ext(&self.ext)
    }
}

/// Formats que le socle décode lui-même (liste calculée une fois).
pub fn is_image_ext(ext: &str) -> bool {
    static SET: std::sync::OnceLock<std::collections::HashSet<&'static str>> = std::sync::OnceLock::new();
    SET.get_or_init(|| socle::images::extensions().into_iter().collect()).contains(ext)
}

pub fn is_video_ext(ext: &str) -> bool {
    matches!(ext, "mp4" | "mkv" | "avi" | "mov" | "webm" | "wmv" | "m4v" | "mpg" | "mpeg" | "flv" | "3gp" | "ts")
}

/// Formats que Windows sait réduire en miniature (selon les codecs installés).
pub fn is_shell_thumb_ext(ext: &str) -> bool {
    is_video_ext(ext)
        || matches!(
            ext,
            "pdf" | "docx" | "doc" | "xlsx" | "pptx" | "odt" | "psd" | "heic" | "heif" | "avif" | "tif" | "tiff"
                | "svg" | "mp3" | "flac" | "m4a" | "wma" | "jxl" | "raw" | "cr2" | "nef" | "dng" | "arw" | "blend"
                | "aseprite" | "kra" | "3mf" | "stl" | "glb"
        )
}

pub fn is_text_ext(ext: &str) -> bool {
    matches!(
        ext,
        "txt" | "md" | "rs" | "toml" | "json" | "js" | "ts" | "tsx" | "jsx" | "py" | "gd" | "tscn" | "tres"
            | "godot" | "cfg" | "ini" | "yaml" | "yml" | "html" | "css" | "scss" | "c" | "h" | "cpp" | "hpp"
            | "cs" | "java" | "go" | "sh" | "ps1" | "bat" | "xml" | "csv" | "log" | "sql" | "lua" | "kt"
            | "swift" | "vue" | "svelte" | "env" | "gitignore" | "lock" | "svg"
    )
}

#[cfg(windows)]
fn is_hidden(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    meta.file_attributes() & (HIDDEN | SYSTEM) != 0
}
#[cfg(not(windows))]
fn is_hidden(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Sous Windows, `read_dir` utilise FindFirstFile/FindNextFile : la métadonnée
/// de chaque entrée vient avec l'énumération, sans appel système en plus.
pub fn list_dir(path: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for de in std::fs::read_dir(path)? {
        let Ok(de) = de else { continue };
        let Ok(meta) = de.metadata() else { continue };
        // Les liens symboliques / jonctions : suivre pour savoir si c'est un dossier.
        let meta = if meta.file_type().is_symlink() {
            std::fs::metadata(de.path()).unwrap_or(meta)
        } else {
            meta
        };
        out.push(Entry::from_meta(de.path(), &meta));
    }
    Ok(out)
}

#[derive(Clone)]
pub struct Drive {
    pub root: PathBuf,
    pub label: String,
    pub total: u64,
    pub free: u64,
}

#[cfg(windows)]
pub fn drives() -> Vec<Drive> {
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetLogicalDrives};
    let mask = unsafe { GetLogicalDrives() };
    let mut v = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let (mut free, mut total, mut tfree) = (0u64, 0u64, 0u64);
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut tfree) };
        if ok == 0 {
            continue;
        }
        v.push(Drive { root: PathBuf::from(&root), label: format!("{letter}:"), total, free });
    }
    v
}
#[cfg(not(windows))]
pub fn drives() -> Vec<Drive> {
    vec![]
}

pub fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    if n < 1024 {
        return format!("{n} o");
    }
    let mut f = n as f64;
    let mut i = 0;
    while f >= 1024.0 && i < U.len() - 1 {
        f /= 1024.0;
        i += 1;
    }
    let s = if f < 10.0 { format!("{f:.1}") } else { format!("{f:.0}") };
    format!("{} {}", s.replace('.', ","), U[i])
}

pub fn human_time(t: Option<SystemTime>) -> String {
    use chrono::{DateTime, Local};
    let Some(t) = t else { return String::new() };
    let now = SystemTime::now();
    if let Ok(d) = now.duration_since(t) {
        let s = d.as_secs();
        if s < 10 {
            return "à l’instant".into();
        }
        if s < 60 {
            return format!("il y a {s} s");
        }
        if s < 3600 {
            return format!("il y a {} min", s / 60);
        }
    }
    let dt: DateTime<Local> = t.into();
    let today = Local::now().date_naive();
    let day = dt.date_naive();
    if day == today {
        format!("aujourd’hui {}", dt.format("%H:%M"))
    } else if Some(day) == today.pred_opt() {
        format!("hier {}", dt.format("%H:%M"))
    } else {
        dt.format("%d/%m/%Y %H:%M").to_string()
    }
}

pub fn kind_label(e: &Entry) -> String {
    if e.is_dir {
        return "Dossier".into();
    }
    match e.ext.as_str() {
        "" => "Fichier".into(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" => format!("Image {}", e.ext.to_uppercase()),
        "txt" => "Texte".into(),
        "md" => "Markdown".into(),
        "pdf" => "PDF".into(),
        "exe" => "Application".into(),
        "zip" | "7z" | "rar" => "Archive".into(),
        other => format!("Fichier {}", other.to_uppercase()),
    }
}

/// Nom libre dans `dir` : « nom », « nom (2) », « nom (3) »…
pub fn unique_name(dir: &Path, name: &str) -> PathBuf {
    let cand = dir.join(name);
    if !cand.exists() {
        return cand;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for i in 2.. {
        let c = dir.join(format!("{stem} ({i}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}

pub fn read_text_preview(path: &Path, max: usize) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.iter().take(2048).any(|&b| b == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

// ---------------------------------------------------------------------------------------------
// Dossiers « bruyants » : la seule liste de l'app (noms en minuscules).
// ---------------------------------------------------------------------------------------------

/// Dossiers de build et de cache : leurs changements n'intéressent personne (flux d'activité,
/// recherche récursive, récents) et on ne les indexe pas.
pub fn is_build_dir(low: &str) -> bool {
    matches!(
        low,
        "target" | "node_modules" | "__pycache__" | ".godot" | ".import" | ".next" | ".turbo" | ".cache" | "obj" | "intermediate"
            | "deriveddatacache"
    )
}

/// Dossiers système, corbeilles, dépôts git : jamais indexés.
pub fn is_system_dir(low: &str) -> bool {
    matches!(low, "$recycle.bin" | "system volume information" | "windows" | "winsxs" | "$windows.~bt" | "$winreagent" | ".git")
}

/// Zones où les fichiers changent sans l'utilisateur (AppData, Program Files, dossiers cachés,
/// temporaires, builds) : exclues des récents, moins bien classées dans la recherche.
pub fn is_noisy_dir(low: &str) -> bool {
    low.starts_with('.')
        || low.starts_with('$')
        || is_build_dir(low)
        || is_system_dir(low)
        || matches!(low, "appdata" | "programdata" | "program files" | "program files (x86)" | "steamapps" | "temp" | "tmp")
}

// ---------------------------------------------------------------------------------------------
// Lecture rapide d'un dossier (la seule boucle FindFirstFileExW de l'app).
// ---------------------------------------------------------------------------------------------

/// Une entrée telle que la renvoie `FindFirstFileExW`, sans allocation.
pub struct Found<'a> {
    /// Nom en UTF-16 (sans le zéro final).
    pub name: &'a [u16],
    pub is_dir: bool,
    /// Lien symbolique ou jonction vers un dossier (« name surrogate ») : à ne pas suivre.
    /// Les dossiers OneDrive / cloud sont aussi des points d'analyse, mais pas des liens.
    pub is_link: bool,
    pub size: u64,
    /// Dernière écriture (FILETIME : 100 ns depuis 1601).
    pub write_time: u64,
}

/// Parcourt `dir` par gros blocs (FIND_FIRST_EX_LARGE_FETCH, FindExInfoBasic : taille, date et
/// attributs viennent avec l'énumération, sans appel en plus). Gère les chemins longs et UNC.
#[cfg(windows)]
pub fn for_each_entry(dir: &Path, mut f: impl FnMut(&Found)) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        FIND_FIRST_EX_LARGE_FETCH, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FindClose, FindExInfoBasic, FindExSearchNameMatch,
        FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW,
    };
    // Préfixe \\?\ : chemins de plus de 260 caractères.
    let s = dir.as_os_str().to_string_lossy();
    let pattern = if s.starts_with(r"\\?\") {
        format!(r"{}\*", s.trim_end_matches('\\'))
    } else if let Some(unc) = s.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{}\*", unc.trim_end_matches('\\'))
    } else {
        format!(r"\\?\{}\*", s.trim_end_matches('\\'))
    };
    let wide: Vec<u16> = std::ffi::OsStr::new(&pattern).encode_wide().chain(std::iter::once(0)).collect();
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let h = unsafe {
        FindFirstFileExW(wide.as_ptr(), FindExInfoBasic, &mut data as *mut _ as *mut _, FindExSearchNameMatch, std::ptr::null(), FIND_FIRST_EX_LARGE_FETCH)
    };
    if h == INVALID_HANDLE_VALUE {
        return;
    }
    loop {
        let n = data.cFileName.iter().position(|&c| c == 0).unwrap_or(data.cFileName.len());
        let name = &data.cFileName[..n];
        let dot = name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16];
        if !dot {
            let attr = data.dwFileAttributes;
            let is_dir = attr & FILE_ATTRIBUTE_DIRECTORY != 0;
            f(&Found {
                name,
                is_dir,
                is_link: is_dir && attr & FILE_ATTRIBUTE_REPARSE_POINT != 0 && data.dwReserved0 & 0x2000_0000 != 0,
                size: ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64,
                write_time: ((data.ftLastWriteTime.dwHighDateTime as u64) << 32) | data.ftLastWriteTime.dwLowDateTime as u64,
            });
        }
        if unsafe { FindNextFileW(h, &mut data) } == 0 {
            break;
        }
    }
    unsafe { FindClose(h) };
}
