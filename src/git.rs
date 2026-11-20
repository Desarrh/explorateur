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
    BurstRestore { root: PathBuf, id: u64, to: String, base: String, files: Vec<String>, redo: bool },
}

pub enum Resp {
    Detected { path: PathBuf, root: Option<PathBuf> },
    Status { root: PathBuf, branch: String, ahead: String, changes: Vec<Change> },
    Diff { rel: String, lines: Vec<DiffLine> },
    Done { what: &'static str, ok: bool, msg: String, root: PathBuf },
    Snapshot { root: PathBuf, why: u64, tree: Result<String, String>, skipped: Vec<String> },
    BurstDiff { id: u64, lines: Vec<DiffLine>, plus: u32, minus: u32 },
    /// `res` : (état d'avant la restauration, état d'après = base de la passe suivante).
    BurstRestored { root: PathBuf, id: u64, redo: bool, res: Result<(String, String), String> },
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
            let (tree, skipped) = match snapshot(&root, base.as_deref(), &files) {
                Ok((t, s)) => (Ok(t), s),
                Err(e) => (Err(e), vec![]),
            };
            Resp::Snapshot { root, why, tree, skipped }
        }
        Req::BurstDiff { root, id, before, after, files } => {
            let after = match after {
                Some(a) => Ok(a),
                None => snapshot(&root, Some(&before), &files).map(|x| x.0),
            };
            // `after` = `before` + les fichiers de la passe : le diff des deux arbres est celui de la passe.
            let text = after
                .and_then(|a| {
                    let mut c = git(&root);
                    c.args(["diff", "--no-color", &before, &a]);
                    out(c)
                })
                .unwrap_or_default();
            let lines = parse_diff(&text);
            let plus = lines.iter().filter(|l| l.kind == LineKind::Add).count() as u32;
            let minus = lines.iter().filter(|l| l.kind == LineKind::Del).count() as u32;
            Resp::BurstDiff { id, lines, plus, minus }
        }
        Req::BurstRestore { root, id, to, base, files, redo } => {
            let res = burst_restore(&root, &to, &base, &files);
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
pub fn snapshot(root: &Path, base: Option<&str>, files: &[String]) -> Result<(String, Vec<String>), String> {
    let idx = TempIndex::new();
    let mut skipped = Vec::new();
    match base {
        Some(tree) => {
            let mut c = idx.cmd(root);
            c.args(["read-tree", tree]);
            out(c)?;
            let mut keep = Vec::new();
            for f in files {
                match std::fs::metadata(abs(root, f)) {
                    Ok(m) if m.is_dir() => {}
                    Ok(m) if m.len() > SNAP_MAX => skipped.push(f.clone()),
                    _ => keep.push(f.clone()),
                }
            }
            if !keep.is_empty() {
                let mut c = idx.cmd(root);
                c.args(["update-index", "--add", "--remove", "-z", "--stdin"]);
                out_stdin(c, &nul_list(&keep))?;
            }
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
            let mut c = idx.cmd(root);
            c.args(["add", "-u"]);
            out(c)?;
        }
    }
    let mut c = idx.cmd(root);
    c.arg("write-tree");
    Ok((out(c)?.trim().to_string(), skipped))
}

/// Remet `files` dans l'état de l'arbre `to` : contenu de `to` pour ceux qu'il contient, corbeille
/// pour les autres (créés depuis). Renvoie (état d'avant, état d'après).
pub fn burst_restore(root: &Path, to: &str, base: &str, files: &[String]) -> Result<(String, String), String> {
    let (undo, _) = snapshot(root, Some(base), files)?;
    let mut c = git(root);
    c.args(["ls-tree", "-r", "-z", "--name-only", to]);
    let listed = out(c)?;
    let present: std::collections::HashSet<&str> = listed.split('\0').filter(|s| !s.is_empty()).collect();
    let (inside, outside): (Vec<String>, Vec<String>) = files.iter().cloned().partition(|f| present.contains(f.as_str()));
    if !inside.is_empty() {
        let mut c = git(root);
        c.args(["--literal-pathspecs", "restore", "--worktree", "--pathspec-from-file=-", "--pathspec-file-nul"]);
        c.arg(format!("--source={to}"));
        out_stdin(c, &nul_list(&inside))?;
    }
    for f in &outside {
        let p = abs(root, f);
        if p.exists() {
            trash::delete(&p).map_err(|e| format!("{f} : {e}"))?;
        }
    }
    let (post, _) = snapshot(root, Some(&undo), files)?;
    Ok((undo, post))
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
