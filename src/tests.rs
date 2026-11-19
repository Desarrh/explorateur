//! Tests des fonctions sans interface : renommage en lot, zip, annulation, tailles.
use crate::app::BatchRename;
use crate::extras::batch_names;
use std::path::{Path, PathBuf};

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("explorateur-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn batch(paths: Vec<PathBuf>, pattern: &str, find: &str, replace: &str, regex: bool) -> BatchRename {
    BatchRename { paths, pattern: pattern.into(), find: find.into(), replace: replace.into(), regex, start: 1, fresh: false }
}

#[test]
fn renommage_motif_et_numeros() {
    let d = tmpdir("batch");
    let files: Vec<PathBuf> = (1..=3).map(|i| d.join(format!("photo {i}.jpg"))).collect();
    for f in &files {
        std::fs::write(f, b"x").unwrap();
    }
    let names: Vec<String> = batch_names(&batch(files.clone(), "Voyage {n:2}", "", "", false)).into_iter().map(|r| r.unwrap()).collect();
    assert_eq!(names, ["Voyage 01.jpg", "Voyage 02.jpg", "Voyage 03.jpg"]);
    let names: Vec<String> = batch_names(&batch(files.clone(), "{nom}", "photo", "img", false)).into_iter().map(|r| r.unwrap()).collect();
    assert_eq!(names, ["img 1.jpg", "img 2.jpg", "img 3.jpg"]);
    let names: Vec<String> = batch_names(&batch(files.clone(), "{nom}", r"photo (\d)", "$1-photo", true)).into_iter().map(|r| r.unwrap()).collect();
    assert_eq!(names, ["1-photo.jpg", "2-photo.jpg", "3-photo.jpg"]);
    let names: Vec<String> = batch_names(&batch(files, "{nom}_{n}.{ext}", "", "", false)).into_iter().map(|r| r.unwrap()).collect();
    assert_eq!(names, ["photo 1_1.jpg", "photo 2_2.jpg", "photo 3_3.jpg"]);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn regex_invalide_signalee() {
    let r = batch_names(&batch(vec![PathBuf::from("a.txt")], "{nom}", "(", "", true));
    assert!(r[0].is_err());
}

fn run_op(op: crate::ops::Op) -> (Vec<String>, Option<crate::ops::Undo>) {
    let ctx = egui::Context::default();
    let ops = crate::ops::Ops::new(&ctx);
    ops.submit(op);
    loop {
        match ops.rx.recv_timeout(std::time::Duration::from_secs(20)).expect("opération trop longue") {
            crate::ops::OpEvent::Finished { errors, undo, .. } => return (errors, undo),
            _ => {}
        }
    }
}

#[test]
fn zip_puis_extraction() {
    let d = tmpdir("zip");
    let src = d.join("projet");
    std::fs::create_dir_all(src.join("sous")).unwrap();
    std::fs::write(src.join("a.txt"), b"bonjour").unwrap();
    std::fs::write(src.join("sous").join("b.txt"), vec![7u8; 50_000]).unwrap();
    let zip = d.join("projet.zip");
    let (err, undo) = run_op(crate::ops::Op::Zip { srcs: vec![src.clone()], dst: zip.clone() });
    assert!(err.is_empty(), "{err:?}");
    assert!(matches!(undo, Some(crate::ops::Undo::Created(_))));
    assert!(std::fs::metadata(&zip).unwrap().len() > 0);
    let out = d.join("extrait");
    let (err, _) = run_op(crate::ops::Op::Unzip { zip, dst: out.clone() });
    assert!(err.is_empty(), "{err:?}");
    assert_eq!(std::fs::read(out.join("projet").join("a.txt")).unwrap(), b"bonjour");
    assert_eq!(std::fs::read(out.join("projet").join("sous").join("b.txt")).unwrap().len(), 50_000);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn deplacement_puis_annulation() {
    let d = tmpdir("undo");
    std::fs::create_dir_all(d.join("dst")).unwrap();
    let f = d.join("fichier.txt");
    std::fs::write(&f, b"x").unwrap();
    let (err, undo) = run_op(crate::ops::Op::Move { srcs: vec![f.clone()], dst: d.join("dst") });
    assert!(err.is_empty(), "{err:?}");
    assert!(!f.exists() && d.join("dst").join("fichier.txt").exists());
    let Some(crate::ops::Undo::Moved(pairs)) = undo else { panic!("pas d'annulation") };
    let (err, _) = run_op(crate::ops::Op::MoveBack(pairs));
    assert!(err.is_empty(), "{err:?}");
    assert!(f.exists() && !d.join("dst").join("fichier.txt").exists());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn copie_puis_annulation_corbeille() {
    let d = tmpdir("copyundo");
    std::fs::create_dir_all(d.join("dst")).unwrap();
    let f = d.join("doc.txt");
    std::fs::write(&f, b"x").unwrap();
    let (err, undo) = run_op(crate::ops::Op::Copy { srcs: vec![f.clone()], dst: d.join("dst") });
    assert!(err.is_empty(), "{err:?}");
    let Some(crate::ops::Undo::Created(created)) = undo else { panic!("pas d'annulation") };
    assert!(created[0].exists());
    let (err, _) = run_op(crate::ops::Op::TrashForUndo(created.clone()));
    assert!(err.is_empty(), "{err:?}");
    assert!(!created[0].exists() && f.exists());
    let _ = std::fs::remove_dir_all(d);
}

/// Compare l'ancien calcul (un seul thread, read_dir + metadata) au nouveau.
/// `cargo test --release bench_taille -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_taille() {
    let dir = PathBuf::from(std::env::var("BENCH_DIR").unwrap_or_else(|_| r"C:\Program Files".into()));
    fn naive(d: &Path) -> u64 {
        let mut t = 0;
        let mut stack = vec![d.to_path_buf()];
        while let Some(p) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&p) else { continue };
            for e in rd.flatten() {
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    stack.push(e.path());
                } else if let Ok(m) = e.metadata() {
                    t += m.len();
                }
            }
        }
        t
    }
    let _ = naive(&dir); // réchauffe le cache disque pour comparer à armes égales
    let t0 = std::time::Instant::now();
    let a = naive(&dir);
    let old = t0.elapsed();
    let (b, cached, new) = crate::sizes::bench(&dir);
    println!("{}: ancien {:?} ({a} o) | nouveau {:?} ({b} o, {cached} sous-dossiers mis en cache)", dir.display(), old, new);
}

#[test]
fn taille_de_dossier() {
    let d = tmpdir("sizes");
    std::fs::create_dir_all(d.join("a").join("b")).unwrap();
    std::fs::write(d.join("a").join("x.bin"), vec![0u8; 1000]).unwrap();
    std::fs::write(d.join("a").join("b").join("y.bin"), vec![0u8; 2345]).unwrap();
    let ctx = egui::Context::default();
    let mut s = crate::sizes::Sizes::new(&ctx);
    s.focus(vec![d.join("a")]);
    let t0 = std::time::Instant::now();
    while !s.map.contains_key(&d.join("a")) && t0.elapsed().as_secs() < 10 {
        s.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(s.map.get(&d.join("a")).copied(), Some(3345));
    assert!(s.invalidate(&d.join("a").join("x.bin")));
    assert!(!s.map.contains_key(Path::new(&d.join("a"))));
    let _ = std::fs::remove_dir_all(d);
}
