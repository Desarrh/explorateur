//! Taille des dossiers, façon WizTree (sans droits admin) :
//! - parcours parallèle (vol de tâches rayon, plusieurs dossiers lus en même temps) ;
//! - lecture des dossiers par gros blocs (FindFirstFileExW + FIND_FIRST_EX_LARGE_FETCH,
//!   FindExInfoBasic) : taille et attributs viennent avec l'énumération, aucun appel en plus ;
//! - chaque sous-dossier rencontré est mémorisé : entrer dans un dossier déjà scanné
//!   affiche les tailles instantanément.
//! Changer de dossier annule le calcul en cours (numéro de génération).
//! (WizTree va encore plus vite en lisant la MFT NTFS, mais il faut être administrateur.)
use crossbeam_channel::{unbounded, Receiver, Sender};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Profondeur jusqu'à laquelle on garde les tailles des sous-dossiers en cache.
const KEEP_DEPTH: usize = 6;
const MAX_CACHE: usize = 300_000;

pub struct Sizes {
    tx: Sender<(u64, Vec<PathBuf>, Arc<HashMap<PathBuf, u64>>)>,
    rx: Receiver<Vec<(PathBuf, u64)>>,
    current: Arc<AtomicU64>,
    pub map: HashMap<PathBuf, u64>,
    pending: HashSet<PathBuf>,
    /// Change à chaque arrivée ou invalidation de tailles (la carte de l'espace se relit).
    pub version: u64,
}

impl Sizes {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, req_rx) = unbounded::<(u64, Vec<PathBuf>, Arc<HashMap<PathBuf, u64>>)>();
        let (res_tx, rx) = unbounded();
        let current = Arc::new(AtomicU64::new(0));
        let cur = current.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // Disques SSD : beaucoup de lectures en parallèle paient.
            let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8).clamp(4, 16);
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
            while let Ok((g, dirs, known)) = req_rx.recv() {
                if g != cur.load(Ordering::Relaxed) {
                    continue;
                }
                let res_tx = res_tx.clone();
                let ctx = ctx.clone();
                let cur = cur.clone();
                pool.install(|| {
                    dirs.par_iter().for_each(|d| {
                        let found = Mutex::new(Vec::new());
                        if let Some(total) = walk(d, 0, g, &cur, &found, &known) {
                            let mut v = found.into_inner().unwrap_or_default();
                            v.push((d.clone(), total));
                            let _ = res_tx.send(v);
                            ctx.request_repaint();
                        }
                    });
                });
            }
        });
        Sizes { tx, rx, current, map: HashMap::new(), pending: HashSet::new(), version: 0 }
    }

    /// Calcule les dossiers donnés (ceux du dossier affiché), en abandonnant les anciens.
    pub fn focus(&mut self, dirs: Vec<PathBuf>) {
        let g = self.current.fetch_add(1, Ordering::Relaxed) + 1;
        self.pending.clear();
        let todo: Vec<PathBuf> = dirs.into_iter().filter(|d| !self.map.contains_key(d)).collect();
        if todo.is_empty() {
            return;
        }
        self.pending.extend(todo.iter().cloned());
        // Sous-dossiers déjà mesurés (sous le dossier affiché) : réutilisés tels quels par `walk`.
        let parent = todo[0].parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let known: HashMap<PathBuf, u64> = self.map.iter().filter(|(k, _)| k.starts_with(&parent)).map(|(k, v)| (k.clone(), *v)).collect();
        let _ = self.tx.send((g, todo, Arc::new(known)));
    }

    /// `true` si de nouvelles tailles sont arrivées.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok(batch) = self.rx.try_recv() {
            for (p, s) in batch {
                self.pending.remove(&p);
                self.map.insert(p, s);
            }
            any = true;
        }
        if self.map.len() > MAX_CACHE {
            self.map.clear();
        }
        if any {
            self.version += 1;
        }
        any
    }

    /// Dossiers dont la taille est en cours de calcul.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn is_pending(&self, p: &Path) -> bool {
        self.pending.contains(p)
    }

    /// Un fichier a changé : les tailles des dossiers qui le contiennent sont périmées.
    pub fn invalidate(&mut self, changed: &Path) -> bool {
        let before = self.map.len();
        // On ne retire que les ancêtres du fichier (pas de parcours complet de la carte).
        let mut a = changed.parent();
        while let Some(p) = a {
            self.map.remove(p);
            a = p.parent();
        }
        self.map.remove(changed);
        let changed = self.map.len() != before;
        if changed {
            self.version += 1;
        }
        changed
    }
}

/// Taille totale d'un dossier. `None` si annulé.
fn walk(dir: &Path, depth: usize, g: u64, cur: &AtomicU64, found: &Mutex<Vec<(PathBuf, u64)>>, known: &HashMap<PathBuf, u64>) -> Option<u64> {
    if g != cur.load(Ordering::Relaxed) {
        return None;
    }
    if depth > 0 {
        if let Some(&s) = known.get(dir) {
            return Some(s);
        }
    }
    let (files, subdirs) = list(dir);
    let sub: Option<u64> = if subdirs.len() > 1 {
        subdirs.par_iter().map(|s| walk(s, depth + 1, g, cur, found, known)).try_reduce(|| 0, |a, b| Some(a + b))
    } else {
        subdirs.iter().try_fold(0u64, |acc, s| walk(s, depth + 1, g, cur, found, known).map(|v| acc + v))
    };
    let total = files + sub?;
    if depth > 0 && depth <= KEEP_DEPTH {
        if let Ok(mut f) = found.lock() {
            f.push((dir.to_path_buf(), total));
        }
    }
    Some(total)
}

/// Somme des fichiers d'un dossier + liste de ses sous-dossiers (sans suivre liens ni jonctions).
#[cfg(windows)]
fn list(dir: &Path) -> (u64, Vec<PathBuf>) {
    use std::os::windows::ffi::OsStringExt;
    let (mut files, mut dirs) = (0u64, Vec::new());
    crate::fsx::for_each_entry(dir, |e| {
        if !e.is_dir {
            files += e.size;
        } else if !e.is_link {
            dirs.push(dir.join(std::ffi::OsString::from_wide(e.name)));
        }
    });
    (files, dirs)
}

#[cfg(not(windows))]
fn list(dir: &Path) -> (u64, Vec<PathBuf>) {
    let mut files = 0;
    let mut dirs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                dirs.push(e.path());
            } else if let Ok(m) = e.metadata() {
                files += m.len();
            }
        }
    }
    (files, dirs)
}

/// Mesure de vitesse : `cargo test --release bench_taille -- --ignored --nocapture`
#[cfg(test)]
pub fn bench(dir: &Path) -> (u64, usize, std::time::Duration) {
    let cur = AtomicU64::new(1);
    let found = Mutex::new(Vec::new());
    let t0 = std::time::Instant::now();
    let total = walk(dir, 0, 1, &cur, &found, &HashMap::new()).unwrap_or(0);
    (total, found.into_inner().unwrap().len(), t0.elapsed())
}
