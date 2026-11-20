//! Opérations sur les fichiers, exécutées en file d'attente hors de l'interface.
//! Chaque opération réussie renvoie de quoi l'annuler (Ctrl+Z).
use crate::fsx::unique_name;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

pub enum Op {
    Copy { srcs: Vec<PathBuf>, dst: PathBuf },
    Move { srcs: Vec<PathBuf>, dst: PathBuf },
    Trash(Vec<PathBuf>),
    Delete(Vec<PathBuf>),
    Zip { srcs: Vec<PathBuf>, dst: PathBuf },
    Unzip { zip: PathBuf, dst: PathBuf },
    /// Conversion par le lecteur universel.
    Convert { viewer: PathBuf, fmt: String, files: Vec<PathBuf> },
    /// Annulation d'une création : corbeille, sans nouvelle entrée d'annulation.
    TrashForUndo(Vec<PathBuf>),
    /// Annulation : remettre des fichiers déplacés/renommés à leur place (to → from).
    MoveBack(Vec<(PathBuf, PathBuf)>),
    /// Renommage en lot (ancien → nouveau).
    Rename(Vec<(PathBuf, PathBuf)>),
    /// Annulation : restaurer depuis la corbeille.
    Restore { paths: Vec<PathBuf>, since: SystemTime },
}

/// Ce qu'il faut pour défaire une opération.
#[derive(Clone)]
pub enum Undo {
    /// Fichiers créés (copie, nouveau dossier, zip…) → corbeille.
    Created(Vec<PathBuf>),
    /// Fichiers déplacés ou renommés (avant, après) → les remettre.
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Fichiers mis à la corbeille → les restaurer.
    Trashed { paths: Vec<PathBuf>, at: SystemTime },
}

impl Undo {
    pub fn label(&self) -> String {
        match self {
            Undo::Created(v) => format!("création de {} élément(s)", v.len()),
            Undo::Moved(v) => format!("déplacement/renommage de {} élément(s)", v.len()),
            Undo::Trashed { paths, .. } => format!("mise à la corbeille de {} élément(s)", paths.len()),
        }
    }
}

impl Op {
    fn label(&self) -> &'static str {
        match self {
            Op::Copy { .. } => "Copie",
            Op::Move { .. } => "Déplacement",
            Op::Trash(_) => "Mise à la corbeille",
            Op::Delete(_) => "Suppression",
            Op::Zip { .. } => "Compression",
            Op::Unzip { .. } => "Extraction",
            Op::Convert { .. } => "Conversion",
            Op::MoveBack(_) | Op::Restore { .. } | Op::TrashForUndo(_) => "Annulation",
            Op::Rename(_) => "Renommage",
        }
    }
}

pub enum OpEvent {
    Progress { label: &'static str, done: usize, total: usize, current: String },
    Finished { label: &'static str, errors: Vec<String>, touched: Vec<PathBuf>, undo: Option<Undo> },
}

pub struct Ops {
    tx: Sender<Op>,
    pub rx: Receiver<OpEvent>,
    pub cancel: Arc<AtomicBool>,
}

impl Ops {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, op_rx) = unbounded::<Op>();
        let (ev_tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            while let Ok(op) = op_rx.recv() {
                c2.store(false, Ordering::Relaxed);
                run(op, &ev_tx, &c2, &ctx);
                ctx.request_repaint();
            }
        });
        Ops { tx, rx, cancel }
    }

    pub fn submit(&self, op: Op) {
        let _ = self.tx.send(op);
    }
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn run(op: Op, ev: &Sender<OpEvent>, cancel: &AtomicBool, ctx: &egui::Context) {
    let label = op.label();
    let mut errors = Vec::new();
    let mut touched = Vec::new();
    let mut undo = None;
    let progress = |done: usize, total: usize, cur: &Path| {
        let _ = ev.send(OpEvent::Progress { label, done, total, current: name_of(cur) });
        ctx.request_repaint();
    };
    match op {
        Op::Copy { srcs, dst } | Op::Move { srcs, dst } if srcs.is_empty() || !dst.is_dir() => {
            let _ = dst;
            errors.push("Destination invalide".into());
        }
        Op::Copy { srcs, dst } => {
            let total = srcs.len();
            let mut created = Vec::new();
            for (i, s) in srcs.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    errors.push("Annulé".into());
                    break;
                }
                progress(i, total, s);
                let name = name_of(s);
                if dst.starts_with(s) {
                    errors.push(format!("{name} : impossible de copier un dossier dans lui-même"));
                    continue;
                }
                let target = unique_name(&dst, &name);
                match copy_any(s, &target, cancel) {
                    Ok(()) => {
                        touched.push(target.clone());
                        created.push(target);
                    }
                    Err(e) => errors.push(format!("{name} : {e}")),
                }
            }
            if !created.is_empty() {
                undo = Some(Undo::Created(created));
            }
        }
        Op::Move { srcs, dst } => {
            let total = srcs.len();
            let mut moved = Vec::new();
            for (i, s) in srcs.iter().enumerate() {
                progress(i, total, s);
                let name = name_of(s);
                if s.parent() == Some(dst.as_path()) {
                    continue;
                }
                if dst.starts_with(s) {
                    errors.push(format!("{name} : impossible de déplacer un dossier dans lui-même"));
                    continue;
                }
                let target = unique_name(&dst, &name);
                match move_any(s, &target, cancel) {
                    Ok(()) => {
                        touched.push(target.clone());
                        touched.push(s.clone());
                        moved.push((s.clone(), target));
                    }
                    Err(e) => errors.push(format!("{name} : {e}")),
                }
            }
            if !moved.is_empty() {
                undo = Some(Undo::Moved(moved));
            }
        }
        Op::MoveBack(pairs) => {
            // (to → from), en deux temps : l'annulation d'un renommage « décalé » (b→a, c→b) marche.
            let back: Vec<(PathBuf, PathBuf)> = pairs.into_iter().map(|(from, to)| (to, from)).collect();
            let (done, errs) = move_batch(&back, cancel, &progress);
            errors.extend(errs);
            touched.extend(done.into_iter().flat_map(|(a, b)| [a, b]));
        }
        Op::Rename(pairs) => {
            let (done, errs) = move_batch(&pairs, cancel, &progress);
            errors.extend(errs);
            touched.extend(done.iter().flat_map(|(a, b)| [a.clone(), b.clone()]));
            if !done.is_empty() {
                undo = Some(Undo::Moved(done));
            }
        }
        Op::Trash(paths) => {
            progress(0, paths.len(), paths.first().map(|p| p.as_path()).unwrap_or(Path::new("")));
            let at = SystemTime::now();
            match trash::delete_all(&paths) {
                Ok(()) => undo = Some(Undo::Trashed { paths: paths.clone(), at }),
                Err(e) => errors.push(e.to_string()),
            }
            touched = paths;
        }
        Op::Convert { viewer, fmt, files } => {
            progress(0, files.len(), files.first().map(|p| p.as_path()).unwrap_or(Path::new("")));
            let (created, errs) = crate::viewer::convert(&viewer, &fmt, &files);
            errors.extend(errs);
            touched = created.clone();
            if !created.is_empty() {
                undo = Some(Undo::Created(created));
            }
        }
        Op::TrashForUndo(paths) => {
            let existing: Vec<PathBuf> = paths.into_iter().filter(|p| p.exists()).collect();
            if !existing.is_empty() {
                if let Err(e) = trash::delete_all(&existing) {
                    errors.push(e.to_string());
                }
            }
            touched = existing;
        }
        Op::Restore { paths, since } => {
            #[cfg(any(windows, target_os = "linux"))]
            {
                use trash::os_limited;
                let since_s = since.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) - 5;
                match os_limited::list() {
                    Ok(items) => {
                        let wanted: Vec<_> = items
                            .into_iter()
                            .filter(|it| it.time_deleted >= since_s && paths.contains(&it.original_parent.join(&it.name)))
                            .collect();
                        if wanted.is_empty() {
                            errors.push("introuvable dans la corbeille".into());
                        } else if let Err(e) = os_limited::restore_all(wanted) {
                            errors.push(e.to_string());
                        }
                    }
                    Err(e) => errors.push(e.to_string()),
                }
            }
            touched = paths;
        }
        Op::Delete(paths) => {
            let total = paths.len();
            for (i, p) in paths.iter().enumerate() {
                progress(i, total, p);
                let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
                if let Err(e) = r {
                    errors.push(format!("{} : {e}", p.display()));
                }
            }
            touched = paths;
        }
        Op::Zip { srcs, dst } => {
            match zip_files(&srcs, &dst, cancel, &progress) {
                Ok(()) => {
                    touched.push(dst.clone());
                    undo = Some(Undo::Created(vec![dst]));
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&dst);
                    errors.push(e.to_string());
                }
            }
        }
        Op::Unzip { zip, dst } => {
            progress(0, 1, &zip);
            let r = std::fs::File::open(&zip)
                .map_err(|e| e.to_string())
                .and_then(|f| zip::ZipArchive::new(f).map_err(|e| e.to_string()))
                .and_then(|mut a| {
                    std::fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
                    a.extract(&dst).map_err(|e| e.to_string())
                });
            match r {
                Ok(()) => {
                    touched.push(dst.clone());
                    undo = Some(Undo::Created(vec![dst]));
                }
                Err(e) => errors.push(e),
            }
        }
    }
    let _ = ev.send(OpEvent::Finished { label, errors, touched, undo });
}

/// Déplace ou renomme plusieurs chemins (source → destination) en deux temps : chaque source
/// prend d'abord un nom temporaire à côté d'elle, puis va à sa destination. Permet a→b, b→c dans
/// un même lot (et leur annulation). Une destination qui existe (hors celles libérées par le lot)
/// n'est jamais écrasée : la source reprend son nom. Renvoie les paires faites et les erreurs.
pub fn move_batch(
    pairs: &[(PathBuf, PathBuf)],
    cancel: &AtomicBool,
    progress: &dyn Fn(usize, usize, &Path),
) -> (Vec<(PathBuf, PathBuf)>, Vec<String>) {
    let total = pairs.len();
    let mut errors = Vec::new();
    let mut temps = Vec::with_capacity(total);
    for (i, (src, _)) in pairs.iter().enumerate() {
        let tmp = src.with_file_name(format!(".explorateur-{}-{i}.tmp", std::process::id()));
        match std::fs::rename(src, &tmp) {
            Ok(()) => temps.push(Some(tmp)),
            Err(e) => {
                errors.push(format!("{} : {e}", name_of(src)));
                temps.push(None);
            }
        }
    }
    let mut done = Vec::new();
    for (i, ((src, dst), tmp)) in pairs.iter().zip(temps).enumerate() {
        let Some(tmp) = tmp else { continue };
        progress(i, total, dst);
        let res = if cancel.load(Ordering::Relaxed) {
            Err("annulé".to_string())
        } else if std::fs::symlink_metadata(dst).is_ok() {
            Err(format!("{} existe déjà", name_of(dst)))
        } else {
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            move_any(&tmp, dst, cancel).map_err(|e| format!("{} : {e}", name_of(dst)))
        };
        match res {
            Ok(()) => done.push((src.clone(), dst.clone())),
            Err(e) => {
                let _ = std::fs::rename(&tmp, src);
                errors.push(e);
            }
        }
    }
    (done, errors)
}

fn move_any(src: &Path, dst: &Path, cancel: &AtomicBool) -> std::io::Result<()> {
    // Même disque : renommage instantané. Sinon copie puis suppression.
    std::fs::rename(src, dst).or_else(|_| {
        copy_any(src, dst, cancel)?;
        if src.is_dir() { std::fs::remove_dir_all(src) } else { std::fs::remove_file(src) }
    })
}

fn copy_any(src: &Path, dst: &Path, cancel: &AtomicBool) -> std::io::Result<()> {
    let meta = std::fs::metadata(src)?;
    if meta.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            if cancel.load(Ordering::Relaxed) {
                return Err(std::io::Error::other("annulé"));
            }
            let e = e?;
            copy_any(&e.path(), &dst.join(e.file_name()), cancel)?;
        }
        Ok(())
    } else {
        // Sous Windows, std::fs::copy utilise CopyFileExW (rapide, garde les attributs).
        std::fs::copy(src, dst).map(|_| ())
    }
}

fn zip_files(srcs: &[PathBuf], dst: &Path, cancel: &AtomicBool, progress: &dyn Fn(usize, usize, &Path)) -> std::io::Result<()> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let file = std::fs::File::create(dst)?;
    let mut w = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // Liste à plat de (chemin disque, nom dans l'archive)
    let mut items: Vec<(PathBuf, String)> = Vec::new();
    for s in srcs {
        let base = s.parent().unwrap_or(Path::new(""));
        let mut stack = vec![s.clone()];
        while let Some(p) = stack.pop() {
            let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            if p.is_dir() {
                items.push((p.clone(), format!("{rel}/")));
                if let Ok(rd) = std::fs::read_dir(&p) {
                    stack.extend(rd.flatten().map(|e| e.path()));
                }
            } else {
                items.push((p.clone(), rel));
            }
        }
    }
    let total = items.len();
    for (i, (p, name)) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("annulé"));
        }
        if i % 16 == 0 {
            progress(i, total, p);
        }
        if name.ends_with('/') {
            w.add_directory(name.trim_end_matches('/'), opts).map_err(std::io::Error::other)?;
        } else {
            w.start_file(name, opts).map_err(std::io::Error::other)?;
            let mut f = std::fs::File::open(p)?;
            std::io::copy(&mut f, &mut w)?;
        }
    }
    w.finish().map_err(std::io::Error::other)?.flush()?;
    Ok(())
}
