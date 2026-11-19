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
}

pub enum Resp {
    Detected { path: PathBuf, root: Option<PathBuf> },
    Status { root: PathBuf, branch: String, ahead: String, changes: Vec<Change> },
    Diff { rel: String, lines: Vec<DiffLine> },
    Done { what: &'static str, ok: bool, msg: String, root: PathBuf },
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
            Resp::Diff { rel, lines: parse_diff(&text) }
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
    }
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
