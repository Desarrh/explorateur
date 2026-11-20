//! **Rafales d'agent** (E1) : les changements d'un dépôt sont regroupés en « passes » (une passe se
//! termine après 20 s sans écriture). Chaque passe garde un instantané git *avant* et *après*
//! (arbres écrits avec un index temporaire, sans toucher à l'index, à la branche ni à l'historique),
//! ce qui donne son diff cumulé exact et une restauration en bloc réversible.
use crate::app::Act;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Silence qui termine une passe.
pub const QUIET: Duration = Duration::from_secs(20);
/// Passes gardées en mémoire (toutes dépôts confondus).
const MAX_BURSTS: usize = 40;

pub struct Burst {
    pub id: u64,
    pub root: PathBuf,
    pub start: SystemTime,
    pub end: SystemTime,
    pub last: Instant,
    /// Chemin relatif (séparateur `/`) → nature nette du changement sur la passe.
    pub files: BTreeMap<String, Act>,
    /// Arbre git avant la passe (instantané précédent du dépôt).
    pub before: Option<String>,
    /// Arbre git à la fin de la passe (calculé à la clôture).
    pub after: Option<String>,
    /// Fichiers trop gros pour l'instantané : jamais restaurés.
    pub skipped: Vec<String>,
    pub closed: bool,
    /// Après « Restaurer » : arbre pour « Rétablir ».
    pub undo: Option<String>,
    pub busy: bool,
}

impl Burst {
    pub fn open(&self) -> bool {
        !self.closed
    }
    /// Fichiers à restaurer (sans les trop gros).
    pub fn restorable(&self) -> Vec<String> {
        self.files.keys().filter(|f| !self.skipped.contains(f)).cloned().collect()
    }
}

/// Ce que le journal demande au thread git.
pub enum SnapWhy {
    /// Premier instantané d'un dépôt (état du disque au moment où on commence à le suivre).
    Base,
    /// Clôture de la passe `id`.
    Close(u64),
}

pub struct SnapRequest {
    pub root: PathBuf,
    pub base: Option<String>,
    pub files: Vec<String>,
    pub why: SnapWhy,
}

#[derive(Default)]
pub struct Bursts {
    pub list: VecDeque<Burst>,
    /// Dernier instantané connu de chaque dépôt (base de la passe suivante).
    snaps: HashMap<PathBuf, String>,
    /// Instantané de base déjà demandé.
    asked: std::collections::HashSet<PathBuf>,
    /// Fichiers réécrits par une restauration : leurs événements ne forment pas une nouvelle passe.
    muted: HashMap<PathBuf, (std::collections::HashSet<String>, Instant)>,
    next_id: u64,
}

impl Bursts {
    /// À appeler quand un dépôt devient actif : demande l'instantané de base une seule fois.
    pub fn watch_repo(&mut self, root: &Path) -> Option<SnapRequest> {
        if !self.asked.insert(root.to_path_buf()) {
            return None;
        }
        Some(SnapRequest { root: root.to_path_buf(), base: None, files: vec![], why: SnapWhy::Base })
    }

    /// Un fichier du dépôt vient de changer.
    pub fn record(&mut self, root: &Path, rel: String, act: Act) {
        let now = Instant::now();
        if let Some((files, until)) = self.muted.get(root) {
            if now < *until && files.contains(&rel) {
                return;
            }
        }
        let open = self.list.iter_mut().find(|b| b.open() && b.root == root);
        let b = match open {
            Some(b) => b,
            None => {
                self.next_id += 1;
                let before = self.snaps.get(root).cloned();
                self.list.push_front(Burst {
                    id: self.next_id,
                    root: root.to_path_buf(),
                    start: SystemTime::now(),
                    end: SystemTime::now(),
                    last: now,
                    files: BTreeMap::new(),
                    before,
                    after: None,
                    skipped: vec![],
                    closed: false,
                    undo: None,
                    busy: false,
                });
                // On ne jette que des passes terminées.
                while self.list.len() > MAX_BURSTS {
                    match self.list.iter().rposition(|b| b.closed) {
                        Some(i) => {
                            self.list.remove(i);
                        }
                        None => break,
                    }
                }
                self.list.front_mut().unwrap()
            }
        };
        b.last = now;
        b.end = SystemTime::now();
        match (b.files.get(&rel).copied(), act) {
            // Créé puis supprimé pendant la passe : rien de net.
            (Some(Act::Created), Act::Removed) => {
                b.files.remove(&rel);
            }
            // Un fichier créé reste « créé » même s'il est réécrit ensuite.
            (Some(Act::Created), _) => {}
            // Supprimé puis recréé : c'est une modification.
            (Some(Act::Removed), Act::Created) => {
                b.files.insert(rel, Act::Modified);
            }
            _ => {
                b.files.insert(rel, act);
            }
        }
    }

    /// Clôt les passes silencieuses depuis 20 s et renvoie les instantanés à calculer.
    pub fn tick(&mut self) -> Vec<SnapRequest> {
        let mut out = Vec::new();
        for b in self.list.iter_mut().filter(|b| b.open() && b.last.elapsed() >= QUIET) {
            b.closed = true;
            if b.files.is_empty() {
                continue;
            }
            if b.before.is_none() {
                b.before = self.snaps.get(&b.root).cloned();
            }
            if let Some(base) = b.before.clone() {
                b.busy = true;
                out.push(SnapRequest { root: b.root.clone(), base: Some(base), files: b.files.keys().cloned().collect(), why: SnapWhy::Close(b.id) });
            }
        }
        // Passes vidées (créé puis supprimé) : inutile de les montrer.
        self.list.retain(|b| b.open() || !b.files.is_empty());
        out
    }

    /// Délai avant la prochaine clôture (pour redessiner à temps).
    pub fn next_close_in(&self) -> Option<Duration> {
        self.list.iter().filter(|b| b.open()).map(|b| QUIET.saturating_sub(b.last.elapsed())).min()
    }

    /// Réponse du thread git à un instantané.
    pub fn on_snapshot(&mut self, root: &Path, why: SnapWhy, tree: Result<String, String>, skipped: Vec<String>) {
        let Ok(tree) = tree else {
            if let SnapWhy::Close(id) = why {
                if let Some(b) = self.get_mut(id) {
                    b.busy = false;
                }
            }
            return;
        };
        self.snaps.insert(root.to_path_buf(), tree.clone());
        match why {
            SnapWhy::Base => {
                for b in self.list.iter_mut().filter(|b| b.root == root && b.before.is_none()) {
                    b.before = Some(tree.clone());
                }
            }
            SnapWhy::Close(id) => {
                if let Some(b) = self.get_mut(id) {
                    b.after = Some(tree);
                    b.skipped = skipped;
                    b.busy = false;
                }
            }
        }
    }

    pub fn get(&self, id: u64) -> Option<&Burst> {
        self.list.iter().find(|b| b.id == id)
    }
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Burst> {
        self.list.iter_mut().find(|b| b.id == id)
    }

    /// Après une restauration, l'état du disque a changé : la passe suivante partira de là.
    pub fn set_snapshot(&mut self, root: &Path, tree: String) {
        self.snaps.insert(root.to_path_buf(), tree);
    }

    /// Ignore les événements de ces fichiers pendant 3 s (réécrits par une restauration).
    pub fn mute(&mut self, root: &Path, files: &[String]) {
        self.muted.insert(root.to_path_buf(), (files.iter().cloned().collect(), Instant::now() + Duration::from_secs(3)));
    }

    /// Instantané courant d'un dépôt.
    pub fn snapshot_of(&self, root: &Path) -> Option<String> {
        self.snaps.get(root).cloned()
    }
}
