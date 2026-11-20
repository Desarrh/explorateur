//! **Récents partout** (E2) : les fichiers modifiés récemment sur tous les disques fixes (exports,
//! captures, téléchargements…), dans un onglet « Récents » qui se comporte comme un dossier.
//! - au lancement : les dates déjà dans l'index global (4 octets par nœud), sélection des plus
//!   récents par un tas de taille fixe ;
//! - en direct : une surveillance récursive de chaque disque fixe (ReadDirectoryChangesW, sans
//!   droits admin, 0 % CPU au repos) ajoute les fichiers dès qu'ils changent ;
//! - les zones « bruyantes » (AppData, Program Files, dossiers cachés, builds, fichiers temporaires)
//!   sont ignorées.
use crate::fsx::Entry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

/// Chemin virtuel de l'onglet « Récents » (le « : » est interdit dans un nom de fichier Windows).
pub const RECENTS: &str = "::Récents";
/// Fichiers montrés.
pub const LIMIT: usize = 300;
/// Fichiers retenus par la surveillance en direct.
const LIVE_MAX: usize = 3000;

pub fn path() -> PathBuf {
    PathBuf::from(RECENTS)
}

pub fn is_recents(p: &Path) -> bool {
    p.as_os_str() == RECENTS
}

/// Dossier (nom en minuscules) dont le contenu change sans que l'utilisateur y soit pour rien.
pub fn noisy_dir(low: &str) -> bool {
    crate::fsx::is_noisy_dir(low)
}

/// Fichier (nom en minuscules) technique ou temporaire.
pub fn noisy_file(low: &str) -> bool {
    low.starts_with("~$")
        || low.starts_with('.')
        || matches!(low, "thumbs.db" | "desktop.ini" | "ntuser.dat")
        || [".tmp", ".temp", ".part", ".crdownload", ".partial", ".lock", ".log", ".etl", ".db-journal", ".db-wal", ".db-shm", ".pf"]
            .iter()
            .any(|x| low.ends_with(x))
}

/// Un chemin complet est-il dans une zone bruyante ?
pub fn noisy_path(p: &Path) -> bool {
    let mut comps = p.components().peekable();
    while let Some(c) = comps.next() {
        let std::path::Component::Normal(os) = c else { continue };
        let low = os.to_string_lossy().to_lowercase();
        let last = comps.peek().is_none();
        if if last { noisy_file(&low) } else { noisy_dir(&low) } {
            return true;
        }
    }
    false
}

/// Fichiers modifiés vus en direct depuis le lancement.
pub struct Live {
    map: Arc<Mutex<HashMap<PathBuf, SystemTime>>>,
    /// Incrémenté à chaque nouveau fichier : l'onglet « Récents » se recharge s'il a changé.
    pub generation: Arc<AtomicU64>,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl Live {
    /// Surveille récursivement chaque disque fixe.
    pub fn start(roots: &[PathBuf]) -> Live {
        use notify::{EventKind, Watcher};
        let map: Arc<Mutex<HashMap<PathBuf, SystemTime>>> = Arc::default();
        let generation = Arc::new(AtomicU64::new(0));
        let (m2, g2) = (map.clone(), generation.clone());
        let roots2 = roots.to_vec();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if !matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                return;
            }
            // Renommage : le dernier chemin est le nouveau nom.
            let Some(p) = ev.paths.last() else { return };
            // Filtre sur le chemin relatif à la racine surveillée (le disque).
            let rel = roots2.iter().find_map(|r| p.strip_prefix(r).ok()).unwrap_or(p);
            if noisy_path(rel) || p.is_dir() {
                return;
            }
            let Ok(mut m) = m2.lock() else { return };
            m.insert(p.clone(), SystemTime::now());
            if m.len() > LIVE_MAX {
                let mut v: Vec<(PathBuf, SystemTime)> = m.drain().collect();
                v.sort_unstable_by(|a, b| b.1.cmp(&a.1));
                v.truncate(LIVE_MAX * 2 / 3);
                m.extend(v);
            }
            g2.fetch_add(1, Ordering::Relaxed);
        })
        .ok();
        let watcher = watcher.map(|mut w| {
            for r in roots {
                let _ = w.watch(r, notify::RecursiveMode::Recursive);
            }
            w
        });
        Live { map, generation, _watcher: watcher }
    }

    /// Liste partagée avec le thread de chargement de l'onglet.
    pub fn map(&self) -> LiveMap {
        self.map.clone()
    }
}

pub type LiveMap = Arc<Mutex<HashMap<PathBuf, SystemTime>>>;

fn live_recent(map: &Mutex<HashMap<PathBuf, SystemTime>>, n: usize) -> Vec<PathBuf> {
    let Ok(m) = map.lock() else { return vec![] };
    let mut v: Vec<(&PathBuf, &SystemTime)> = m.iter().collect();
    v.sort_unstable_by(|a, b| b.1.cmp(a.1));
    v.into_iter().take(n).map(|(p, _)| p.clone()).collect()
}

/// Construit la liste : candidats de l'index + du direct, relus sur le disque (ceux qui ont disparu
/// sont écartés, la date est exacte), du plus récent au plus ancien.
pub fn collect(index: Option<&RwLock<crate::index::Data>>, live: Option<&Mutex<HashMap<PathBuf, SystemTime>>>, limit: usize) -> Vec<Entry> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(l) = live {
        cands.extend(live_recent(l, limit));
    }
    if let Some(ix) = index {
        if let Ok(d) = ix.read() {
            cands.extend(d.recent(limit).into_iter().map(|(p, _)| crate::index::real_case(&p)));
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Entry> = cands
        .into_iter()
        .filter(|p| seen.insert(p.to_string_lossy().to_lowercase()))
        .filter_map(|p| Entry::stat(&p))
        .filter(|e| !e.is_dir && !e.hidden)
        .map(|mut e| {
            let dir = e.path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
            e.display = Some(format!("{}   ·   {dir}", e.name));
            e
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.truncate(limit);
    out
}
