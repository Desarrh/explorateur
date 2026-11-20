//! Recherche globale instantanée, à la manière d'Everything (sans droits admin) :
//! - un index compact de tous les noms de fichiers des disques fixes, construit en tâche de fond
//!   avec FindFirstFileExW (lecture par gros blocs) ;
//! - stocké en deux « arènes » de texte (noms tels quels + noms en minuscules) et un tableau de
//!   nœuds (parent, type) : ~40 octets par fichier ;
//! - chercher = une passe `memmem` sur l'arène des minuscules : quelques millisecondes pour
//!   des centaines de milliers de fichiers ;
//! - sauvegardé sur disque : disponible dès le lancement, puis rafraîchi en arrière-plan.
//! Everything lit le journal NTFS en direct, mais il faut un service administrateur ; ici l'index
//! est reconstruit au démarrage puis toutes les 6 heures (les « Récents » en direct couvrent le reste).
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

const NONE: u32 = u32::MAX;
/// Reconstruction complète : rare, les « Récents » en direct couvrent ce qui change entre-temps.
const RESCAN: Duration = Duration::from_secs(6 * 3600);
/// L'index n'est réécrit sur le disque que si la liste des noms a changé, ou au plus une fois par
/// jour pour rafraîchir les dates.
const REWRITE_DATES: Duration = Duration::from_secs(24 * 3600);
/// Dossiers qu'on n'indexe pas : système, corbeilles, et dossiers « bruyants » de build.
const SKIP_DIRS: [&str; 11] = [
    "$recycle.bin", "system volume information", "windows", "node_modules", "target", ".git", "__pycache__", ".cache",
    "winsxs", "$windows.~bt", "$winreagent",
];

#[derive(Clone, Copy)]
struct Node {
    off: u32,
    len: u16,
    /// `DIR` | `NOISY`
    flags: u8,
    parent: u32,
    /// Dernière modification, en minutes depuis 1970 (0 = inconnue). Sert aux « Récents partout ».
    mtime: u32,
}

const DIR: u8 = 1;
/// Dans une zone « bruyante » (AppData, Program Files, dossiers cachés…) : exclu des récents.
const NOISY: u8 = 2;

impl Node {
    fn is_dir(&self) -> bool {
        self.flags & DIR != 0
    }
}

/// Noms en minuscules seulement (Windows ne distingue pas la casse) : la vraie casse
/// n'est relue sur le disque que pour les quelques résultats affichés.
#[derive(Default)]
pub struct Data {
    lower: String,
    nodes: Vec<Node>,
    pub built: Option<SystemTime>,
}

pub struct Hit {
    pub path: PathBuf,
    pub is_dir: bool,
    pub name: String,
}

impl Data {
    fn push(&mut self, name: &str, is_dir: bool, parent: u32, mtime: u32) -> u32 {
        let off = self.lower.len() as u32;
        let low = name.to_lowercase();
        let len = low.len().min(u16::MAX as usize);
        let noisy = (parent != NONE && self.nodes[parent as usize].flags & NOISY != 0)
            || (is_dir && crate::recents::noisy_dir(&low))
            || (!is_dir && crate::recents::noisy_file(&low));
        self.lower.push_str(&low[..len]);
        self.lower.push('\u{0}'); // séparateur : une recherche ne déborde pas sur le nom suivant
        let flags = if is_dir { DIR } else { 0 } | if noisy { NOISY } else { 0 };
        self.nodes.push(Node { off, len: len as u16, flags, parent, mtime });
        (self.nodes.len() - 1) as u32
    }

    /// Les `limit` fichiers modifiés le plus récemment (hors zones bruyantes), du plus récent au
    /// plus ancien : une passe sur les nœuds avec un tas de taille `limit`.
    pub fn recent(&self, limit: usize) -> Vec<(PathBuf, u32)> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::with_capacity(limit + 1);
        for (i, n) in self.nodes.iter().enumerate() {
            if n.flags != 0 || n.mtime == 0 {
                continue;
            }
            if heap.len() < limit {
                heap.push(Reverse((n.mtime, i as u32)));
            } else if heap.peek().map(|r| r.0.0 < n.mtime).unwrap_or(false) {
                heap.pop();
                heap.push(Reverse((n.mtime, i as u32)));
            }
        }
        let mut v: Vec<(u32, u32)> = heap.into_iter().map(|r| r.0).collect();
        v.sort_unstable_by(|a, b| b.cmp(a));
        v.into_iter().map(|(m, i)| (self.path(i), m)).collect()
    }

    fn shrink(&mut self) {
        self.lower.shrink_to_fit();
        self.nodes.shrink_to_fit();
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Empreinte de la liste des noms (sans les dates) : l'index a-t-il changé ?
    fn names_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.lower.hash(&mut h);
        for n in &self.nodes {
            (n.parent, n.flags).hash(&mut h);
        }
        h.finish()
    }

    fn name(&self, i: u32) -> &str {
        let n = self.nodes[i as usize];
        &self.lower[n.off as usize..n.off as usize + n.len as usize]
    }

    fn path(&self, mut i: u32) -> PathBuf {
        let mut parts = Vec::new();
        while i != NONE {
            parts.push(self.name(i));
            i = self.nodes[i as usize].parent;
        }
        let mut p = PathBuf::new();
        for (k, part) in parts.iter().rev().enumerate() {
            if k == 0 {
                p.push(format!("{part}\\")); // racine « C: »
            } else {
                p.push(part);
            }
        }
        p
    }

    /// Les meilleurs résultats pour `query` (sous-chaîne, insensible à la casse).
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let q = query.trim().to_lowercase();
        if q.len() < 2 || self.nodes.is_empty() {
            return vec![];
        }
        let finder = memchr::memmem::Finder::new(q.as_bytes());
        let hay = self.lower.as_bytes();
        let mut cands: Vec<(i32, u32)> = Vec::new();
        let mut last = NONE;
        for pos in finder.find_iter(hay) {
            // nœud qui contient cette position
            let idx = match self.nodes.binary_search_by_key(&(pos as u32), |n| n.off) {
                Ok(i) => i,
                Err(i) => i - 1,
            } as u32;
            if idx == last {
                continue;
            }
            last = idx;
            let start = self.nodes[idx as usize].off as usize;
            let nlen = self.nodes[idx as usize].len as i32;
            // préfixe > début de mot > ailleurs ; noms courts d'abord ; dossiers un peu avant
            let mut score = -nlen;
            if pos == start {
                score += 1000;
            } else if hay.get(pos - 1).map(|b| matches!(b, b' ' | b'_' | b'-' | b'.')).unwrap_or(false) {
                score += 400;
            }
            if nlen as usize == q.len() {
                score += 500;
            }
            if self.nodes[idx as usize].is_dir() {
                score += 50;
            }
            cands.push((score, idx));
            if cands.len() > 200_000 {
                break;
            }
        }
        cands.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        // Les meilleurs candidats : on pénalise ce qui est profond ou caché dans AppData / dossiers « . »
        cands.truncate(600);
        for (score, i) in cands.iter_mut() {
            let mut depth = 0;
            let mut hidden_area = false;
            let mut j = self.nodes[*i as usize].parent;
            while j != NONE {
                depth += 1;
                let n = self.name(j);
                if n == "appdata" || n == "programdata" || n.starts_with('.') || n == "steamapps" {
                    hidden_area = true;
                }
                j = self.nodes[j as usize].parent;
            }
            *score -= depth * 8;
            if hidden_area {
                *score -= 600;
            }
        }
        cands.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        cands
            .into_iter()
            .take(limit)
            .map(|(_, i)| Hit { path: self.path(i), is_dir: self.nodes[i as usize].is_dir(), name: self.name(i).to_string() })
            .collect()
    }

    // ---- sauvegarde ----
    fn save(&self, file: &Path) -> std::io::Result<()> {
        use std::io::Write;
        let tmp = file.with_extension("tmp");
        let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        w.write_all(b"EXPLIDX2")?;
        w.write_all(&(self.nodes.len() as u32).to_le_bytes())?;
        for (i, n) in self.nodes.iter().enumerate() {
            w.write_all(&n.parent.to_le_bytes())?;
            w.write_all(&[n.is_dir() as u8])?;
            w.write_all(&n.mtime.to_le_bytes())?;
            let name = self.name(i as u32).as_bytes();
            w.write_all(&(name.len() as u16).to_le_bytes())?;
            w.write_all(name)?;
        }
        w.flush()?;
        drop(w);
        std::fs::rename(tmp, file)
    }

    fn load(file: &Path) -> Option<Data> {
        let b = std::fs::read(file).ok()?;
        // v1 : sans date (lu quand même, les récents viendront au prochain parcours).
        let v2 = match b.get(..8)? {
            b"EXPLIDX2" => true,
            b"EXPLIDX1" => false,
            _ => return None,
        };
        let n = u32::from_le_bytes(b.get(8..12)?.try_into().ok()?) as usize;
        let mut d = Data::default();
        d.nodes.reserve(n);
        let mut k = 12;
        for _ in 0..n {
            let parent = u32::from_le_bytes(b.get(k..k + 4)?.try_into().ok()?);
            let is_dir = *b.get(k + 4)? != 0;
            k += 5;
            let mtime = if v2 {
                k += 4;
                u32::from_le_bytes(b.get(k - 4..k)?.try_into().ok()?)
            } else {
                0
            };
            let len = u16::from_le_bytes(b.get(k..k + 2)?.try_into().ok()?) as usize;
            let name = std::str::from_utf8(b.get(k + 2..k + 2 + len)?).ok()?;
            if parent != NONE && parent as usize >= d.nodes.len() {
                return None; // fichier abîmé
            }
            d.push(name, is_dir, parent, mtime);
            k += 2 + len;
        }
        d.built = std::fs::metadata(file).and_then(|m| m.modified()).ok();
        d.shrink();
        Some(d)
    }
}

pub struct Index {
    pub data: Arc<RwLock<Data>>,
    pub scanning: Arc<std::sync::atomic::AtomicBool>,
}

fn index_file() -> Option<PathBuf> {
    // (en mode test : dossier « ExplorateurTest », jamais l'index de l'utilisateur)
    dirs::cache_dir().or_else(dirs::config_dir).map(|d| d.join(crate::app::app_name()).join("index.bin"))
}

impl Index {
    /// Charge l'index sauvegardé puis le reconstruit en arrière-plan (et toutes les 6 h).
    pub fn start(ctx: &egui::Context) -> Index {
        let data = Arc::new(RwLock::new(Data::default()));
        let scanning = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (d2, s2, ctx) = (data.clone(), scanning.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("index".into())
            .spawn(move || {
                if let Some(f) = index_file() {
                    if let Some(d) = Data::load(&f) {
                        if let Ok(mut w) = d2.write() {
                            *w = d;
                        }
                        ctx.request_repaint();
                    }
                }
                loop {
                    s2.store(true, std::sync::atomic::Ordering::Relaxed);
                    // Réserve à la taille de l'index précédent : pas de réallocations en cours de route.
                    let (hint_nodes, hint_text, old_sig) =
                        d2.read().map(|d| (d.nodes.len(), d.lower.len(), d.names_signature())).unwrap_or((0, 0, 0));
                    let fresh = scan_all(hint_nodes, hint_text);
                    if let Some(f) = index_file() {
                        if let Some(dir) = f.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        let age = std::fs::metadata(&f).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
                        if fresh.names_signature() != old_sig || age.map(|a| a > REWRITE_DATES).unwrap_or(true) {
                            let _ = fresh.save(&f);
                        }
                    }
                    if let Ok(mut w) = d2.write() {
                        *w = fresh;
                    }
                    s2.store(false, std::sync::atomic::Ordering::Relaxed);
                    ctx.request_repaint();
                    std::thread::sleep(RESCAN);
                }
            })
            .ok();
        Index { data, scanning }
    }
}

/// Racines des disques fixes (`C:\`, `D:\`…).
pub fn fixed_roots() -> Vec<PathBuf> {
    crate::fsx::drives().into_iter().map(|d| d.root).filter(|r| is_fixed(r)).collect()
}

/// Parcourt tous les disques fixes.
fn scan_all(hint_nodes: usize, hint_text: usize) -> Data {
    let t0 = Instant::now();
    let mut d = Data::default();
    d.nodes.reserve(hint_nodes + hint_nodes / 50);
    d.lower.reserve(hint_text + hint_text / 50);
    for drive in crate::fsx::drives() {
        if !is_fixed(&drive.root) {
            continue;
        }
        let label = drive.label.clone(); // « C: »
        let root = d.push(&label, true, NONE, 0);
        let mut stack: Vec<(PathBuf, u32)> = vec![(drive.root.clone(), root)];
        while let Some((dir, id)) = stack.pop() {
            for (name, is_dir, mtime) in list_names(&dir) {
                if is_dir && SKIP_DIRS.contains(&name.to_lowercase().as_str()) {
                    continue;
                }
                let child = d.push(&name, is_dir, id, mtime);
                if is_dir {
                    stack.push((dir.join(&name), child));
                }
            }
            // Ne pas monopoliser le disque : petite pause de temps en temps.
            if d.nodes.len() % 50_000 < 64 {
                std::thread::yield_now();
            }
        }
    }
    d.built = Some(SystemTime::now());
    let _ = t0;
    d.shrink();
    d
}

/// Chemin avec sa vraie casse (lue sur le disque) ; sinon tel quel.
pub fn real_case(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(c) => {
            let s = c.to_string_lossy();
            PathBuf::from(s.strip_prefix(r"\\?\").map(|x| x.to_string()).unwrap_or_else(|| s.into_owned()))
        }
        Err(_) => p.to_path_buf(),
    }
}

#[cfg(windows)]
fn is_fixed(root: &Path) -> bool {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    let w: Vec<u16> = root.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { GetDriveTypeW(w.as_ptr()) == 3 } // DRIVE_FIXED
}
#[cfg(not(windows))]
fn is_fixed(_root: &Path) -> bool {
    true
}

/// Noms d'un dossier (sans suivre les liens/jonctions), lus par gros blocs.
#[cfg(windows)]
fn list_names(dir: &Path) -> Vec<(String, bool, u32)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        FIND_FIRST_EX_LARGE_FETCH, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FindClose, FindExInfoBasic,
        FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW,
    };
    let s = dir.to_string_lossy();
    let pattern = format!(r"\\?\{}\*", s.trim_end_matches('\\'));
    let wide: Vec<u16> = std::ffi::OsStr::new(&pattern).encode_wide().chain(std::iter::once(0)).collect();
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let h = unsafe {
        FindFirstFileExW(wide.as_ptr(), FindExInfoBasic, &mut data as *mut _ as *mut _, FindExSearchNameMatch, std::ptr::null(), FIND_FIRST_EX_LARGE_FETCH)
    };
    let mut out = Vec::new();
    if h == INVALID_HANDLE_VALUE {
        return out;
    }
    loop {
        let n = data.cFileName.iter().position(|&c| c == 0).unwrap_or(data.cFileName.len());
        let name = String::from_utf16_lossy(&data.cFileName[..n]);
        if name != "." && name != ".." {
            let attr = data.dwFileAttributes;
            let is_dir = attr & FILE_ATTRIBUTE_DIRECTORY != 0;
            let surrogate = attr & FILE_ATTRIBUTE_REPARSE_POINT != 0 && data.dwReserved0 & 0x2000_0000 != 0;
            if !(is_dir && surrogate) {
                let ft = ((data.ftLastWriteTime.dwHighDateTime as u64) << 32) | data.ftLastWriteTime.dwLowDateTime as u64;
                out.push((name, is_dir, filetime_minutes(ft)));
            }
        }
        if unsafe { FindNextFileW(h, &mut data) } == 0 {
            break;
        }
    }
    unsafe { FindClose(h) };
    out
}

#[cfg(not(windows))]
fn list_names(dir: &Path) -> Vec<(String, bool, u32)> {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().is_dir(), 0)).collect())
        .unwrap_or_default()
}

/// FILETIME (100 ns depuis 1601) → minutes depuis 1970.
fn filetime_minutes(ft: u64) -> u32 {
    const EPOCH_DIFF_SECS: u64 = 11_644_473_600;
    (ft / 10_000_000).saturating_sub(EPOCH_DIFF_SECS).checked_div(60).unwrap_or(0).min(u32::MAX as u64) as u32
}

#[cfg(test)]
pub fn scan_dir_for_test(root: &Path) -> Data {
    let mut d = Data::default();
    let r = d.push(&root.to_string_lossy().trim_end_matches('\\').to_string(), true, NONE, 0);
    let mut stack = vec![(root.to_path_buf(), r)];
    while let Some((dir, id)) = stack.pop() {
        for (name, is_dir, mtime) in list_names(&dir) {
            let c = d.push(&name, is_dir, id, mtime);
            if is_dir {
                stack.push((dir.join(&name), c));
            }
        }
    }
    d
}

#[cfg(test)]
pub fn roundtrip(d: &Data, file: &Path) -> Option<Data> {
    d.save(file).ok()?;
    Data::load(file)
}

#[cfg(test)]
pub fn full_scan_for_test() -> (Data, Duration) {
    let t0 = Instant::now();
    let d = scan_all(0, 0);
    let t = t0.elapsed();
    (d, t)
}

#[cfg(test)]
impl Data {
    /// Mémoire occupée (approximative).
    pub fn mem_bytes(&self) -> usize {
        self.lower.capacity() + self.nodes.capacity() * std::mem::size_of::<Node>()
    }
}
