//! Intégration git : statut, diff, commit, push, restauration.
//! Prototype : on appelle `git` (déjà installé) sur un thread dédié.
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflict,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Modified => "modifié",
            Kind::Added => "ajouté",
            Kind::Deleted => "supprimé",
            Kind::Renamed => "renommé",
            Kind::Untracked => "nouveau",
            Kind::Conflict => "conflit",
        }
    }
    pub fn letter(self) -> &'static str {
        match self {
            Kind::Modified => "M",
            Kind::Added => "A",
            Kind::Deleted => "D",
            Kind::Renamed => "R",
            Kind::Untracked => "U",
            Kind::Conflict => "!",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Change {
    pub rel: String,
    pub kind: Kind,
    pub plus: Option<u32>,
    pub minus: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Ctx,
    Add,
    Del,
    Hunk,
    /// Nom du fichier (diff de plusieurs fichiers).
    File,
}

#[derive(Clone)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

pub enum Req {
    Detect(PathBuf),
    Status(PathBuf),
    Diff { root: PathBuf, rel: String, untracked: bool },
    Commit { root: PathBuf, files: Vec<String>, msg: String },
    Push(PathBuf),
    Restore { root: PathBuf, rel: String, untracked: bool },
    Init(PathBuf),
    /// Instantané (rafales d'agent) : `why` = 0 pour la base du dépôt, sinon l'id de la passe close.
    Snapshot { root: PathBuf, base: Option<String>, files: Vec<String>, why: u64 },
    BurstDiff { root: PathBuf, id: u64, before: String, after: Option<String>, files: Vec<String> },
    /// Remet `files` dans l'état de l'arbre `to` (`base` = instantané courant du dépôt).
    BurstRestore {
        root: PathBuf,
        id: u64,
        to: String,
        base: String,
        files: Vec<String>,
        may_remove: Vec<String>,
        /// Chemins copiés tels quels dans les instantanés du dépôt.
        raw: std::collections::HashSet<String>,
        /// Fin de ligne d'origine des fichiers suivis (voir `Snap::eol`).
        eol: HashMap<String, bool>,
        redo: bool,
    },
}

pub enum Resp {
    Detected { path: PathBuf, root: Option<PathBuf> },
    Status { root: PathBuf, branch: String, ahead: String, changes: Vec<Change> },
    Diff { rel: String, lines: Vec<DiffLine> },
    Done { what: &'static str, ok: bool, msg: String, root: PathBuf },
    Snapshot { root: PathBuf, why: u64, res: Result<Snap, String> },
    BurstDiff { id: u64, lines: Vec<DiffLine>, plus: u32, minus: u32 },
    /// `res` : (état d'avant la restauration, état d'après = base de la passe suivante).
    BurstRestored { root: PathBuf, id: u64, redo: bool, res: Result<Restored, String> },
}

pub struct Git {
    tx: Sender<Req>,
    pub rx: Receiver<Resp>,
}

impl Git {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, req_rx) = unbounded::<Req>();
        let (res_tx, rx) = unbounded();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            while let Ok(mut req) = req_rx.recv() {
                // Garder seulement la dernière demande de statut/diff en attente.
                while let Ok(next) = req_rx.try_recv() {
                    let replaceable = matches!(req, Req::Status(_) | Req::Diff { .. } | Req::Detect(_));
                    if replaceable && std::mem::discriminant(&req) == std::mem::discriminant(&next) {
                        req = next;
                    } else {
                        let _ = res_tx.send(handle(req));
                        req = next;
                    }
                }
                let _ = res_tx.send(handle(req));
                ctx.request_repaint();
            }
        });
        Git { tx, rx }
    }
    pub fn send(&self, r: Req) {
        let _ = self.tx.send(r);
    }
}

fn git(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).args(["-c", "core.quotepath=off", "-c", "color.ui=false"]);
    // Chemins toujours pris à la lettre : « [ab].txt » ne doit pas désigner a.txt et b.txt.
    c.env("GIT_LITERAL_PATHSPECS", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW : pas de console qui clignote
    }
    c
}

fn out(mut c: Command) -> Result<String, String> {
    match c.output() {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => {
            let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Err(if e.is_empty() { String::from_utf8_lossy(&o.stdout).trim().to_string() } else { e })
        }
        Err(e) => Err(format!("git introuvable : {e}")),
    }
}

fn handle(req: Req) -> Resp {
    match req {
        Req::Detect(path) => {
            let mut c = git(&path);
            c.args(["rev-parse", "--show-toplevel"]);
            let root = out(c).ok().map(|s| PathBuf::from(s.trim().replace('/', "\\")));
            Resp::Detected { path, root }
        }
        Req::Status(root) => status(root),
        Req::Diff { root, rel, untracked } => {
            let text = if untracked {
                let mut c = git(&root);
                c.args(["diff", "--no-index", "--", "/dev/null", &rel]);
                // --no-index renvoie 1 quand il y a une différence : on lit stdout quand même.
                c.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
            } else {
                let mut c = git(&root);
                c.args(["diff", "HEAD", "--", &rel]);
                out(c).or_else(|_| {
                    let mut c = git(&root);
                    c.args(["diff", "--", &rel]);
                    out(c)
                })
                .unwrap_or_default()
            };
            let mut lines = parse_diff(&text);
            lines.retain(|l| l.kind != LineKind::File); // le nom est déjà dans l'en-tête
            Resp::Diff { rel, lines }
        }
        Req::Commit { root, files, msg } => {
            let mut c = git(&root);
            c.args(["add", "-A", "--"]).args(&files);
            let r = out(c).and_then(|_| {
                let mut c = git(&root);
                c.args(["commit", "-m", &msg, "--"]).args(&files);
                out(c)
            });
            done("Commit", r, root)
        }
        Req::Push(root) => {
            let mut c = git(&root);
            c.args(["push"]);
            // git écrit sa progression sur stderr même en cas de succès
            let r = match c.output() {
                Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stderr).into_owned()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                Err(e) => Err(e.to_string()),
            };
            done("Push", r, root)
        }
        Req::Restore { root, rel, untracked } => {
            let r = if untracked {
                trash::delete(root.join(&rel)).map(|_| String::new()).map_err(|e| e.to_string())
            } else {
                let mut c = git(&root);
                c.args(["restore", "--source=HEAD", "--staged", "--worktree", "--", &rel]);
                out(c)
            };
            done("Restauration", r, root)
        }
        Req::Init(root) => {
            let mut c = git(&root);
            c.arg("init");
            done("git init", out(c), root)
        }
        Req::Snapshot { root, base, files, why } => {
            let res = snapshot(&root, base.as_deref(), &files);
            Resp::Snapshot { root, why, res }
        }
        Req::BurstDiff { root, id, before, after, files } => {
            let after = match after {
                Some(a) => Ok(a),
                None => snapshot(&root, Some(&before), &files).map(|x| x.tree),
            };
            // `after` = `before` + les fichiers de la passe : le diff des deux arbres est celui de la passe.
            let text = after
                .and_then(|a| {
                    let mut c = git(&root);
                    // (contenus bruts et normalisés se côtoient : on ignore les CR de fin de ligne)
                    c.args(["diff", "--no-color", "--ignore-cr-at-eol", &before, &a]);
                    out(c)
                })
                .unwrap_or_default();
            let lines = parse_diff(&text);
            let plus = lines.iter().filter(|l| l.kind == LineKind::Add).count() as u32;
            let minus = lines.iter().filter(|l| l.kind == LineKind::Del).count() as u32;
            Resp::BurstDiff { id, lines, plus, minus }
        }
        Req::BurstRestore { root, id, to, base, files, may_remove, raw, eol, redo } => {
            let res = burst_restore(&root, &to, &base, &files, &may_remove, &raw, &eol);
            Resp::BurstRestored { root, id, redo, res }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Instantanés pour les rafales d'agent : un index temporaire (GIT_INDEX_FILE) sert à écrire un
// arbre git. Ni l'index du dépôt, ni la branche, ni l'historique ne changent ; les objets écrits
// ne sont référencés par rien (git gc les nettoie).
// ---------------------------------------------------------------------------------------------

/// Fichiers plus gros : pas copiés dans l'instantané (ni restaurés).
const SNAP_MAX: u64 = 64 * 1024 * 1024;
/// Fichiers non suivis copiés dans l'instantané de base : au plus ce nombre, et ce volume.
const UNTRACKED_FILES: usize = 5000;
const UNTRACKED_BYTES: u64 = 256 * 1024 * 1024;

struct TempIndex(PathBuf);
impl TempIndex {
    fn new() -> TempIndex {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        TempIndex(std::env::temp_dir().join(format!("explorateur-instantane-{}-{n}.idx", std::process::id())))
    }
    fn cmd(&self, root: &Path) -> Command {
        let mut c = git(root);
        c.env("GIT_INDEX_FILE", &self.0);
        c
    }
}
impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("idx.lock"));
    }
}

/// Lance `c` en lui donnant `input` sur l'entrée standard.
fn out_stdin(mut c: Command, input: &[u8]) -> Result<String, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = c
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git introuvable : {e}"))?;
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(input);
    }
    let o = child.wait_with_output().map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// Chemins séparés par des octets nuls (`-z`, `--pathspec-file-nul`).
fn nul_list(files: &[String]) -> Vec<u8> {
    let mut v = Vec::new();
    for f in files {
        v.extend_from_slice(f.as_bytes());
        v.push(0);
    }
    v
}

fn abs(root: &Path, rel: &str) -> PathBuf {
    root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// Arbre = `base` + contenu actuel de `files` sur le disque. Sans `base` : état actuel de tous les
/// fichiers suivis (index du dépôt + `add -u`). Renvoie l'arbre et les fichiers trop gros ignorés.
/// Écrit `paths` **tels quels** (aucune conversion de fin de ligne ni filtre) dans l'index
/// temporaire : une annulation doit redonner exactement les mêmes octets.
fn overlay_raw(root: &Path, idx: &TempIndex, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut c = git(root);
    c.args(["hash-object", "-w", "--no-filters", "--stdin-paths"]);
    let mut input = paths.join("\n");
    input.push('\n');
    let shas = out_stdin(c, input.as_bytes())?;
    let shas: Vec<&str> = shas.lines().collect();
    if shas.len() != paths.len() {
        return Err("git hash-object : réponse incomplète".into());
    }
    let mut info = Vec::new();
    for (sha, p) in shas.iter().zip(paths) {
        info.extend_from_slice(format!("100644 {sha}\t{p}").as_bytes());
        info.push(0);
    }
    let mut c = idx.cmd(root);
    c.args(["update-index", "-z", "--index-info"]);
    out_stdin(c, &info)?;
    Ok(())
}

/// Retire de l'index temporaire des chemins disparus (fichier ou dossier entier).
fn drop_paths(root: &Path, idx: &TempIndex, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut c = idx.cmd(root);
    c.args(["rm", "-r", "--cached", "--quiet", "--ignore-unmatch", "--pathspec-from-file=-", "--pathspec-file-nul"]);
    out_stdin(c, &nul_list(paths))?;
    Ok(())
}

fn nul_split(s: &str) -> impl Iterator<Item = &str> {
    s.split('\0').filter(|f| !f.is_empty())
}

/// Résultat d'un instantané.
pub struct Snap {
    pub tree: String,
    /// Trop gros (ou au-delà du budget des non suivis) : pas dans l'instantané.
    pub skipped: Vec<String>,
    /// Chemins copiés tels quels (à restaurer tels quels, voir `burst_restore`).
    pub raw: Vec<String>,
    /// Instantané de base : fin de ligne réelle sur le disque (`true` = CRLF) des fichiers suivis.
    /// Leur contenu vient de l'index, normalisé : `git restore` ressortirait la fin de ligne de
    /// la configuration, pas forcément celle du fichier (un fichier LF écrit par un outil puis
    /// commité avec autocrlf=true reviendrait en CRLF).
    pub eol: Vec<(String, bool)>,
}

/// Arbre = `base` + contenu actuel de `files` sur le disque. Sans `base` : état actuel du dépôt
/// (index + fichiers suivis modifiés + non suivis hors .gitignore, bornés).
pub fn snapshot(root: &Path, base: Option<&str>, files: &[String]) -> Result<Snap, String> {
    let idx = TempIndex::new();
    let mut skipped = Vec::new();
    let mut raw = Vec::new();
    let mut eol = Vec::new();
    match base {
        Some(tree) => {
            let mut c = idx.cmd(root);
            c.args(["read-tree", tree]);
            out(c)?;
            let mut gone = Vec::new();
            for f in files {
                match std::fs::symlink_metadata(abs(root, f)) {
                    Ok(m) if m.is_dir() => {}
                    Ok(m) if m.len() > SNAP_MAX => skipped.push(f.clone()),
                    Ok(m) if m.is_file() => raw.push(f.clone()),
                    Ok(_) => {} // lien : on n'y touche pas
                    Err(_) => gone.push(f.clone()),
                }
            }
            drop_paths(root, &idx, &gone)?;
            overlay_raw(root, &idx, &raw)?;
        }
        None => {
            let mut c = git(root);
            c.args(["rev-parse", "--git-path", "index"]);
            let p = PathBuf::from(out(c)?.trim());
            let real = if p.is_absolute() { p } else { root.join(p) };
            if real.exists() {
                std::fs::copy(&real, &idx.0).map_err(|e| e.to_string())?;
            } else {
                let mut c = idx.cmd(root);
                c.args(["read-tree", "--empty"]);
                out(c)?;
            }
            // Suivis supprimés, suivis modifiés, puis non suivis (un agent peut modifier des notes
            // jamais commitées : il faut pouvoir les remettre). Bornés en nombre et en taille.
            let mut c = git(root);
            c.args(["ls-files", "-d", "-z"]);
            let deleted: Vec<String> = nul_split(&out(c)?).map(String::from).collect();
            drop_paths(root, &idx, &deleted)?;
            let mut c = git(root);
            c.args(["ls-files", "-m", "-z"]);
            let modified = out(c)?;
            let mut c = git(root);
            c.args(["ls-files", "-o", "--exclude-standard", "-z"]);
            let untracked = out(c)?;
            let mut budget = UNTRACKED_BYTES;
            let candidates = nul_split(&modified).chain(nul_split(&untracked).take(UNTRACKED_FILES));
            for f in candidates {
                match std::fs::symlink_metadata(abs(root, f)) {
                    Ok(m) if m.is_file() && m.len() <= SNAP_MAX && m.len() <= budget => {
                        budget -= m.len();
                        raw.push(f.to_string());
                    }
                    Ok(m) if m.is_file() => skipped.push(f.to_string()),
                    _ => {}
                }
            }
            raw.sort();
            raw.dedup();
            overlay_raw(root, &idx, &raw)?;
            // « i/lf    w/crlf  attr/…\tchemin »
            let mut c = git(root);
            c.args(["ls-files", "--eol", "-z"]);
            for rec in nul_split(&out(c)?) {
                let Some((info, path)) = rec.split_once('\t') else { continue };
                match info.split_whitespace().find_map(|t| t.strip_prefix("w/")) {
                    Some("crlf") => eol.push((path.to_string(), true)),
                    Some("lf") => eol.push((path.to_string(), false)),
                    _ => {} // binaire, mixte, vide : on laisse git faire
                }
            }
        }
    }
    let mut c = idx.cmd(root);
    c.arg("write-tree");
    Ok(Snap { tree: out(c)?.trim().to_string(), skipped, raw, eol })
}

/// Écrit sur le disque, octet pour octet, le contenu de `paths` dans l'arbre `tree`.
fn write_raw_from(root: &Path, tree: &str, paths: &[String]) -> Result<(), String> {
    use std::io::{BufRead, Read, Write};
    use std::process::Stdio;
    if paths.is_empty() {
        return Ok(());
    }
    let mut c = git(root);
    c.args(["cat-file", "--batch"]);
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let mut input = String::new();
    for p in paths {
        input.push_str(&format!("{tree}:{p}\n"));
    }
    let mut stdin = child.stdin.take().ok_or("git cat-file")?;
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let mut r = std::io::BufReader::new(child.stdout.take().ok_or("git cat-file")?);
    for p in paths {
        let mut head = String::new();
        r.read_line(&mut head).map_err(|e| e.to_string())?;
        // « <sha> blob <taille> » ou « <objet> missing »
        let size: usize = head.trim_end().rsplit(' ').next().and_then(|n| n.parse().ok()).ok_or_else(|| format!("{p} : {}", head.trim()))?;
        let mut data = vec![0u8; size];
        r.read_exact(&mut data).map_err(|e| e.to_string())?;
        let mut nl = [0u8; 1];
        r.read_exact(&mut nl).map_err(|e| e.to_string())?;
        let dst = abs(root, p);
        if let Some(parent) = dst.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&dst, &data).map_err(|e| format!("{p} : {e}"))?;
    }
    let _ = writer.join();
    let _ = child.wait();
    Ok(())
}

/// Remet un fichier texte en LF ou en CRLF partout (rien si ce n'est pas du texte).
fn set_eol(p: &Path, crlf: bool) {
    let Ok(data) = std::fs::read(p) else { return };
    if data.contains(&0) {
        return;
    }
    let mut out = Vec::with_capacity(data.len() + data.len() / 32);
    for (i, &b) in data.iter().enumerate() {
        if b == b'\r' && data.get(i + 1) == Some(&b'\n') {
            continue; // remis ci-dessous avec le \n
        }
        if b == b'\n' && crlf {
            out.push(b'\r');
        }
        out.push(b);
    }
    if out != data {
        let _ = std::fs::write(p, out);
    }
}

/// Résultat d'une restauration de passe.
pub struct Restored {
    /// État d'avant la restauration (pour « Rétablir »).
    pub undo: String,
    /// État d'après (base de la passe suivante).
    pub post: String,
    /// Chemins laissés tels quels (absents de `to`, état d'avant inconnu).
    pub kept: usize,
    /// Chemins copiés tels quels dans les instantanés pris ici.
    pub raw: Vec<String>,
}

/// Remet `files` dans l'état de l'arbre `to`.
/// - ce que `to` contient (fichier, ou tout ce qui est sous un dossier) est réécrit depuis `to` :
///   octet pour octet pour les chemins de `raw` (copiés tels quels), par `git restore` sinon
///   (contenu venu de l'index du dépôt, que git sait ressortir exactement) ;
/// - un chemin absent de `to` ne part à la corbeille que s'il est dans `may_remove` (créé par la
///   passe, ou supprimé par elle quand on la rétablit) et que c'est un fichier ; sinon on n'y touche
///   pas (état d'avant inconnu) et il est compté dans `kept`.
pub fn burst_restore(
    root: &Path,
    to: &str,
    base: &str,
    files: &[String],
    may_remove: &[String],
    raw: &std::collections::HashSet<String>,
    eol: &HashMap<String, bool>,
) -> Result<Restored, String> {
    let before = snapshot(root, Some(base), files)?;
    let mut c = git(root);
    c.args(["ls-tree", "-r", "-z", "--name-only", to]);
    let listed = out(c)?;
    let present: Vec<&str> = nul_split(&listed).collect();
    let present_set: std::collections::HashSet<&str> = present.iter().copied().collect();
    let mut inside: Vec<String> = Vec::new();
    let mut outside: Vec<&String> = Vec::new();
    for f in files {
        if present_set.contains(f.as_str()) {
            inside.push(f.clone());
            continue;
        }
        let prefix = format!("{f}/");
        let under: Vec<String> = present.iter().filter(|p| p.starts_with(&prefix)).map(|p| p.to_string()).collect();
        if under.is_empty() {
            outside.push(f);
        } else {
            inside.extend(under); // dossier : on remet tout ce qu'il contenait
        }
    }
    inside.sort();
    inside.dedup();
    let (exact, via_git): (Vec<String>, Vec<String>) = inside.into_iter().partition(|f| raw.contains(f));
    write_raw_from(root, to, &exact)?;
    if !via_git.is_empty() {
        let mut c = git(root);
        c.args(["restore", "--worktree", "--pathspec-from-file=-", "--pathspec-file-nul"]);
        c.arg(format!("--source={to}"));
        out_stdin(c, &nul_list(&via_git))?;
        // Fin de ligne d'origine (voir `Snap::eol`).
        for f in &via_git {
            if let Some(&crlf) = eol.get(f) {
                set_eol(&abs(root, f), crlf);
            }
        }
    }
    let mut kept = 0;
    let mut dirs = Vec::new();
    for f in outside {
        let p = abs(root, f);
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.is_file() && may_remove.contains(f) => trash::delete(&p).map_err(|e| format!("{f} : {e}"))?,
            Ok(m) if m.is_dir() && may_remove.contains(f) => dirs.push(p),
            Ok(_) => kept += 1,
            Err(_) => {}
        }
    }
    // Dossiers créés par la passe : retirés seulement s'ils sont vides une fois ses fichiers partis.
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        let _ = std::fs::remove_dir(&d);
    }
    let after = snapshot(root, Some(&before.tree), files)?;
    let mut raw: Vec<String> = before.raw;
    raw.extend(after.raw);
    Ok(Restored { undo: before.tree, post: after.tree, kept, raw })
}

fn done(what: &'static str, r: Result<String, String>, root: PathBuf) -> Resp {
    match r {
        Ok(s) => Resp::Done { what, ok: true, msg: s.lines().next().unwrap_or("").to_string(), root },
        Err(e) => Resp::Done { what, ok: false, msg: e.lines().next().unwrap_or("erreur").to_string(), root },
    }
}

fn status(root: PathBuf) -> Resp {
    let mut c = git(&root);
    c.args(["status", "--porcelain=v1", "-z", "--branch", "-uall"]);
    let raw = out(c).unwrap_or_default();
    let mut branch = String::from("?");
    let mut ahead = String::new();
    let mut changes = Vec::new();
    let mut recs = raw.split('\0').filter(|s| !s.is_empty());
    while let Some(rec) = recs.next() {
        if let Some(b) = rec.strip_prefix("## ") {
            let (name, rest) = b.split_once("...").unwrap_or((b, ""));
            branch = name.trim_start_matches("No commits yet on ").to_string();
            if let (Some(i), Some(j)) = (rest.find('['), rest.find(']')) {
                ahead = rest[i + 1..j].replace("ahead", "en avance de").replace("behind", "en retard de");
            }
            continue;
        }
        if rec.len() < 4 {
            continue;
        }
        let x = rec.as_bytes()[0] as char;
        let y = rec.as_bytes()[1] as char;
        let rel = rec[3..].to_string();
        let kind = match (x, y) {
            ('?', '?') => Kind::Untracked,
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => Kind::Conflict,
            ('R', _) | ('C', _) => {
                recs.next(); // chemin d'origine
                Kind::Renamed
            }
            ('D', _) | (_, 'D') => Kind::Deleted,
            ('A', _) => Kind::Added,
            _ => Kind::Modified,
        };
        changes.push(Change { rel, kind, plus: None, minus: None });
    }
    // Lignes ajoutées / supprimées
    let mut stats: HashMap<String, (u32, u32)> = HashMap::new();
    let mut c = git(&root);
    c.args(["diff", "HEAD", "--numstat"]);
    let num = out(c).or_else(|_| {
        let mut c = git(&root);
        c.args(["diff", "--cached", "--numstat"]);
        out(c)
    });
    for line in num.unwrap_or_default().lines() {
        let mut it = line.splitn(3, '\t');
        if let (Some(a), Some(d), Some(p)) = (it.next(), it.next(), it.next()) {
            stats.insert(p.to_string(), (a.parse().unwrap_or(0), d.parse().unwrap_or(0)));
        }
    }
    for ch in &mut changes {
        if let Some((a, d)) = stats.get(&ch.rel) {
            ch.plus = Some(*a);
            ch.minus = Some(*d);
        } else if ch.kind == Kind::Untracked {
            if let Ok(m) = std::fs::metadata(root.join(&ch.rel)) {
                if m.len() < 2_000_000 {
                    if let Ok(s) = std::fs::read(root.join(&ch.rel)) {
                        ch.plus = Some(s.iter().filter(|&&b| b == b'\n').count() as u32);
                    }
                }
            }
        }
    }
    Resp::Status { root, branch, ahead, changes }
}

pub fn parse_diff(text: &str) -> Vec<DiffLine> {
    let mut v = Vec::new();
    let (mut o, mut n) = (0u32, 0u32);
    let mut in_hunk = false;
    for line in text.lines() {
        // Diff de plusieurs fichiers (passes d'agent) : un en-tête par fichier.
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            in_hunk = false;
            let name = rest.split(" b/").next().unwrap_or(rest);
            v.push(DiffLine { kind: LineKind::File, old_no: None, new_no: None, text: name.to_string() });
            continue;
        }
        if line.starts_with("@@") {
            in_hunk = true;
            // @@ -a,b +c,d @@
            let parts: Vec<&str> = line.split_whitespace().collect();
            let num = |s: &str| s.trim_start_matches(['-', '+']).split(',').next().and_then(|x| x.parse().ok());
            o = parts.get(1).and_then(|s| num(s)).unwrap_or(1);
            n = parts.get(2).and_then(|s| num(s)).unwrap_or(1);
            v.push(DiffLine { kind: LineKind::Hunk, old_no: None, new_no: None, text: line.to_string() });
            continue;
        }
        if !in_hunk {
            continue;
        }
        if let Some(t) = line.strip_prefix('+') {
            v.push(DiffLine { kind: LineKind::Add, old_no: None, new_no: Some(n), text: t.to_string() });
            n += 1;
        } else if let Some(t) = line.strip_prefix('-') {
            v.push(DiffLine { kind: LineKind::Del, old_no: Some(o), new_no: None, text: t.to_string() });
            o += 1;
        } else if line.starts_with('\\') {
            continue;
        } else {
            let t = line.strip_prefix(' ').unwrap_or(line);
            v.push(DiffLine { kind: LineKind::Ctx, old_no: Some(o), new_no: Some(n), text: t.to_string() });
            o += 1;
            n += 1;
        }
        if v.len() > 20_000 {
            break;
        }
    }
    v
}
