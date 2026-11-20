//! État de l'application et logique (sans dessin).
use crate::fsx::{self, Entry};
use crate::git::{self, DiffLine};
use crate::ops::{Op, OpEvent, Ops};
use socle::theme::{self, Palette};
use crate::thumbs::Thumbs;
use crate::winx;
use crossbeam_channel::{unbounded, Receiver, Sender};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

pub const MARK_SECS: f32 = 8.0;
const NOISY: [&str; 8] = ["target", "node_modules", ".godot", ".import", "__pycache__", ".next", ".turbo", ".cache"];

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Sort {
    Name,
    Modified,
    Kind,
    Size,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub dev: bool,
    pub grid: bool,
    /// Vue « Carte de l'espace » (E3) à la place de la liste / grille.
    pub space_map: bool,
    pub preview: bool,
    pub show_hidden: bool,
    pub pinned: Vec<PathBuf>,
    pub tabs: Vec<PathBuf>,
    pub active: usize,
    pub sort: Sort,
    pub asc: bool,
    pub window: Option<(f32, f32)>,
    /// Dossiers visités : nombre de visites et dernière visite (secondes Unix) → « Aller à ».
    pub visits: HashMap<String, (u32, i64)>,
    pub folder_sizes: bool,
    /// Historiques d'autocomplétion (T4) : filtre et barre d'adresse.
    pub filter_history: socle::autocomplete::History,
    pub addr_history: socle::autocomplete::History,
    /// Commandes récentes de la palette Ctrl+K (en tête de liste).
    pub palette_recent: Vec<String>,
    /// Ouvrir images, PDF, Word, audio, vidéo, code dans le lecteur universel (double-clic, Entrée).
    pub use_viewer: bool,
    /// Espace = aperçu rapide dans le lecteur universel (sinon aperçu plein écran intégré).
    pub quick_viewer: bool,
    /// Index global de tous les fichiers (recherche instantanée dans Ctrl+P).
    pub global_index: bool,
    /// Vérifier les mises à jour sur GitHub (au plus une fois par jour).
    pub auto_update: bool,
    pub last_update_check: i64,

}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dark: false,
            dev: false,
            grid: false,
            space_map: false,
            preview: true,
            show_hidden: false,
            pinned: vec![],
            tabs: vec![],
            active: 0,
            sort: Sort::Name,
            asc: true,
            window: None,
            visits: HashMap::new(),
            folder_sizes: true,
            filter_history: Default::default(),
            addr_history: Default::default(),
            palette_recent: Vec::new(),
            use_viewer: true,
            quick_viewer: true,
            global_index: true,
            auto_update: true,
            last_update_check: 0,

        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    Created,
    Modified,
    Removed,
}

impl Act {
    pub fn label(self) -> &'static str {
        match self {
            Act::Created => "créé",
            Act::Modified => "modifié",
            Act::Removed => "supprimé",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Mark {
    pub at: Instant,
    pub kind: Act,
}

pub struct Activity {
    pub path: PathBuf,
    pub label: String,
    pub kind: Act,
    pub at: Instant,
    pub wall: SystemTime,
    pub count: u32,
}

pub struct Search {
    pub generation: u64,
    pub results: Vec<Entry>,
    pub running: bool,
    pub cancel: Arc<AtomicBool>,
}

pub struct Tab {
    pub id: u64,
    pub path: PathBuf,
    pub back: Vec<PathBuf>,
    pub fwd: Vec<PathBuf>,
    pub entries: Vec<Entry>,
    pub view: Vec<usize>,
    pub view_dirty: bool,
    pub generation: u64,
    pub loading: bool,
    pub load_ms: f32,
    pub error: Option<String>,
    pub selected: HashSet<PathBuf>,
    pub cursor: Option<PathBuf>,
    pub anchor: Option<PathBuf>,
    pub filter: String,
    pub recursive: bool,
    pub search: Option<Search>,
    pub marks: HashMap<PathBuf, Mark>,
    pub renaming: Option<Rename>,
    pub address: Option<String>,
    pub scroll_to: Option<usize>,
    pub focus_after_load: Option<PathBuf>,
    pub scroll_offset: f32,
    pub view_h: f32,
}

pub struct Rename {
    pub path: PathBuf,
    pub text: String,
    pub fresh: bool,
}

impl Tab {
    fn new(id: u64, path: PathBuf) -> Tab {
        Tab {
            id,
            path,
            back: vec![],
            fwd: vec![],
            entries: vec![],
            view: vec![],
            view_dirty: true,
            generation: 0,
            loading: false,
            load_ms: 0.0,
            error: None,
            selected: HashSet::new(),
            cursor: None,
            anchor: None,
            filter: String::new(),
            recursive: false,
            search: None,
            marks: HashMap::new(),
            renaming: None,
            address: None,
            scroll_to: None,
            focus_after_load: None,
            scroll_offset: 0.0,
            view_h: 400.0,
        }
    }

    pub fn source(&self) -> &[Entry] {
        match &self.search {
            Some(s) => &s.results,
            None => &self.entries,
        }
    }

    pub fn entry(&self, view_idx: usize) -> Option<&Entry> {
        self.view.get(view_idx).and_then(|&i| self.source().get(i))
    }

    pub fn view_index_of(&self, p: &Path) -> Option<usize> {
        let src = self.source();
        self.view.iter().position(|&i| src.get(i).map(|e| e.path.as_path()) == Some(p))
    }

    pub fn title(&self) -> String {
        if crate::recents::is_recents(&self.path) {
            return "Récents".into();
        }
        self.path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().trim_end_matches('\\').to_string())
    }

    pub fn has_live(&self) -> bool {
        self.marks.values().any(|m| m.at.elapsed().as_secs_f32() < MARK_SECS)
    }

    /// Chemins sélectionnés, dans l'ordre d'affichage.
    pub fn selection(&self) -> Vec<PathBuf> {
        let src = self.source();
        self.view
            .iter()
            .filter_map(|&i| src.get(i))
            .filter(|e| self.selected.contains(&e.path))
            .map(|e| e.path.clone())
            .collect()
    }
}

pub struct Repo {
    pub root: PathBuf,
    pub branch: String,
    pub ahead: String,
    pub changes: Vec<git::Change>,
    pub excluded: HashSet<String>,
    pub selected: Option<String>,
    pub diff: Vec<DiffLine>,
    pub diff_for: Option<String>,
    pub msg: String,
    pub busy: Option<&'static str>,
    pub last: Option<(String, bool)>,
    pub by_path: HashMap<PathBuf, git::Kind>,
    pub loaded: bool,
    /// Règles .gitignore du dépôt (racine + .git/info/exclude) : filtrent le flux d'activité.
    pub ignore: Option<Arc<ignore::gitignore::Gitignore>>,
}

impl Repo {
    fn new(root: PathBuf) -> Repo {
        let mut b = ignore::gitignore::GitignoreBuilder::new(&root);
        let _ = b.add(root.join(".gitignore"));
        let _ = b.add(root.join(".git").join("info").join("exclude"));
        let ignore = b.build().ok().map(Arc::new);
        Repo {
            ignore,
            root,
            branch: String::new(),
            ahead: String::new(),
            changes: vec![],
            excluded: HashSet::new(),
            selected: None,
            diff: vec![],
            diff_for: None,
            msg: String::new(),
            busy: None,
            last: None,
            by_path: HashMap::new(),
            loaded: false,
        }
    }
    pub fn included(&self) -> Vec<String> {
        self.changes.iter().filter(|c| !self.excluded.contains(&c.rel)).map(|c| c.rel.clone()).collect()
    }
}

pub enum Confirm {
    Delete(Vec<PathBuf>),
    Restore { rel: String, untracked: bool },
    /// Restaurer toute une passe d'agent (E1).
    BurstRestore(u64),
}

pub struct DragPaths(pub Vec<PathBuf>);

/// Actions collectées pendant le dessin puis appliquées.
pub enum Action {
    Navigate(PathBuf),
    NewTab(PathBuf),
    CloseTab(usize),
    ActivateTab(usize),
    Open(PathBuf),
    Select { path: PathBuf, ctrl: bool, shift: bool },
    SelectOnly(PathBuf),
    ClearSel,
    Copy(bool),
    Paste(PathBuf),
    Trash,
    AskDelete,
    StartRename(PathBuf),
    CommitRename,
    CancelRename,
    NewFolder,
    NewFile,
    CopyPath,
    Pin(PathBuf),
    Unpin(PathBuf),
    Terminal(PathBuf),
    Code(PathBuf),
    Reveal(PathBuf),
    Properties(PathBuf),
    OpenWith(PathBuf),
    Sort(Sort),
    DropInto { paths: Vec<PathBuf>, dst: PathBuf, copy: bool },
    Refresh,
    Up,
    Back,
    Forward,
    ShowInFolder(PathBuf),
    /// Corbeille pour des chemins précis (carte de l'espace).
    TrashPaths(Vec<PathBuf>),
    SelectAll,
    GitSelect(String),
    GitRestoreAsk(String, bool),
    /// Rafales d'agent : afficher le diff d'une passe, demander sa restauration, la rétablir.
    BurstView(u64),
    BurstRestoreAsk(u64),
    BurstRedo(u64),
    GitCommit,
    GitPush,
    GitInit(PathBuf),
    GitRefresh,
    StartSearch,
    Undo,
    Zip(Vec<PathBuf>),
    Unzip(PathBuf),
    ToggleSplit,
    /// Copier (false) ou déplacer (true) la sélection vers l'autre volet.
    ToOtherPane(bool),
    OpenGoto,
    FullView(PathBuf),
    OpenBatchRename,
    ClaudeHere(PathBuf),
    NativeMenu(Vec<PathBuf>),
    OpenInViewer(Vec<PathBuf>),
    Convert(String, Vec<PathBuf>),
    CheckUpdate,
    InstallUpdate,
}

pub enum UpdateMsg {
    Checked { manual: bool, res: Result<Option<socle::update::Release>, String> },
    Installed(Result<(), String>),
}

/// État de la palette « Aller à » (Ctrl+P).
pub struct Goto {
    pub query: String,
    pub sel: usize,
    pub fresh: bool,
    /// Résultats de l'index global pour hits_q (recalculés seulement quand la saisie change).
    pub hits: Vec<(PathBuf, bool, String)>,
    pub hits_q: String,
    /// Dossiers candidats pour `folders_q` (même principe).
    pub folders: Vec<String>,
    pub folders_q: Option<String>,
}

/// Renommage en lot.
pub struct BatchRename {
    pub paths: Vec<PathBuf>,
    /// Motif : {nom} = nom sans extension, {n} / {n:3} = numéro, {ext} = extension, {date} = date de modif.
    pub pattern: String,
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub start: u32,
    pub fresh: bool,
    /// Aperçu (nouveaux noms, conflits) pour les champs `preview_for` : recalculé seulement quand
    /// un champ change, pas à chaque image (regex, et un accès disque par fichier).
    pub preview: Vec<Result<String, String>>,
    pub conflicts: Vec<bool>,
    pub preview_for: Option<(String, String, String, bool, u32)>,
}

enum LoadMsg {
    Dir { tab: u64, generation: u64, res: Result<Vec<Entry>, String>, ms: f32 },
    Search { tab: u64, generation: u64, batch: Vec<Entry>, done: bool },
}

pub struct App {
    pub s: Settings,
    pub pal: Palette,
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_id: u64,
    pub thumbs: Thumbs,
    pub ops: Ops,
    git: Git2,
    watcher: Option<RecommendedWatcher>,
    watch_rx: Receiver<notify::Result<notify::Event>>,
    watched: HashMap<PathBuf, RecursiveMode>,
    load_tx: Sender<LoadMsg>,
    load_rx: Receiver<LoadMsg>,
    pub activity: VecDeque<Activity>,
    pub activity_paused: bool,
    /// Rafales d'agent (E1) : passes, passe affichée dans le volet de diff, son diff.
    pub bursts: crate::bursts::Bursts,
    pub burst_view: Option<u64>,
    pub burst_diff: Option<(u64, Vec<DiffLine>, u32, u32)>,
    pub burst_dirty_at: Option<Instant>,
    pub burst_expanded: HashSet<u64>,
    /// Flux d'activité affiché par passes (sinon fichier par fichier).
    pub activity_by_burst: bool,
    pub repo: Option<Repo>,
    repo_for: Option<PathBuf>,
    git_dirty_at: Option<Instant>,
    diff_dirty: bool,
    pub progress: Option<(&'static str, usize, usize, String)>,
    pub toast: Option<(String, Instant, bool)>,
    pub drives: Vec<fsx::Drive>,
    drives_at: Instant,
    pub confirm: Option<Confirm>,
    pub actions: Vec<Action>,
    /// Début du fichier texte affiché en aperçu (chemin, date, texte), lu sur un thread.
    pub preview_cache: Option<(PathBuf, Option<SystemTime>, Option<Arc<str>>)>,
    preview_pending: Option<(PathBuf, Option<SystemTime>)>,
    preview_tx: Sender<(PathBuf, Option<SystemTime>, Option<Arc<str>>)>,
    preview_rx: Receiver<(PathBuf, Option<SystemTime>, Option<Arc<str>>)>,
    /// Disques relus sur un thread (un disque en veille peut mettre des secondes à répondre).
    drives_tx: Sender<Vec<fsx::Drive>>,
    drives_rx: Receiver<Vec<fsx::Drive>>,
    drives_busy: bool,
    pub show_help: bool,
    typeahead: (String, Instant),
    v_was_down: bool,
    pub quick: Vec<(String, PathBuf)>,
    pub watch_ok: bool,
    pub ctx: egui::Context,
    /// Fichiers à glisser hors de la fenêtre (Bureau, Discord…) : traité par la boucle principale.
    pub drag_out: Option<Vec<PathBuf>>,
    /// Menu clic droit natif de Windows à ouvrir (traité par la boucle principale, qui a la fenêtre).
    pub native_menu: Option<Vec<PathBuf>>,
    /// Pile d'annulation (Ctrl+Z).
    pub undo: Vec<crate::ops::Undo>,
    pub sizes: crate::sizes::Sizes,
    sizes_for: Option<(u64, PathBuf, usize)>,
    pub goto: Option<Goto>,
    /// Double volet : onglet affiché dans l'autre volet, et volet actif à droite ?
    pub split_other: Option<usize>,
    pub focus_right: bool,
    /// Aperçu plein écran (barre d'espace).
    pub fullview: Option<PathBuf>,
    pub batch: Option<BatchRename>,
    /// Exe du lecteur universel, s'il est installé.
    pub viewer: Option<PathBuf>,
    // --- autocomplétion (T4) ---
    pub filter_ac: socle::autocomplete::Autocomplete,
    pub filter_field: socle::ui::AutoField,
    filter_ac_for: Option<(u64, u64, usize)>,
    pub addr_ac: socle::autocomplete::Autocomplete,
    pub addr_field: socle::ui::AutoField,
    pub addr_focused: bool,
    addr_cand_for: String,
    addr_tx: Sender<(String, Vec<String>)>,
    addr_rx: Receiver<(String, Vec<String>)>,
    // --- palette Ctrl+K (T5) ---
    pub palette: socle::ui::Palette,
    /// Repli de l'aperçu rapide : le lecteur n'a pas pu s'ouvrir → aperçu intégré pour ce fichier.
    preview_fallback_tx: Sender<PathBuf>,
    preview_fallback_rx: Receiver<PathBuf>,
    pub index: Option<crate::index::Index>,
    /// Récents partout (E2) : surveillance en direct des disques fixes.
    pub recents_live: Option<crate::recents::Live>,
    saver: socle::settings::Saver,
    settings_hash: u64,
    settings_checked: Instant,
    /// Carte de l'espace de chaque onglet (par id d'onglet).
    pub space_maps: HashMap<u64, crate::spacemap::MapState>,
    recents_seen: u64,
    recents_loaded_at: Instant,
    /// Mise à jour disponible (bandeau dans la barre d'état).
    pub update: Option<socle::update::Release>,
    pub update_busy: Option<&'static str>,
    update_tx: Sender<UpdateMsg>,
    update_rx: Receiver<UpdateMsg>,
    /// Dossiers demandés par une autre instance (boîte aux lettres).
    inbox_rx: Receiver<String>,
    /// Pour l'écoute de l'instance unique (main.rs) : chaque message = un dossier ("" = au premier plan).
    pub inbox_tx: Sender<String>,
    /// Pour l'écoute de l'instance unique (main.rs) : chaque message = un dossier (\
    pub bring_front: bool,
    /// Cette app est-elle l'explorateur par défaut (Win+E, dossiers) ?
    pub is_default: bool,
}

/// Nom de l'app pour le socle (dossiers %APPDATA% / %LOCALAPPDATA%, tube, mutex).
pub const APP: &str = "Explorateur";

/// Nom utilisé en mode test (`--test-hors-ecran` / `SOCLE_TEST=1`) : autre mutex, autre tube,
/// autre cache ; jamais l'instance ni les réglages de l'utilisateur.
pub fn app_name() -> &'static str {
    if socle::testmode::enabled() { "ExplorateurTest" } else { APP }
}

/// Mise à jour depuis les releases GitHub (socle::update).
pub const UPDATER: socle::update::Updater =
    socle::update::Updater { repo: "Desarrh/explorateur", current: env!("CARGO_PKG_VERSION"), asset_suffix: "windows-x64.zip", exe_name: None };

impl Settings {
    /// `%APPDATA%\Explorateur\settings.json` (socle : même fichier qu'avant, écriture atomique).
    /// (en mode test : réglages d'« ExplorateurTest », sans mise à jour automatique)
    pub fn load() -> Settings {
        let mut s: Settings = socle::settings::load(app_name());
        if socle::testmode::enabled() {
            s.auto_update = false;
        }
        s
    }
}

/// Petit alias pour garder le nom `git` libre pour le module.
type Git2 = git::Git;

impl App {
    pub fn new(ctx: &egui::Context, s: Settings) -> App {
        let ctx = ctx.clone();
        let (ptx, prx) = unbounded();
        let (dtx, drx) = unbounded();
        // Socle ≥ 0.1.3 : ni gras ni emoji par défaut (−13 Mo), Segoe UI Symbol gardé.
        theme::install_fonts(&ctx);
        let pal = theme::palette(s.dark);
        theme::apply(&ctx, &pal);
        ctx.all_styles_mut(|st| {
            st.spacing.item_spacing = egui::vec2(8.0, 6.0);
            st.spacing.button_padding = egui::vec2(10.0, 5.0);
            st.interaction.selectable_labels = false;
        });

        let (wtx, watch_rx) = unbounded();
        let c2 = ctx.clone();
        let watcher = notify::recommended_watcher(move |res| {
            let _ = wtx.send(res);
            c2.request_repaint();
        })
        .ok();
        let (load_tx, load_rx) = unbounded();

        let mut quick = Vec::new();
        let mut add = |name: &str, p: Option<PathBuf>| {
            if let Some(p) = p {
                if p.is_dir() {
                    quick.push((name.to_string(), p));
                }
            }
        };
        add("Bureau", dirs::desktop_dir());
        add("Documents", dirs::document_dir());
        add("Téléchargements", dirs::download_dir());
        add("Images", dirs::picture_dir());
        add("Musique", dirs::audio_dir());
        add("Vidéos", dirs::video_dir());
        add("Dossier perso", dirs::home_dir());

        // Dossiers transmis par une 2ᵉ instance (socle::instance, branché dans main.rs) :
        // plus de sondage, le thread du socle dort dans le tube nommé (0 % CPU au repos).
        let (inbox_tx, inbox_rx) = unbounded();

        let (utx, urx) = unbounded();
        let (atx, arx) = unbounded();
        let (filter_hist, addr_hist, palette_recent) = (s.filter_history.clone(), s.addr_history.clone(), s.palette_recent.clone());
        let (pftx, pfrx) = unbounded();
        let mut app = App {
            drag_out: None,
            native_menu: None,
            undo: Vec::new(),
            sizes: crate::sizes::Sizes::new(&ctx),
            sizes_for: None,
            goto: None,
            split_other: None,
            focus_right: false,
            fullview: None,
            batch: None,
            viewer: crate::viewer::find(),
            filter_ac: socle::autocomplete::Autocomplete::new(filter_hist),
            filter_field: Default::default(),
            filter_ac_for: None,
            addr_ac: socle::autocomplete::Autocomplete::new(addr_hist),
            addr_field: Default::default(),
            addr_focused: false,
            addr_cand_for: String::new(),
            addr_tx: atx,
            addr_rx: arx,
            palette: { let mut pal = socle::ui::Palette::default(); pal.recent = palette_recent; pal },
            preview_fallback_tx: pftx,
            preview_fallback_rx: pfrx,
            index: None,
            recents_live: None,
            space_maps: HashMap::new(),
            saver: socle::settings::Saver::new(Duration::from_secs(2)),
            settings_hash: 0,
            settings_checked: Instant::now(),
            recents_seen: 0,
            recents_loaded_at: Instant::now(),
            update: None,
            update_busy: None,
            update_tx: utx,
            update_rx: urx,
            inbox_rx,
            inbox_tx,
            bring_front: false,
            #[cfg(windows)]
            is_default: winx::default_explorer::is_enabled(),
            #[cfg(not(windows))]
            is_default: false,
            watch_ok: watcher.is_some(),
            s,
            pal,
            tabs: vec![],
            active: 0,
            next_id: 1,
            thumbs: Thumbs::new(&ctx),
            ops: Ops::new(&ctx),
            git: git::Git::new(&ctx),
            watcher,
            watch_rx,
            watched: HashMap::new(),
            load_tx,
            load_rx,
            activity: VecDeque::new(),
            activity_paused: false,
            bursts: Default::default(),
            burst_view: None,
            burst_diff: None,
            burst_dirty_at: None,
            burst_expanded: HashSet::new(),
            activity_by_burst: true,
            repo: None,
            repo_for: None,
            git_dirty_at: None,
            diff_dirty: false,
            progress: None,
            toast: None,
            drives: Vec::new(),
            drives_at: Instant::now(),
            confirm: None,
            actions: vec![],
            preview_cache: None,
            preview_pending: None,
            preview_tx: ptx,
            preview_rx: prx,
            drives_tx: dtx,
            drives_rx: drx,
            drives_busy: false,
            show_help: false,
            typeahead: (String::new(), Instant::now()),
            v_was_down: false,
            quick,
            ctx: ctx.clone(),
        };

        let arg = std::env::args().nth(1).map(PathBuf::from).filter(|p| p.is_dir());
        let mut start: Vec<PathBuf> = app.s.tabs.iter().filter(|p| p.is_dir() || crate::recents::is_recents(p)).cloned().collect();
        if let Some(a) = arg {
            start = vec![a];
            app.s.active = 0;
        }
        if start.is_empty() {
            start.push(dirs::home_dir().unwrap_or_else(|| PathBuf::from("C:\\")));
        }
        for p in start {
            app.add_tab(p);
        }
        app.active = app.s.active.min(app.tabs.len() - 1);
        if app.s.global_index {
            app.index = Some(crate::index::Index::start(&ctx));
            app.start_recents();
        }
        app.refresh_drives();
        // Explorateur par défaut actif mais exe déplacé : on réinscrit le bon chemin (hors du fil
        // de l'interface : registre + sonde de conteneur).
        #[cfg(windows)]
        if app.is_default {
            std::thread::spawn(|| winx::default_explorer::refresh_if_moved());
        }
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        if app.s.auto_update && now - app.s.last_update_check > 24 * 3600 {
            app.start_update_check(false);
        }
        // Plantage depuis la dernière fois ? On le signale une fois (socle::crash).
        if let Some(msg) = socle::crash::pending(app_name()) {
            app.toast(format!("L’explorateur s’est fermé brutalement la dernière fois ({msg}) — détails : {}", socle::crash::log_path(APP).display()), true);
        }
        app
    }

    /// Réglages enregistrés peu après chaque changement, sur un thread (socle::settings::Saver) :
    /// un plantage ne perd plus ce qui a changé depuis le lancement. Le changement est repéré par
    /// une empreinte des réglages, calculée au plus toutes les 3 s et seulement quand l'interface
    /// se redessine (rien au repos).
    fn tick_settings(&mut self) {
        if self.settings_checked.elapsed() >= Duration::from_secs(3) {
            self.settings_checked = Instant::now();
            let h = settings_hash(self.settings_for_save());
            if h != self.settings_hash {
                self.settings_hash = h;
                self.saver.touch();
            }
        }
        if let Some(d) = self.saver.tick(app_name(), &self.s) {
            self.ctx.request_repaint_after(d);
        }
        if let Some(e) = self.saver.take_error() {
            self.toast(format!("Réglages non enregistrés : {e}"), true);
        }
    }

    /// Fermeture : écrit les réglages (taille de fenêtre comprise) et attend la fin de l'écriture.
    pub fn flush_settings(&mut self, window: Option<(f32, f32)>) {
        self.settings_for_save();
        if window.is_some() {
            self.s.window = window;
        }
        self.saver.touch();
        let _ = self.saver.flush(app_name(), &self.s);
    }

    pub fn settings_for_save(&mut self) -> &Settings {
        self.s.tabs = self.tabs.iter().map(|t| t.path.clone()).collect();
        self.s.active = self.active;
        self.s.palette_recent = self.palette.recent.clone();
        &self.s
    }

    /// Dossiers les plus visités (candidats de l'adresse et de « Aller à »).
    fn top_visits(&self, n: usize) -> Vec<String> {
        let mut v: Vec<(&String, &(u32, i64))> = self.s.visits.iter().collect();
        v.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(b.1.1.cmp(&a.1.1)));
        v.into_iter().take(n).map(|(k, _)| k.clone()).collect()
    }

    /// Candidats de l'autocomplétion de la barre d'adresse : sous-dossiers du dossier parent de
    /// ce qui est tapé (lus sur un thread, jamais sur le fil de l'interface) + dossiers fréquents.
    pub fn refresh_address_candidates(&mut self, text: &str) {
        while let Ok((key, mut list)) = self.addr_rx.try_recv() {
            if key == self.addr_cand_for {
                list.extend(self.top_visits(300));
                self.addr_ac.set_candidates(list);
            }
        }
        let t = text.trim().trim_matches('"');
        let parent = match t.rfind('\\') {
            Some(i) => t[..=i].to_string(),
            None => String::new(),
        };
        if parent == self.addr_cand_for && !self.addr_ac.candidates().is_empty() {
            return;
        }
        self.addr_cand_for = parent.clone();
        // Tout de suite : les dossiers fréquents (jamais de liste vide pendant la lecture).
        let visits = self.top_visits(300);
        self.addr_ac.set_candidates(visits);
        if parent.is_empty() {
            return;
        }
        let (tx, ctx) = (self.addr_tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&parent) {
                for e in rd.flatten().take(5000) {
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        out.push(format!("{}{}", parent, e.file_name().to_string_lossy()));
                    }
                }
            }
            let _ = tx.send((parent, out));
            ctx.request_repaint();
        });
    }

    /// Candidats du filtre : les noms du dossier affiché (mis à jour quand il change).
    fn refresh_filter_candidates(&mut self) {
        let t = &self.tabs[self.active];
        if t.loading {
            return;
        }
        let key = (t.id, t.generation, t.entries.len());
        if self.filter_ac_for == Some(key) {
            return;
        }
        self.filter_ac_for = Some(key);
        let names: Vec<String> = t.entries.iter().take(20_000).map(|e| e.name.clone()).collect();
        self.filter_ac.set_candidates(names);
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.s.dark = dark;
        self.pal = theme::palette(dark);
        theme::apply(&self.ctx, &self.pal);
    }

    pub fn toast(&mut self, msg: impl Into<String>, err: bool) {
        self.toast = Some((msg.into(), Instant::now(), err));
    }

    fn add_tab(&mut self, path: PathBuf) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.record_visit(&path);
        self.tabs.push(Tab::new(id, path));
        let i = self.tabs.len() - 1;
        self.start_load(i);
        i
    }

    pub fn start_load(&mut self, ti: usize) {
        let tab = &mut self.tabs[ti];
        tab.generation += 1;
        tab.loading = true;
        let (id, generation, path) = (tab.id, tab.generation, tab.path.clone());
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        if crate::recents::is_recents(&path) {
            // Onglet « Récents » : index global + surveillance en direct, relus sur le disque.
            let index = self.index.as_ref().map(|i| i.data.clone());
            let live = self.recents_live.as_ref().map(|l| l.map());
            self.recents_seen = self.recents_live.as_ref().map(|l| l.generation.load(Ordering::Relaxed)).unwrap_or(0);
            self.recents_loaded_at = Instant::now();
            std::thread::spawn(move || {
                let t0 = Instant::now();
                let v = crate::recents::collect(index.as_deref(), live.as_deref(), crate::recents::LIMIT);
                let ms = t0.elapsed().as_secs_f32() * 1000.0;
                let _ = tx.send(LoadMsg::Dir { tab: id, generation, res: Ok(v), ms });
                ctx.request_repaint();
            });
            return;
        }
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let res = fsx::list_dir(&path).map_err(|e| e.to_string());
            let ms = t0.elapsed().as_secs_f32() * 1000.0;
            let _ = tx.send(LoadMsg::Dir { tab: id, generation, res, ms });
            ctx.request_repaint();
        });
    }

    /// Démarre la surveillance des disques pour les « Récents partout ».
    pub fn start_recents(&mut self) {
        if self.recents_live.is_none() {
            self.recents_live = Some(crate::recents::Live::start(&crate::index::fixed_roots()));
        }
    }

    /// Onglet « Récents » affiché et nouveaux fichiers vus : rechargement (au plus toutes les 2 s).
    fn tick_recents(&mut self) {
        let Some(g) = self.recents_live.as_ref().map(|l| l.generation.load(Ordering::Relaxed)) else { return };
        if g == self.recents_seen {
            return;
        }
        let showing: Vec<usize> = (0..self.tabs.len())
            .filter(|&i| crate::recents::is_recents(&self.tabs[i].path) && (i == self.active || Some(i) == self.split_other))
            .collect();
        if showing.is_empty() {
            return;
        }
        let wait = Duration::from_secs(2).saturating_sub(self.recents_loaded_at.elapsed());
        if !wait.is_zero() {
            self.ctx.request_repaint_after(wait);
            return;
        }
        for i in showing {
            self.start_load(i);
        }
    }

    fn navigate(&mut self, path: PathBuf) {
        let tab = self.tab_mut();
        if tab.path == path {
            return;
        }
        let old = std::mem::replace(&mut tab.path, path);
        tab.back.push(old);
        tab.fwd.clear();
        self.reset_tab_view();
    }

    /// Mémorise la visite d'un dossier (pour « Aller à », classé par fréquence et récence).
    pub fn record_visit(&mut self, p: &Path) {
        if crate::recents::is_recents(p) {
            return;
        }
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let e = self.s.visits.entry(p.to_string_lossy().into_owned()).or_insert((0, now));
        e.0 = e.0.saturating_add(1);
        e.1 = now;
        if self.s.visits.len() > 2000 {
            // on oublie les dossiers les plus anciens
            let mut v: Vec<(String, i64)> = self.s.visits.iter().map(|(k, v)| (k.clone(), v.1)).collect();
            v.sort_by_key(|x| x.1);
            for (k, _) in v.into_iter().take(500) {
                self.s.visits.remove(&k);
            }
        }
    }

    fn reset_tab_view(&mut self) {
        let p = self.tab().path.clone();
        self.record_visit(&p);
        let tab = self.tab_mut();
        tab.entries.clear();
        tab.view.clear();
        tab.selected.clear();
        tab.cursor = None;
        tab.anchor = None;
        tab.filter.clear();
        tab.recursive = false;
        if let Some(s) = tab.search.take() {
            s.cancel.store(true, Ordering::Relaxed);
        }
        tab.marks.clear();
        tab.renaming = None;
        tab.address = None;
        tab.scroll_offset = 0.0;
        tab.scroll_to = Some(0);
        let i = self.active;
        self.start_load(i);
    }

    pub fn recompute_view(&mut self, ti: usize) {
        let show_hidden = self.s.show_hidden;
        let (sort, asc) = (self.s.sort, self.s.asc);
        let dir_sizes = &self.sizes.map;
        let tab = &mut self.tabs[ti];
        if !tab.view_dirty {
            return;
        }
        tab.view_dirty = false;
        let searching = tab.search.is_some();
        let needle = tab.filter.trim().to_lowercase();
        let src = tab.source();
        let mut scored: Vec<(usize, i32)> = src
            .iter()
            .enumerate()
            .filter(|(_, e)| show_hidden || !e.hidden)
            .filter_map(|(i, e)| {
                if searching || needle.is_empty() {
                    Some((i, 0))
                } else {
                    socle::fuzzy::score(&e.name, &needle).map(|s| (i, s))
                }
            })
            .collect();
        let by_score = !needle.is_empty() && !searching;
        // Onglet « Récents » : toujours du plus récent au plus ancien.
        let (sort, asc) = if crate::recents::is_recents(&tab.path) && !by_score { (Sort::Modified, false) } else { (sort, asc) };
        scored.sort_by(|a, b| {
            let (ea, eb) = (&src[a.0], &src[b.0]);
            eb.is_dir.cmp(&ea.is_dir).then_with(|| {
                if by_score {
                    return b.1.cmp(&a.1).then_with(|| natural_cmp(&ea.lower, &eb.lower));
                }
                let o = match sort {
                    Sort::Name => natural_cmp(&ea.lower, &eb.lower),
                    Sort::Modified => ea.modified.cmp(&eb.modified),
                    Sort::Kind => ea.ext.cmp(&eb.ext).then_with(|| natural_cmp(&ea.lower, &eb.lower)),
                    Sort::Size => {
                        let sz = |e: &Entry| if e.is_dir { dir_sizes.get(&e.path).copied().unwrap_or(0) } else { e.size };
                        sz(ea).cmp(&sz(eb))
                    }
                };
                if asc { o } else { o.reverse() }
            })
        });
        tab.view = scored.into_iter().map(|(i, _)| i).collect();
    }

    /// Met à jour un chemin dans les onglets concernés (événement live ou opération locale).
    fn apply_path(&mut self, p: &Path, mark: Option<Act>) -> Option<Act> {
        let exists = Entry::stat(p);
        let mut result = None;
        for tab in &mut self.tabs {
            if p.parent() != Some(tab.path.as_path()) {
                continue;
            }
            let pos = tab.entries.iter().position(|e| e.path == p);
            let kind = match (pos, &exists) {
                (Some(i), Some(e)) => {
                    tab.entries[i] = e.clone();
                    Act::Modified
                }
                (None, Some(e)) => {
                    tab.entries.push(e.clone());
                    Act::Created
                }
                (Some(i), None) => {
                    tab.entries.remove(i);
                    tab.selected.remove(p);
                    Act::Removed
                }
                (None, None) => continue,
            };
            tab.view_dirty = true;
            if let Some(m) = mark {
                let k = if m == Act::Created || kind == Act::Created { Act::Created } else { kind };
                tab.marks.insert(p.to_path_buf(), Mark { at: Instant::now(), kind: k });
                result = Some(k);
            }
        }
        result
    }

    fn sync_watches(&mut self) {
        let Some(w) = self.watcher.as_mut() else { return };
        let mut want: HashMap<PathBuf, RecursiveMode> = HashMap::new();
        for t in self.tabs.iter().filter(|t| !crate::recents::is_recents(&t.path)) {
            want.insert(t.path.clone(), RecursiveMode::NonRecursive);
        }
        if self.s.dev {
            if let Some(r) = &self.repo {
                want.insert(r.root.clone(), RecursiveMode::Recursive);
            }
        }
        if want == self.watched {
            return;
        }
        for (p, m) in &self.watched {
            if want.get(p) != Some(m) {
                let _ = w.unwatch(p);
            }
        }
        let mut ok = HashMap::new();
        for (p, m) in want {
            if self.watched.get(&p) == Some(&m) || w.watch(&p, m).is_ok() {
                ok.insert(p, m);
            }
        }
        self.watched = ok;
    }

    fn process_watch(&mut self) {
        let mut evs: Vec<(PathBuf, Act)> = Vec::new();
        let mut seen: HashMap<PathBuf, usize> = HashMap::new();
        while let Ok(res) = self.watch_rx.try_recv() {
            let Ok(ev) = res else { continue };
            let kind = match ev.kind {
                EventKind::Create(_) => Act::Created,
                EventKind::Remove(_) => Act::Removed,
                EventKind::Access(_) => continue,
                _ => Act::Modified,
            };
            for p in ev.paths {
                match seen.get(&p) {
                    Some(_) => {}
                    None => {
                        seen.insert(p.clone(), evs.len());
                        evs.push((p, kind));
                    }
                }
            }
        }
        if evs.is_empty() {
            return;
        }
        let tab_dirs: HashSet<PathBuf> = self.tabs.iter().map(|t| t.path.clone()).collect();
        let root = self.repo.as_ref().map(|r| r.root.clone());
        let gi = self.repo.as_ref().and_then(|r| r.ignore.clone());
        let sel_abs = self.repo.as_ref().and_then(|r| r.selected.as_ref().map(|s| r.root.join(s)));
        for (p, first) in evs {
            let in_tab = p.parent().map(|d| tab_dirs.contains(d)).unwrap_or(false);
            let comps: Vec<String> = p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
            if comps.iter().any(|c| c == ".git") {
                let tail = comps.iter().skip_while(|c| *c != ".git").skip(1).cloned().collect::<Vec<_>>();
                if tail.first().map(|s| s == "head" || s == "refs" || s == "merge_head").unwrap_or(false) {
                    self.git_dirty_at = Some(Instant::now());
                }
                if !in_tab {
                    continue;
                }
            }
            let noisy = comps.iter().any(|c| NOISY.contains(&c.as_str()));
            if noisy && !in_tab {
                continue;
            }
            // Fichier ignoré par git (build, cache…) : écarté avant tout accès au disque, sauf s'il
            // est dans un dossier affiché. (Règles « dossier/ » : revérifiées plus bas pour un dossier.)
            let in_repo = root.as_ref().map(|r| p.starts_with(r)).unwrap_or(false);
            let ignored_path = in_repo && gi.as_ref().map(|g| g.matched_path_or_any_parents(&p, false).is_ignore()).unwrap_or(false);
            if ignored_path && !in_tab {
                continue;
            }
            let exists = p.exists();
            let fin = match (exists, first) {
                (false, Act::Created) => {
                    // fichier temporaire créé puis supprimé (écriture atomique) : on ignore
                    self.apply_path(&p, None);
                    continue;
                }
                (false, _) => Act::Removed,
                (true, Act::Created) => Act::Created,
                (true, _) => Act::Modified,
            };
            let applied = self.apply_path(&p, Some(fin)).unwrap_or(fin);
            self.thumbs.invalidate(&p);
            if self.sizes.invalidate(&p) {
                self.sizes_for = None;
            }
            if self.preview_cache.as_ref().map(|c| c.0 == p).unwrap_or(false) {
                self.preview_cache = None;
            }
            // Ignoré par git : ni dans le flux d'activité, ni dans le statut.
            let git_ignored = ignored_path
                || (in_repo
                    && exists
                    && p.is_dir()
                    && gi.as_ref().map(|g| g.matched_path_or_any_parents(&p, true).is_ignore()).unwrap_or(false));
            if git_ignored && !in_tab {
                continue;
            }
            if in_repo && !git_ignored {
                self.git_dirty_at = Some(Instant::now());
            }
            if sel_abs.as_ref() == Some(&p) {
                self.diff_dirty = true;
            }
            if exists && p.is_dir() && applied == Act::Modified {
                continue;
            }
            if in_repo && p.file_name().map(|n| n == ".gitignore").unwrap_or(false) {
                if let Some(r) = self.repo.as_mut() {
                    let fresh = Repo::new(r.root.clone());
                    r.ignore = fresh.ignore;
                }
            }
            if (in_repo || in_tab) && !git_ignored && !self.activity_paused {
                let label = match &root {
                    Some(r) if in_repo => p.strip_prefix(r).map(|x| x.to_string_lossy().replace('\\', "/")).unwrap_or_default(),
                    _ => p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                };
                if let (Some(r), true) = (&root, in_repo) {
                    self.bursts.record(r, label.clone(), applied);
                    let viewing_open = self.burst_view.and_then(|id| self.bursts.get(id)).map(|b| b.open()).unwrap_or(false);
                    if viewing_open {
                        self.burst_dirty_at.get_or_insert_with(Instant::now);
                    }
                }
                if let Some(front) = self.activity.front_mut() {
                    if front.path == p && front.at.elapsed() < Duration::from_secs(4) {
                        front.count += 1;
                        front.at = Instant::now();
                        front.wall = SystemTime::now();
                        if applied == Act::Removed || front.kind != Act::Created {
                            front.kind = applied;
                        }
                        continue;
                    }
                }
                self.activity.push_front(Activity {
                    path: p.clone(),
                    label,
                    kind: applied,
                    at: Instant::now(),
                    wall: SystemTime::now(),
                    count: 1,
                });
                self.activity.truncate(300);
            }
        }
    }

    fn process_loads(&mut self) {
        while let Ok(msg) = self.load_rx.try_recv() {
            match msg {
                LoadMsg::Dir { tab, generation, res, ms } => {
                    let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab && t.generation == generation) else { continue };
                    t.loading = false;
                    t.load_ms = ms;
                    match res {
                        Ok(v) => {
                            t.entries = v;
                            t.error = None;
                        }
                        Err(e) => {
                            t.entries.clear();
                            t.error = Some(e);
                        }
                    }
                    let keep: HashSet<PathBuf> = t.entries.iter().map(|e| e.path.clone()).collect();
                    t.selected.retain(|p| keep.contains(p));
                    t.view_dirty = true;
                    if let Some(f) = t.focus_after_load.take() {
                        t.selected = [f.clone()].into_iter().collect();
                        t.cursor = Some(f.clone());
                        t.anchor = Some(f);
                        t.scroll_to = None; // recalculé après le tri
                        t.focus_after_load = Some(PathBuf::new()); // marqueur : défiler vers le curseur
                    }
                }
                LoadMsg::Search { tab, generation, batch, done } => {
                    let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { continue };
                    let Some(s) = t.search.as_mut().filter(|s| s.generation == generation) else { continue };
                    s.results.extend(batch);
                    if done {
                        s.running = false;
                    }
                    t.view_dirty = true;
                }
            }
        }
    }

    fn process_ops(&mut self) {
        while let Ok(ev) = self.ops.rx.try_recv() {
            match ev {
                OpEvent::Progress { label, done, total, current } => self.progress = Some((label, done, total, current)),
                OpEvent::Finished { label, errors, touched, undo } => {
                    self.progress = None;
                    for p in &touched {
                        self.apply_path(p, None);
                        self.sizes.invalidate(p);
                    }
                    if let Some(u) = undo {
                        self.undo.push(u);
                        if self.undo.len() > 100 {
                            self.undo.remove(0);
                        }
                    }
                    self.sizes_for = None; // recalculer les tailles affichées
                    if errors.is_empty() {
                        self.toast(format!("{label} terminée"), false);
                    } else {
                        self.toast(format!("{label} : {}", errors.join(" · ")), true);
                    }
                    if self.repo.is_some() {
                        self.git_dirty_at = Some(Instant::now());
                    }
                }
            }
        }
    }

    fn process_git(&mut self) {
        // Détection du dépôt quand le dossier actif change
        let cur = self.tab().path.clone();
        if self.repo_for.as_ref() != Some(&cur) {
            self.repo_for = Some(cur.clone());
            let inside = self.repo.as_ref().map(|r| cur.starts_with(&r.root)).unwrap_or(false);
            if !inside {
                self.git.send(git::Req::Detect(cur));
            }
        }
        if let Some(t) = self.git_dirty_at {
            if t.elapsed() > Duration::from_millis(300) {
                self.git_dirty_at = None;
                if let Some(r) = &self.repo {
                    self.git.send(git::Req::Status(r.root.clone()));
                }
            }
        }
        if self.diff_dirty {
            self.diff_dirty = false;
            self.request_diff();
        }
        while let Ok(resp) = self.git.rx.try_recv() {
            match resp {
                git::Resp::Detected { path, root } => {
                    if Some(&path) != self.repo_for.as_ref() {
                        continue;
                    }
                    match root {
                        Some(root) => {
                            if self.repo.as_ref().map(|r| r.root != root).unwrap_or(true) {
                                if let Some(q) = self.bursts.watch_repo(&root) {
                                    self.git.send(git::Req::Snapshot { root: q.root, base: q.base, files: q.files, why: 0 });
                                }
                                self.repo = Some(Repo::new(root.clone()));
                                self.git.send(git::Req::Status(root));
                            }
                        }
                        None => self.repo = None,
                    }
                }
                git::Resp::Status { root, branch, ahead, changes } => {
                    let Some(r) = self.repo.as_mut().filter(|r| r.root == root) else { continue };
                    r.branch = branch;
                    r.ahead = ahead;
                    r.by_path = changes
                        .iter()
                        .map(|c| (root.join(c.rel.replace('/', "\\")), c.kind))
                        .collect();
                    let rels: HashSet<String> = changes.iter().map(|c| c.rel.clone()).collect();
                    r.excluded.retain(|e| rels.contains(e));
                    r.changes = changes;
                    r.loaded = true;
                    if r.selected.as_ref().map(|s| !rels.contains(s)).unwrap_or(false) {
                        r.selected = None;
                        r.diff.clear();
                        r.diff_for = None;
                    }
                    if r.selected.is_none() {
                        if let Some(first) = r.changes.first() {
                            r.selected = Some(first.rel.clone());
                        }
                    }
                    self.diff_dirty = true;
                }
                git::Resp::Diff { rel, lines } => {
                    if let Some(r) = self.repo.as_mut() {
                        if r.selected.as_ref() == Some(&rel) {
                            r.diff = lines;
                            r.diff_for = Some(rel);
                        }
                    }
                }
                git::Resp::Snapshot { root, why, res } => {
                    let w = if why == 0 { crate::bursts::SnapWhy::Base } else { crate::bursts::SnapWhy::Close(why) };
                    if let Some(q) = self.bursts.on_snapshot(&root, w, res) {
                        self.git.send(git::Req::Snapshot { root: q.root, base: q.base, files: q.files, why: 0 });
                    }
                    if why != 0 && self.burst_view == Some(why) {
                        self.request_burst_diff(why);
                    }
                }
                git::Resp::BurstDiff { id, lines, plus, minus } => {
                    if self.burst_view == Some(id) {
                        self.burst_diff = Some((id, lines, plus, minus));
                    }
                }
                git::Resp::BurstRestored { root, id, redo, res } => {
                    let n = self.bursts.get(id).map(|b| b.restorable().len()).unwrap_or(0);
                    if let Some(b) = self.bursts.get_mut(id) {
                        b.busy = false;
                    }
                    match res {
                        Ok(r) => {
                            let kept = r.kept;
                            if let Some(b) = self.bursts.get_mut(id) {
                                b.undo = if redo { None } else { Some(r.undo) };
                            }
                            self.bursts.add_raw(&root, r.raw);
                            self.bursts.set_snapshot(&root, r.post);
                            let mut msg = if redo { format!("Passe rétablie ({n} fichier(s))") } else { format!("Passe annulée : {n} fichier(s) remis comme avant") };
                            if kept > 0 {
                                msg.push_str(&format!(" ; {kept} laissé(s) tel(s) quel(s) (état d’avant inconnu)"));
                            }
                            self.toast(msg, false);
                        }
                        Err(e) => {
                            // État du disque incertain : nouvel instantané de base.
                            self.toast(format!("Restauration de la passe impossible : {e}"), true);
                            self.git.send(git::Req::Snapshot { root: root.clone(), base: None, files: vec![], why: 0 });
                        }
                    }
                    self.git.send(git::Req::Status(root));
                }
                git::Resp::Done { what, ok, msg, root } => {
                    if let Some(r) = self.repo.as_mut() {
                        r.busy = None;
                        if what == "Commit" && ok {
                            r.msg.clear();
                        }
                        let text = if msg.is_empty() { format!("{what} : OK") } else { format!("{what} : {msg}") };
                        r.last = Some((text.clone(), ok));
                    }
                    if what == "git init" {
                        self.repo_for = None;
                        self.repo = None;
                    }
                    self.toast(if ok { format!("{what} réussi") } else { format!("{what} : {msg}") }, !ok);
                    self.git.send(git::Req::Status(root));
                }
            }
        }
    }

    /// Relit les disques (espace libre) sur un thread.
    pub fn refresh_drives(&mut self) {
        if self.drives_busy {
            return;
        }
        self.drives_busy = true;
        let (tx, ctx) = (self.drives_tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(fsx::drives());
            ctx.request_repaint();
        });
    }

    /// Début d'un fichier texte pour l'aperçu : depuis le cache, sinon lu sur un thread (`None`
    /// en attendant ; l'interface se redessine à l'arrivée).
    pub fn text_preview(&mut self, path: &Path, modified: Option<SystemTime>) -> Option<Arc<str>> {
        if let Some((p, m, t)) = &self.preview_cache {
            if p == path && *m == modified {
                return t.clone();
            }
        }
        let key = (path.to_path_buf(), modified);
        if self.preview_pending.as_ref() != Some(&key) {
            self.preview_pending = Some(key);
            let (tx, ctx, p) = (self.preview_tx.clone(), self.ctx.clone(), path.to_path_buf());
            std::thread::spawn(move || {
                let t = fsx::read_text_preview(&p, 64 * 1024).map(Arc::from);
                let _ = tx.send((p, modified, t));
                ctx.request_repaint();
            });
        }
        None
    }

    /// Demande le diff cumulé d'une passe (avant → après, ou avant → disque si elle est en cours).
    pub fn request_burst_diff(&mut self, id: u64) {
        let Some(b) = self.bursts.get(id) else { return };
        let Some(before) = b.before.clone() else {
            self.burst_diff = Some((id, vec![], 0, 0));
            return;
        };
        let req = git::Req::BurstDiff { root: b.root.clone(), id, before, after: b.after.clone(), files: b.files.keys().cloned().collect() };
        self.git.send(req);
    }

    fn tick_bursts(&mut self) {
        for q in self.bursts.tick() {
            let why = match q.why {
                crate::bursts::SnapWhy::Close(id) => id,
                crate::bursts::SnapWhy::Base => 0,
            };
            self.git.send(git::Req::Snapshot { root: q.root, base: q.base, files: q.files, why });
        }
        if let Some(d) = self.bursts.next_close_in() {
            self.ctx.request_repaint_after(d + Duration::from_millis(50));
        }
        if let Some(t) = self.burst_dirty_at {
            if t.elapsed() > Duration::from_millis(500) {
                self.burst_dirty_at = None;
                if let Some(id) = self.burst_view {
                    self.request_burst_diff(id);
                }
            } else {
                self.ctx.request_repaint_after(Duration::from_millis(520));
            }
        }
    }

    fn request_diff(&mut self) {
        if let Some(r) = &self.repo {
            if let Some(sel) = &r.selected {
                let untracked = r.changes.iter().any(|c| &c.rel == sel && c.kind == git::Kind::Untracked);
                self.git.send(git::Req::Diff { root: r.root.clone(), rel: sel.clone(), untracked });
            }
        }
    }

    fn start_search(&mut self) {
        let tab = self.tab_mut();
        if let Some(s) = tab.search.take() {
            s.cancel.store(true, Ordering::Relaxed);
        }
        let needle = tab.filter.trim().to_lowercase();
        tab.view_dirty = true;
        if !tab.recursive || needle.is_empty() {
            return;
        }
        let generation = tab.generation.wrapping_add(1_000_000 + tab.search.as_ref().map(|s| s.generation).unwrap_or(0));
        let cancel = Arc::new(AtomicBool::new(false));
        tab.search = Some(Search { generation, results: vec![], running: true, cancel: cancel.clone() });
        let (id, root) = (tab.id, tab.path.clone());
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            // Parcours parallèle (celui de ripgrep) qui respecte .gitignore, .git/info/exclude
            // et .ignore dans les dépôts git ; ailleurs, seuls les dossiers « bruyants » sont sautés.
            let (etx, erx) = unbounded::<Entry>();
            let found = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let walker = ignore::WalkBuilder::new(&root)
                .hidden(false)
                .git_ignore(true)
                .git_exclude(true)
                .git_global(false)
                .require_git(true)
                .follow_links(false)
                .threads(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8).min(12))
                .filter_entry(|e| {
                    let n = e.file_name().to_string_lossy().to_lowercase();
                    !(e.depth() > 0 && e.file_type().map(|t| t.is_dir()).unwrap_or(false) && (n == ".git" || NOISY.contains(&n.as_str())))
                })
                .build_parallel();
            let collector = {
                let (tx, ctx, root) = (tx.clone(), ctx.clone(), root.clone());
                std::thread::spawn(move || {
                    let mut batch = Vec::new();
                    let mut last = Instant::now();
                    loop {
                        match erx.recv_timeout(Duration::from_millis(100)) {
                            Ok(mut e) => {
                                e.display = e.path.strip_prefix(&root).ok().map(|r| r.to_string_lossy().into_owned());
                                batch.push(e);
                            }
                            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                            Err(_) => break,
                        }
                        if !batch.is_empty() && (batch.len() > 300 || last.elapsed() > Duration::from_millis(120)) {
                            let _ = tx.send(LoadMsg::Search { tab: id, generation, batch: std::mem::take(&mut batch), done: false });
                            ctx.request_repaint();
                            last = Instant::now();
                        }
                    }
                    let _ = tx.send(LoadMsg::Search { tab: id, generation, batch, done: true });
                    ctx.request_repaint();
                })
            };
            walker.run(|| {
                let (etx, cancel, found, needle) = (etx.clone(), cancel.clone(), found.clone(), needle.clone());
                Box::new(move |res| {
                    use ignore::WalkState;
                    if cancel.load(Ordering::Relaxed) || found.load(Ordering::Relaxed) > 20_000 {
                        return WalkState::Quit;
                    }
                    let Ok(de) = res else { return WalkState::Continue };
                    if de.depth() == 0 {
                        return WalkState::Continue;
                    }
                    if de.file_name().to_string_lossy().to_lowercase().contains(&needle) {
                        if let Ok(meta) = de.metadata() {
                            found.fetch_add(1, Ordering::Relaxed);
                            let _ = etx.send(Entry::from_meta(de.path().to_path_buf(), &meta));
                        }
                    }
                    WalkState::Continue
                })
            });
            drop(etx);
            let _ = collector.join();
        });
    }

    /// Appelé à chaque image, avant le dessin.
    pub fn tick(&mut self) {
        self.process_updates();
        self.refresh_filter_candidates();
        // Le lecteur n'a pas répondu à un aperçu rapide : aperçu intégré, sans message d'erreur.
        while let Ok(p) = self.preview_fallback_rx.try_recv() {
            self.fullview = Some(p);
        }
        while let Ok(req) = self.inbox_rx.try_recv() {
            // (un disque peut arriver en « C:" » depuis une ancienne inscription du registre)
            let p = PathBuf::from(req.trim().trim_end_matches('"'));
            let p = if p.is_dir() { p } else { dirs::home_dir().unwrap_or_else(|| PathBuf::from("C:\\")) };
            let i = self.add_tab(p);
            self.active = i;
            self.bring_front = true;
        }
        self.thumbs.poll(&self.ctx);
        self.process_loads();
        self.process_watch();
        self.process_ops();
        self.process_git();
        self.tick_settings();
        self.tick_bursts();
        self.tick_recents();
        self.sync_watches();
        while let Ok(d) = self.drives_rx.try_recv() {
            self.drives = d;
            self.drives_busy = false;
        }
        if self.drives_at.elapsed() > Duration::from_secs(10) {
            self.drives_at = Instant::now();
            self.refresh_drives();
        }
        while let Ok((p, m, text)) = self.preview_rx.try_recv() {
            if self.preview_pending.as_ref() == Some(&(p.clone(), m)) {
                self.preview_pending = None;
            }
            self.preview_cache = Some((p, m, text));
        }
        // Taille des dossiers du dossier affiché (en arrière-plan) ; la carte de l'espace en a besoin aussi.
        if self.s.folder_sizes || (self.s.space_map && !self.s.dev) {
            if self.sizes.poll() && self.s.sort == Sort::Size {
                for t in &mut self.tabs {
                    t.view_dirty = true;
                }
            }
            let t = self.tab();
            let key = (t.id, t.path.clone(), t.entries.len());
            if !t.loading && self.sizes_for.as_ref() != Some(&key) {
                let dirs: Vec<PathBuf> = t.entries.iter().filter(|e| e.is_dir).map(|e| e.path.clone()).collect();
                self.sizes.focus(dirs);
                self.sizes_for = Some(key);
            }
        }
        for i in 0..self.tabs.len() {
            self.recompute_view(i);
            let t = &mut self.tabs[i];
            if t.focus_after_load.as_ref().map(|p| p.as_os_str().is_empty()).unwrap_or(false) {
                t.focus_after_load = None;
                if let Some(c) = t.cursor.clone() {
                    t.scroll_to = t.view_index_of(&c);
                }
            }
            t.marks.retain(|_, m| m.at.elapsed().as_secs_f32() < MARK_SECS + 1.0);
        }
        if let Some((_, at, _)) = &self.toast {
            if at.elapsed() > Duration::from_secs(5) {
                self.toast = None;
            }
        }
        // Rafraîchir les « il y a N s » et l'estompage des surlignages
        let live = self.tabs.iter().any(|t| !t.marks.is_empty());
        if live || self.toast.is_some() || self.progress.is_some() {
            self.ctx.request_repaint_after(Duration::from_millis(120));
        } else if self.s.dev {
            // « il y a N s » : chaque seconde pendant la 1re minute, puis toutes les 30 s pendant une
            // heure, puis plus rien (pas de redessin sans fin au repos).
            match self.activity.front().map(|a| a.at.elapsed()) {
                Some(e) if e < Duration::from_secs(60) => self.ctx.request_repaint_after(Duration::from_secs(1)),
                Some(e) if e < Duration::from_secs(3600) => self.ctx.request_repaint_after(Duration::from_secs(30)),
                _ => {}
            }
        }
        // Dropped files from other apps → copy here
        let dropped: Vec<PathBuf> =
            self.ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        // (on ignore ce qui vient du dossier courant : c'est un glisser-sortir revenu dans la fenêtre)
        let dst = self.tab().path.clone();
        let dropped: Vec<PathBuf> = dropped.into_iter().filter(|p| p.parent() != Some(dst.as_path())).collect();
        if !dropped.is_empty() {
            self.ops.submit(Op::Copy { srcs: dropped, dst });
        }
        // Glisser hors de la fenêtre : on passe la main au glisser-déposer de Windows.
        if let Some(payload) = egui::DragAndDrop::payload::<DragPaths>(&self.ctx) {
            let outside = self.ctx.input(|i| {
                let r = i.viewport_rect().shrink(2.0);
                i.pointer.primary_down() && i.pointer.latest_pos().map(|p| !r.contains(p)).unwrap_or(false)
            });
            if outside {
                self.drag_out = Some(payload.0.clone());
                egui::DragAndDrop::clear_payload(&self.ctx);
            }
        }
    }

    pub fn handle_keys(&mut self) {
        let ctx = self.ctx.clone();
        // Fenêtres modales : elles gèrent leurs propres touches.
        if self.goto.is_some() || self.batch.is_some() || self.palette.is_open() || self.show_help {
            return;
        }
        // Ctrl+1 / Ctrl+2 : liste / grille, par la touche physique (marche aussi en AZERTY).
        if !ctx.egui_wants_keyboard_input() {
            match socle::keys::consume_ctrl_digit(&ctx) {
                Some(1) => (self.s.grid, self.s.space_map) = (false, false),
                Some(2) => (self.s.grid, self.s.space_map) = (true, false),
                Some(3) => self.s.space_map = true,
                _ => {}
            }
        }
        if let Some(cur) = self.fullview.clone() {
            use egui::{Key, Modifiers as M};
            let (close, step) = ctx.input_mut(|i| {
                let close = i.consume_key(M::NONE, Key::Escape) || i.consume_key(M::NONE, Key::Space);
                let step = if i.consume_key(M::NONE, Key::ArrowRight) || i.consume_key(M::NONE, Key::ArrowDown) {
                    1
                } else if i.consume_key(M::NONE, Key::ArrowLeft) || i.consume_key(M::NONE, Key::ArrowUp) {
                    -1
                } else {
                    0
                };
                (close, step)
            });
            if close {
                self.fullview = None;
            } else if step != 0 {
                let t = self.tab();
                if let Some(idx) = t.view_index_of(&cur) {
                    let n = t.view.len() as isize;
                    let next = ((idx as isize + step).rem_euclid(n)) as usize;
                    if let Some(e) = t.entry(next) {
                        let p = e.path.clone();
                        self.fullview = Some(p.clone());
                        self.apply(Action::Select { path: p, ctrl: false, shift: false });
                    }
                }
            }
            return;
        }
        let typing = ctx.egui_wants_keyboard_input() || self.tab().renaming.is_some();
        let mut acts = Vec::new();
        let mut paste_now = false;
        // Ctrl+V même quand le presse-papiers ne contient pas de texte (fichiers copiés dans l'Explorateur)
        #[cfg(windows)]
        {
            let focused = ctx.input(|i| i.focused);
            let down = focused && winx::key_down(0x11) && winx::key_down(0x56);
            if down && !self.v_was_down && !typing {
                paste_now = true;
            }
            self.v_was_down = down;
        }
        let tab_count = self.tabs.len();
        let mut focus_search = false;
        let grid_cols = ctx.data(|d| d.get_temp::<usize>(egui::Id::new("grid_cols"))).unwrap_or(1).max(1);
        ctx.input_mut(|i| {
            use egui::{Key, Modifiers as M};
            // Événements sémantiques du presse-papiers
            let mut copy = None;
            for ev in &i.events {
                match ev {
                    egui::Event::Copy if !typing => copy = Some(false),
                    egui::Event::Cut if !typing => {
                        if i.modifiers.shift && !i.modifiers.command {
                            acts.push(Action::AskDelete); // Maj+Suppr
                        } else {
                            copy = Some(true)
                        }
                    }
                    egui::Event::Text(t) if !typing && !i.modifiers.command && !t.trim().is_empty() => {
                        let (buf, at) = &mut self.typeahead;
                        if at.elapsed() > Duration::from_millis(900) {
                            buf.clear();
                        }
                        buf.push_str(&t.to_lowercase());
                        *at = Instant::now();
                    }
                    _ => {}
                }
            }
            if let Some(cut) = copy {
                if i.modifiers.shift && !cut {
                    acts.push(Action::CopyPath);
                } else {
                    acts.push(Action::Copy(cut));
                }
            }
            if i.consume_key(M::COMMAND, Key::T) {
                acts.push(Action::NewTab(PathBuf::new()));
            }
            if i.consume_key(M::COMMAND, Key::W) {
                acts.push(Action::CloseTab(usize::MAX));
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::Tab) {
                acts.push(Action::ActivateTab((self.active + tab_count - 1) % tab_count));
            }
            if i.consume_key(M::COMMAND, Key::Tab) {
                acts.push(Action::ActivateTab((self.active + 1) % tab_count));
            }
            // Liste / grille : Ctrl+Maj+L / Ctrl+Maj+G (tous claviers), Ctrl+1 / Ctrl+2 (QWERTY).
            if i.consume_key(M::COMMAND | M::SHIFT, Key::L) {
                (self.s.grid, self.s.space_map) = (false, false);
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::G) {
                (self.s.grid, self.s.space_map) = (true, false);
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::M) {
                self.s.space_map = true;
            }
            if i.consume_key(M::COMMAND, Key::L) {
                let p = self.tabs[self.active].path.to_string_lossy().into_owned();
                self.tabs[self.active].address = Some(p);
            }
            // (Ctrl+K = palette de commandes, F1 = aide : gérés par le socle dans draw())
            if i.consume_key(M::COMMAND, Key::F) {
                focus_search = true;
            }
            if i.consume_key(M::NONE, Key::F5) {
                acts.push(Action::Refresh);
            }
            if i.consume_key(M::COMMAND, Key::P) {
                acts.push(Action::OpenGoto);
            }
            if i.consume_key(M::NONE, Key::F3) {
                acts.push(Action::ToggleSplit);
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::N) {
                acts.push(Action::NewFolder);
            }
            if i.consume_key(M::COMMAND, Key::H) {
                self.s.show_hidden = !self.s.show_hidden;
                for t in &mut self.tabs {
                    t.view_dirty = true;
                }
            }
            if i.consume_key(M::COMMAND | M::SHIFT, Key::D) {
                self.s.dev = !self.s.dev;
            }
            if i.consume_key(M::ALT, Key::ArrowLeft) || i.pointer.button_pressed(egui::PointerButton::Extra1) {
                acts.push(Action::Back);
            }
            if i.consume_key(M::ALT, Key::ArrowRight) || i.pointer.button_pressed(egui::PointerButton::Extra2) {
                acts.push(Action::Forward);
            }
            if i.consume_key(M::ALT, Key::ArrowUp) {
                acts.push(Action::Up);
            }
            if typing {
                return;
            }
            if i.consume_key(M::NONE, Key::Backspace) {
                acts.push(Action::Up);
            }
            if i.consume_key(M::COMMAND, Key::A) {
                acts.push(Action::SelectAll);
            }
            if i.consume_key(M::NONE, Key::F2) {
                if self.tabs[self.active].selected.len() > 1 {
                    acts.push(Action::OpenBatchRename);
                } else if let Some(c) = self.tabs[self.active].cursor.clone() {
                    acts.push(Action::StartRename(c));
                }
            }
            if i.consume_key(M::COMMAND, Key::Z) {
                acts.push(Action::Undo);
            }
            if i.consume_key(M::NONE, Key::F6) {
                acts.push(Action::ToOtherPane(false));
            }
            if i.consume_key(M::SHIFT, Key::F6) {
                acts.push(Action::ToOtherPane(true));
            }
            if i.consume_key(M::NONE, Key::Space) {
                if let Some(c) = self.tabs[self.active].cursor.clone() {
                    acts.push(Action::FullView(c));
                }
            }
            if i.consume_key(M::NONE, Key::Delete) {
                acts.push(Action::Trash);
            }
            if i.consume_key(M::NONE, Key::Enter) {
                for p in self.tabs[self.active].selection().into_iter().take(16) {
                    acts.push(Action::Open(p));
                }
            }
            if i.consume_key(M::NONE, Key::Escape) {
                let t = &mut self.tabs[self.active];
                if !t.filter.is_empty() {
                    t.filter.clear();
                    acts.push(Action::StartSearch);
                } else {
                    acts.push(Action::ClearSel);
                }
            }
            // Déplacement du curseur
            let step = if self.s.grid { grid_cols } else { 1 };
            let page = ((self.tabs[self.active].view_h / 28.0) as usize).max(1);
            let mut mv: Option<(isize, bool)> = None;
            for (key, d) in [
                (Key::ArrowDown, step as isize),
                (Key::ArrowUp, -(step as isize)),
                (Key::ArrowRight, 1),
                (Key::ArrowLeft, -1),
                (Key::PageDown, page as isize),
                (Key::PageUp, -(page as isize)),
                (Key::End, isize::MAX / 2),
                (Key::Home, isize::MIN / 2),
            ] {
                if !self.s.grid && matches!(key, Key::ArrowLeft | Key::ArrowRight) {
                    continue;
                }
                if i.consume_key(M::NONE, key) {
                    mv = Some((d, false));
                }
                if i.consume_key(M::SHIFT, key) {
                    mv = Some((d, true));
                }
            }
            if let Some((d, shift)) = mv {
                let t = &self.tabs[self.active];
                let n = t.view.len();
                if n > 0 {
                    let cur = t.cursor.as_ref().and_then(|c| t.view_index_of(c));
                    let next = match cur {
                        None => 0,
                        Some(c) => (c as isize).saturating_add(d).clamp(0, n as isize - 1) as usize,
                    };
                    if let Some(e) = t.entry(next) {
                        acts.push(Action::Select { path: e.path.clone(), ctrl: false, shift });
                    }
                }
            }
        });
        if focus_search {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("search")));
        }
        // Recherche par frappe (« type-ahead »)
        if !self.typeahead.0.is_empty() && self.typeahead.1.elapsed() < Duration::from_millis(50) {
            let buf = self.typeahead.0.clone();
            let t = self.tab();
            let src = t.source();
            if let Some(e) = t.view.iter().filter_map(|&i| src.get(i)).find(|e| e.lower.starts_with(&buf)) {
                acts.push(Action::Select { path: e.path.clone(), ctrl: false, shift: false });
            }
        }
        let pasted_text = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
        if (paste_now || pasted_text) && !typing {
            acts.push(Action::Paste(self.tab().path.clone()));
        }
        self.actions.extend(acts);
    }

    pub fn apply_actions(&mut self) {
        let acts = std::mem::take(&mut self.actions);
        for a in acts {
            self.apply(a);
        }
    }

    fn apply(&mut self, a: Action) {
        match a {
            Action::Navigate(p) => {
                if p.is_dir() || crate::recents::is_recents(&p) {
                    self.navigate(p);
                } else if p.exists() {
                    let _ = open::that_detached(&p);
                } else {
                    self.toast(format!("Introuvable : {}", p.display()), true);
                }
            }
            Action::NewTab(p) => {
                let p = if p.as_os_str().is_empty() { self.tab().path.clone() } else { p };
                let i = self.add_tab(p);
                self.active = i;
            }
            Action::CloseTab(i) => {
                let i = if i == usize::MAX { self.active } else { i };
                if let Some(t) = self.tabs.get(i) {
                    self.space_maps.remove(&t.id);
                }
                if self.tabs.len() > 1 && i < self.tabs.len() {
                    let t = self.tabs.remove(i);
                    if let Some(s) = t.search {
                        s.cancel.store(true, Ordering::Relaxed);
                    }
                    if self.active >= self.tabs.len() || self.active > i {
                        self.active = self.active.saturating_sub(1);
                    }
                    // Double volet : garder l'autre volet sur le bon onglet
                    self.split_other = match self.split_other {
                        Some(o) if o == i => None,
                        Some(o) if o > i => Some(o - 1),
                        o => o,
                    };
                    if self.split_other == Some(self.active) || self.split_other.map(|o| o >= self.tabs.len()).unwrap_or(false) {
                        self.split_other = None;
                    }
                }
            }
            Action::ActivateTab(i) => {
                if i < self.tabs.len() {
                    if self.split_other == Some(i) {
                        // onglet de l'autre volet : on passe simplement dans ce volet
                        self.split_other = Some(self.active);
                        self.focus_right = !self.focus_right;
                    }
                    self.active = i;
                }
            }
            Action::Open(p) => {
                let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                if p.is_dir() {
                    self.navigate(p);
                } else if let (true, Some(v), true) = (self.s.use_viewer, self.viewer.clone(), crate::viewer::handles(&ext)) {
                    if !crate::viewer::open(&v, &[p.clone()]) {
                        let _ = open::that_detached(&p);
                    }
                } else if let Err(e) = open::that_detached(&p) {
                    self.toast(format!("Impossible d’ouvrir : {e}"), true);
                }
            }
            Action::Select { path, ctrl, shift } => {
                let t = self.tab_mut();
                let idx = t.view_index_of(&path);
                if shift {
                    let a = t.anchor.as_ref().and_then(|a| t.view_index_of(a)).or(idx);
                    if let (Some(a), Some(b)) = (a, idx) {
                        if !ctrl {
                            t.selected.clear();
                        }
                        let (lo, hi) = (a.min(b), a.max(b));
                        let paths: Vec<PathBuf> = (lo..=hi).filter_map(|k| t.entry(k).map(|e| e.path.clone())).collect();
                        t.selected.extend(paths);
                    }
                } else if ctrl {
                    if !t.selected.remove(&path) {
                        t.selected.insert(path.clone());
                    }
                    t.anchor = Some(path.clone());
                } else {
                    t.selected.clear();
                    t.selected.insert(path.clone());
                    t.anchor = Some(path.clone());
                }
                t.cursor = Some(path);
                t.scroll_to = idx;
            }
            Action::SelectOnly(p) => {
                let t = self.tab_mut();
                if !t.selected.contains(&p) {
                    t.selected.clear();
                    t.selected.insert(p.clone());
                    t.anchor = Some(p.clone());
                }
                t.cursor = Some(p);
            }
            Action::ClearSel => {
                let t = self.tab_mut();
                t.selected.clear();
                t.renaming = None;
            }
            Action::SelectAll => {
                let t = self.tab_mut();
                let src = t.source();
                let all: HashSet<PathBuf> = t.view.iter().filter_map(|&i| src.get(i)).map(|e| e.path.clone()).collect();
                t.selected = all;
            }
            Action::Copy(cut) => {
                let sel = self.tab().selection();
                if sel.is_empty() {
                    return;
                }
                #[cfg(windows)]
                winx::set_files(&sel, cut);
                self.toast(
                    format!("{} élément(s) {} — Ctrl+V pour coller", sel.len(), if cut { "coupé(s)" } else { "copié(s)" }),
                    false,
                );
            }
            Action::Paste(dst) => {
                if crate::recents::is_recents(&dst) {
                    self.toast("« Récents » n’est pas un vrai dossier : colle dans un dossier", true);
                    return;
                }
                #[cfg(windows)]
                if let Some((files, cut)) = winx::get_files() {
                    if files.is_empty() {
                        return;
                    }
                    if cut {
                        self.ops.submit(Op::Move { srcs: files, dst });
                        // Après un « couper », le presse-papiers ne doit plus resservir
                        unsafe_clear_clipboard();
                    } else {
                        self.ops.submit(Op::Copy { srcs: files, dst });
                    }
                }
            }
            Action::TrashPaths(paths) => {
                if !paths.is_empty() {
                    self.ops.submit(Op::Trash(paths));
                }
            }
            Action::Trash => {
                let sel = self.tab().selection();
                if !sel.is_empty() {
                    self.ops.submit(Op::Trash(sel));
                }
            }
            Action::AskDelete => {
                let sel = self.tab().selection();
                if !sel.is_empty() {
                    self.confirm = Some(Confirm::Delete(sel));
                }
            }
            Action::StartRename(p) => {
                let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.tab_mut().renaming = Some(Rename { path: p, text: name, fresh: true });
            }
            Action::CommitRename => {
                let Some(r) = self.tab_mut().renaming.take() else { return };
                let new_name = r.text.trim().to_string();
                let old_name = r.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if new_name.is_empty() || new_name == old_name {
                    return;
                }
                if new_name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
                    self.toast("Un nom ne peut pas contenir \\ / : * ? \" < > |", true);
                    return;
                }
                let target = r.path.with_file_name(&new_name);
                // (même fichier à la casse près, accents compris : « Été » → « été »)
                let same = target.to_string_lossy().to_lowercase() == r.path.to_string_lossy().to_lowercase();
                if target.exists() && !same {
                    self.toast(format!("« {new_name} » existe déjà"), true);
                    return;
                }
                match std::fs::rename(&r.path, &target) {
                    Ok(()) => {
                        self.undo.push(crate::ops::Undo::Moved(vec![(r.path.clone(), target.clone())]));
                        self.apply_path(&r.path, None);
                        self.apply_path(&target, None);
                        self.recompute_view(self.active);
                        self.apply(Action::Select { path: target, ctrl: false, shift: false });
                    }
                    Err(e) => self.toast(format!("Renommage impossible : {e}"), true),
                }
            }
            Action::CancelRename => self.tab_mut().renaming = None,
            Action::NewFolder | Action::NewFile => {
                let folder = matches!(a, Action::NewFolder);
                let dir = self.tab().path.clone();
                if crate::recents::is_recents(&dir) {
                    self.toast("« Récents » n’est pas un vrai dossier : rien ne peut y être créé", true);
                    return;
                }
                let p = fsx::unique_name(&dir, if folder { "Nouveau dossier" } else { "Nouveau fichier.txt" });
                let r = if folder { std::fs::create_dir(&p) } else { std::fs::write(&p, b"") };
                match r {
                    Ok(()) => {
                        self.undo.push(crate::ops::Undo::Created(vec![p.clone()]));
                        self.apply_path(&p, None);
                        self.recompute_view(self.active);
                        self.apply(Action::Select { path: p.clone(), ctrl: false, shift: false });
                        self.apply(Action::StartRename(p));
                    }
                    Err(e) => self.toast(format!("Création impossible : {e}"), true),
                }
            }
            Action::CopyPath => {
                let sel = self.tab().selection();
                let text = if sel.is_empty() {
                    self.tab().path.to_string_lossy().into_owned()
                } else {
                    sel.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\r\n")
                };
                self.ctx.copy_text(text);
                self.toast("Chemin copié", false);
            }
            Action::Pin(p) => {
                if !self.s.pinned.contains(&p) {
                    self.s.pinned.push(p);
                }
            }
            Action::Unpin(p) => self.s.pinned.retain(|x| x != &p),
            Action::Terminal(p) => winx::open_terminal(&p),
            Action::Code(p) => {
                if !winx::open_in_code(&p) {
                    self.toast("VS Code introuvable (commande `code`)", true);
                }
            }
            Action::Reveal(p) => winx::reveal_in_explorer(&p),
            Action::Properties(p) => {
                #[cfg(windows)]
                winx::properties(&p);
            }
            Action::OpenWith(p) => winx::open_with(&p),
            Action::Sort(k) => {
                if self.s.sort == k {
                    self.s.asc = !self.s.asc;
                } else {
                    self.s.sort = k;
                    self.s.asc = k == Sort::Name || k == Sort::Kind;
                }
                for t in &mut self.tabs {
                    t.view_dirty = true;
                }
            }
            Action::DropInto { paths, dst, copy } => {
                if crate::recents::is_recents(&dst) {
                    return;
                }
                let paths: Vec<PathBuf> = paths.into_iter().filter(|p| p != &dst && p.parent() != Some(&dst)).collect();
                if paths.is_empty() {
                    return;
                }
                if copy {
                    self.ops.submit(Op::Copy { srcs: paths, dst });
                } else {
                    self.ops.submit(Op::Move { srcs: paths, dst });
                }
            }
            Action::Refresh => {
                let i = self.active;
                self.start_load(i);
                self.refresh_drives();
                if let Some(r) = &self.repo {
                    self.git.send(git::Req::Status(r.root.clone()));
                }
            }
            Action::Up => {
                let cur = self.tab().path.clone();
                if crate::recents::is_recents(&cur) {
                    return;
                }
                if let Some(parent) = cur.parent() {
                    let parent = parent.to_path_buf();
                    self.navigate(parent);
                    self.tab_mut().focus_after_load = Some(cur);
                }
            }
            Action::Back => {
                let t = self.tab_mut();
                if let Some(p) = t.back.pop() {
                    let cur = std::mem::replace(&mut t.path, p);
                    t.fwd.push(cur.clone());
                    self.reset_tab_view();
                    self.tab_mut().focus_after_load = Some(cur);
                }
            }
            Action::Forward => {
                let t = self.tab_mut();
                if let Some(p) = t.fwd.pop() {
                    let cur = std::mem::replace(&mut t.path, p);
                    t.back.push(cur);
                    self.reset_tab_view();
                }
            }
            Action::ShowInFolder(p) => {
                if let Some(parent) = p.parent() {
                    let parent = parent.to_path_buf();
                    if self.tab().path != parent {
                        self.navigate(parent);
                    }
                    self.s.dev = false;
                    self.tab_mut().focus_after_load = Some(p.clone());
                    self.apply(Action::Select { path: p, ctrl: false, shift: false });
                }
            }
            Action::GitSelect(rel) => {
                self.burst_view = None;
                if let Some(r) = self.repo.as_mut() {
                    r.selected = Some(rel);
                }
                self.request_diff();
            }
            Action::GitRestoreAsk(rel, untracked) => self.confirm = Some(Confirm::Restore { rel, untracked }),
            Action::BurstView(id) => {
                if self.burst_view == Some(id) {
                    self.burst_view = None;
                } else {
                    self.burst_view = Some(id);
                    self.burst_diff = None;
                    self.request_burst_diff(id);
                }
            }
            Action::BurstRestoreAsk(id) => self.confirm = Some(Confirm::BurstRestore(id)),
            Action::BurstRedo(id) => self.restore_burst(id, true),
            Action::GitCommit => {
                if let Some(r) = self.repo.as_mut() {
                    let files = r.included();
                    if files.is_empty() || r.msg.trim().is_empty() {
                        return;
                    }
                    r.busy = Some("Commit…");
                    self.git.send(git::Req::Commit { root: r.root.clone(), files, msg: r.msg.trim().to_string() });
                }
            }
            Action::GitPush => {
                if let Some(r) = self.repo.as_mut() {
                    r.busy = Some("Push…");
                    self.git.send(git::Req::Push(r.root.clone()));
                }
            }
            Action::GitInit(p) => self.git.send(git::Req::Init(p)),
            Action::GitRefresh => {
                if let Some(r) = &self.repo {
                    self.git.send(git::Req::Status(r.root.clone()));
                }
            }
            Action::StartSearch => self.start_search(),
            Action::Undo => match self.undo.pop() {
                Some(u) => {
                    let label = u.label();
                    match u {
                        crate::ops::Undo::Created(v) => self.ops.submit(Op::TrashForUndo(v)),
                        crate::ops::Undo::Moved(v) => self.ops.submit(Op::MoveBack(v)),
                        crate::ops::Undo::Trashed { paths, at } => self.ops.submit(Op::Restore { paths, since: at }),
                    }
                    self.toast(format!("Annulation : {label}"), false);
                }
                None => self.toast("Rien à annuler", false),
            },
            Action::Zip(paths) => {
                if paths.is_empty() {
                    return;
                }
                let dir = paths[0].parent().map(|p| p.to_path_buf()).unwrap_or_else(|| self.tab().path.clone());
                let base = if paths.len() == 1 {
                    let p = &paths[0];
                    let stem = if p.is_dir() { p.file_name() } else { p.file_stem() };
                    stem.map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into())
                } else {
                    dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into())
                };
                let dst = fsx::unique_name(&dir, &format!("{base}.zip"));
                self.ops.submit(Op::Zip { srcs: paths, dst });
            }
            Action::Unzip(zip) => {
                let dir = zip.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                let stem = zip.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into());
                let dst = fsx::unique_name(&dir, &stem);
                self.ops.submit(Op::Unzip { zip, dst });
            }
            Action::ToggleSplit => {
                if self.split_other.take().is_none() {
                    let other = if self.tabs.len() > 1 {
                        (self.active + 1) % self.tabs.len()
                    } else {
                        let p = self.tab().path.clone();
                        self.add_tab(p)
                    };
                    self.split_other = Some(other);
                    self.focus_right = false;
                }
            }
            Action::ToOtherPane(mv) => {
                let Some(o) = self.split_other else {
                    self.toast("Ouvre le double volet avec F3", false);
                    return;
                };
                let dst = self.tabs[o].path.clone();
                let sel = self.tab().selection();
                if sel.is_empty() {
                    return;
                }
                self.ops.submit(if mv { Op::Move { srcs: sel, dst } } else { Op::Copy { srcs: sel, dst } });
            }
            Action::OpenGoto => {
                self.goto = Some(Goto { query: String::new(), sel: 0, fresh: true, hits: vec![], hits_q: String::new(), folders: vec![], folders_q: None })
            }
            Action::FullView(p) => {
                let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                match self.viewer.clone() {
                    Some(v) if self.s.quick_viewer && p.is_file() && crate::viewer::handles(&ext) => {
                        crate::viewer::quick_look(&v, &p, self.preview_fallback_tx.clone(), self.ctx.clone());
                    }
                    _ => self.fullview = Some(p),
                }
            }
            Action::OpenBatchRename => {
                let paths = self.tab().selection();
                if !paths.is_empty() {
                    self.batch = Some(BatchRename {
                        paths,
                        pattern: "{nom}".into(),
                        find: String::new(),
                        replace: String::new(),
                        regex: false,
                        start: 1,
                        fresh: true,
                        preview: vec![],
                        conflicts: vec![],
                        preview_for: None,
                    });
                }
            }
            Action::ClaudeHere(p) => {
                let dir = if p.is_dir() { p } else { p.parent().map(|x| x.to_path_buf()).unwrap_or(p) };
                if winx::open_claude_code(&dir) {
                    self.toast("Nouvelle session Claude Code ouverte dans l’app Claude", false);
                } else {
                    self.toast("App Claude introuvable : Claude Code lancé dans un terminal", false);
                }
            }
            Action::NativeMenu(v) => self.native_menu = Some(v),
            Action::CheckUpdate => self.start_update_check(true),
            Action::InstallUpdate => {
                if let Some(r) = self.update.clone() {
                    self.update_busy = Some("Téléchargement de la mise à jour…");
                    let (tx, ctx) = (self.update_tx.clone(), self.ctx.clone());
                    std::thread::spawn(move || {
                        let res = UPDATER.install(&r, &|_| {}).and_then(|exe| socle::update::relaunch(&exe));
                        let _ = tx.send(UpdateMsg::Installed(res));
                        ctx.request_repaint();
                    });
                }
            }
            Action::OpenInViewer(v) => match &self.viewer {
                Some(exe) => {
                    crate::viewer::open(exe, &v);
                }
                None => self.toast("Lecteur universel introuvable (C:\\Dev\\lecteur-universel)", true),
            },
            Action::Convert(fmt, files) => match self.viewer.clone() {
                Some(viewer) => {
                    self.toast(format!("Conversion en {} de {} fichier(s)…", fmt.to_uppercase(), files.len()), false);
                    self.ops.submit(Op::Convert { viewer, fmt, files });
                }
                None => self.toast("Lecteur universel introuvable (C:\\Dev\\lecteur-universel)", true),
            },
        }
    }

    pub fn confirm_yes(&mut self) {
        match self.confirm.take() {
            Some(Confirm::Delete(paths)) => self.ops.submit(Op::Delete(paths)),
            Some(Confirm::Restore { rel, untracked }) => {
                if let Some(r) = &self.repo {
                    self.git.send(git::Req::Restore { root: r.root.clone(), rel, untracked });
                }
            }
            Some(Confirm::BurstRestore(id)) => self.restore_burst(id, false),
            None => {}
        }
    }

    /// Annule une passe (remet ses fichiers dans l'état d'avant) ou la rétablit (`redo`).
    fn restore_burst(&mut self, id: u64, redo: bool) {
        let Some(b) = self.bursts.get(id) else { return };
        let to = if redo { b.undo.clone() } else { b.before.clone() };
        let (Some(to), Some(base)) = (to, self.bursts.snapshot_of(&b.root)) else {
            self.toast("Pas d’instantané git pour cette passe : restauration impossible", true);
            return;
        };
        let (root, files) = (b.root.clone(), b.restorable());
        // Seuls ces chemins peuvent partir à la corbeille : créés par la passe (on l'annule) ou
        // supprimés par elle (on la rétablit). Un fichier dont on ignore l'état d'avant reste.
        let gone = if redo { Act::Removed } else { Act::Created };
        let may_remove: Vec<String> = b.files.iter().filter(|(_, a)| **a == gone).map(|(f, _)| f.clone()).collect();
        let (raw, eol) = (self.bursts.raw_of(&root), self.bursts.eol_of(&root));
        self.bursts.mute(&root, &files);
        self.bursts.hold(&root);
        if let Some(b) = self.bursts.get_mut(id) {
            b.busy = true;
        }
        self.git.send(git::Req::BurstRestore { root, id, to, base, files, may_remove, raw, eol, redo });
    }

    #[cfg(windows)]
    pub fn set_default_explorer(&mut self, on: bool) {
        // Lancé depuis une app empaquetée : le registre écrit serait celui de son conteneur.
        if winx::virtualized() {
            self.toast("Explorateur a été lancé depuis une autre app (conteneur) : relance-le depuis le menu Démarrer pour changer ce réglage", true);
            self.is_default = winx::default_explorer::is_enabled();
            return;
        }
        if on {
            match winx::default_explorer::enable() {
                Ok(()) => self.toast("Explorateur est maintenant l’explorateur par défaut (Win+E, dossiers)", false),
                Err(e) => self.toast(format!("Impossible : {e}"), true),
            }
        } else {
            winx::default_explorer::disable();
            self.toast("L’Explorateur Windows est de nouveau l’explorateur par défaut", false);
        }
        self.is_default = winx::default_explorer::is_enabled();
    }

    pub fn start_update_check(&mut self, manual: bool) {
        self.s.last_update_check = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        self.update_busy = Some("Recherche de mise à jour…");
        let (tx, ctx) = (self.update_tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let res = UPDATER.check();
            let _ = tx.send(UpdateMsg::Checked { manual, res });
            ctx.request_repaint();
        });
    }

    fn process_updates(&mut self) {
        while let Ok(m) = self.update_rx.try_recv() {
            self.update_busy = None;
            match m {
                UpdateMsg::Checked { manual, res } => match res {
                    Ok(Some(r)) => self.update = Some(r),
                    Ok(None) if manual => self.toast(format!("Tu as la dernière version ({})", env!("CARGO_PKG_VERSION")), false),
                    Err(e) if manual => self.toast(format!("Vérification impossible : {e}"), true),
                    _ => {}
                },
                UpdateMsg::Installed(Ok(())) => {
                    // La nouvelle version est lancée : on enregistre et on s'arrête.
                    self.flush_settings(None);
                    std::process::exit(0);
                }
                UpdateMsg::Installed(Err(e)) => self.toast(format!("Mise à jour impossible : {e}"), true),
            }
        }
    }

    pub fn cancel_ops(&self) {
        self.ops.cancel.store(true, Ordering::Relaxed);
    }

    pub fn git_kind(&self, p: &Path) -> Option<git::Kind> {
        self.repo.as_ref().and_then(|r| r.by_path.get(p).copied())
    }
}

#[cfg(windows)]
fn unsafe_clear_clipboard() {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard};
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) != 0 {
            EmptyClipboard();
            CloseClipboard();
        }
    }
}

/// Tri « naturel » : fichier2 avant fichier10.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    bi.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// Empreinte des réglages (pour savoir s'il faut les réécrire).
fn settings_hash(s: &Settings) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(s).unwrap_or_default().hash(&mut h);
    h.finish()
}
