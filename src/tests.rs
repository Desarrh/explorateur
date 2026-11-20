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
fn conversion_par_le_lecteur() {
    let Some(viewer) = crate::viewer::find() else {
        eprintln!("lecteur universel absent : test sauté");
        return;
    };
    let d = tmpdir("convert");
    let src = d.join("carre.png");
    image::RgbaImage::from_pixel(32, 32, image::Rgba([46, 94, 158, 255])).save(&src).unwrap();
    let (created, errors) = crate::viewer::convert(&viewer, "webp", &[src]);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(created.len(), 1, "{created:?}");
    assert_eq!(created[0].extension().unwrap(), "webp");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn index_recherche_et_sauvegarde() {
    let d = tmpdir("index");
    std::fs::create_dir_all(d.join("Projets").join("PsychoNight")).unwrap();
    std::fs::write(d.join("Projets").join("PsychoNight").join("rapport_final.docx"), b"x").unwrap();
    std::fs::write(d.join("Projets").join("notes.txt"), b"x").unwrap();
    std::fs::write(d.join("final-cut.mp4"), b"x").unwrap();
    let ix = crate::index::scan_dir_for_test(&d);
    // insensible à la casse, sous-chaîne, chemin complet reconstruit
    let hits = ix.search("PSYCHO", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(crate::index::real_case(&hits[0].path), d.join("Projets").join("PsychoNight"));
    assert!(hits[0].is_dir);
    let hits = ix.search("final", 10);
    let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"rapport_final.docx") && names.contains(&"final-cut.mp4"));
    assert_eq!(names[0], "final-cut.mp4", "le préfixe passe devant");
    // sauvegarde / rechargement
    let f = d.join("index.bin");
    let back = crate::index::roundtrip(&ix, &f).expect("relecture");
    assert_eq!(back.len(), ix.len());
    assert_eq!(crate::index::real_case(&back.search("notes", 5)[0].path), d.join("Projets").join("notes.txt"));
    let _ = std::fs::remove_dir_all(d);
}

/// `cargo test --release bench_index -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_index() {
    let (d, t) = crate::index::full_scan_for_test();
    println!("index de tous les disques fixes : {} éléments en {t:?}", d.len());
    for q in ["psycho", "readme", "zz", ".rs"] {
        let t0 = std::time::Instant::now();
        let hits = d.search(q, 40);
        println!("  recherche « {q} » : {} résultats en {:?} (ex. {:?})", hits.len(), t0.elapsed(), hits.first().map(|h| h.path.display().to_string()));
    }
    println!("  mémoire de l'index ≈ {} Mo", d.mem_bytes() / 1_000_000);
}

/// Tube inexistant : le test ne doit jamais parler au vrai lecteur s'il est en veille.
const FAUX_TUBE: &str = r"\\.\pipe\explorateur-test-inexistant";

#[test]
fn apercu_rapide_repli_silencieux() {
    let ctx = egui::Context::default();
    let f = PathBuf::from(r"C:\Windows\win.ini");
    // 1. lecteur introuvable → repli immédiat
    let (tx, rx) = crossbeam_channel::unbounded();
    crate::viewer::quick_look_via(FAUX_TUBE, Path::new(r"C:\nexiste\pas\lecteur.exe"), &f, tx, ctx.clone());
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(2)).ok(), Some(f.clone()));
    // 2. « lecteur » qui sort en erreur tout de suite (where.exe refuse --apercu) → repli
    let (tx, rx) = crossbeam_channel::unbounded();
    crate::viewer::quick_look_via(FAUX_TUBE, Path::new(r"C:\Windows\System32\where.exe"), &f, tx, ctx);
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(3)).ok(), Some(f));
}

/// Mesure sans fenêtre (règle n°0 : rien à l'écran, aucun focus pris) : l'app complète tourne
/// avec egui + le rendu CPU dans un tampon en mémoire, sur un dossier de 10 000 fichiers.
/// Mesure le temps jusqu'à la première image où la liste est affichée, et la mémoire du processus.
/// `cargo test --release bench_affichage -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_affichage() {
    use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
    let d = tmpdir("bench10k");
    for i in 0..10_000 {
        std::fs::write(d.join(format!("fichier {i:05}.txt")), b"x").unwrap();
    }
    let (w, h) = (1280usize, 800usize);
    let mut buf = vec![[0u8; 4]; w * h];
    let mut renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra);
    let frame = |ctx: &egui::Context, app: &mut crate::app::App, renderer: &mut EguiSoftwareRender, buf: &mut Vec<[u8; 4]>| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            app.tick();
            app.draw(ui);
            app.apply_actions();
        });
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        renderer.render(&mut BufferMutRef::new(buf, w, h), &prims, &out.textures_delta, out.pixels_per_point);
    };
    let t0 = std::time::Instant::now();
    let ctx = egui::Context::default();
    let s = crate::app::Settings {
        tabs: vec![d.clone()],
        global_index: false,
        auto_update: false,
        folder_sizes: false,
        ..Default::default()
    };
    let mut app = crate::app::App::new(&ctx, s);
    let t_new = t0.elapsed();
    frame(&ctx, &mut app, &mut renderer, &mut buf);
    let t_first = t0.elapsed();
    // On attend que la liste soit chargée, triée et dessinée.
    let mut frames = 1;
    while (app.tab().loading || app.tab().view.len() < 10_000) && t0.elapsed().as_secs() < 10 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        frame(&ctx, &mut app, &mut renderer, &mut buf);
        frames += 1;
    }
    frame(&ctx, &mut app, &mut renderer, &mut buf);
    let t_list = t0.elapsed();
    // image suivante, sans changement : coût d'un rafraîchissement à l'état stable
    let t1 = std::time::Instant::now();
    frame(&ctx, &mut app, &mut renderer, &mut buf);
    let t_steady = t1.elapsed();
    let ram = mem_mb();
    println!(
        "10 000 fichiers : App::new {:.1} ms, 1re image {:.1} ms, liste affichée {:.1} ms ({frames} images), image stable {:.1} ms, RAM du processus {ram:.0} Mo (privé)",
        t_new.as_secs_f64() * 1e3,
        t_first.as_secs_f64() * 1e3,
        t_list.as_secs_f64() * 1e3,
        t_steady.as_secs_f64() * 1e3,
    );
    assert_eq!(app.tab().view.len(), 10_000);
    drop(app);
    let _ = std::fs::remove_dir_all(d);
}

/// Mémoire privée du processus (Mo).
fn mem_mb() -> f64 {
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &format!("(Get-Process -Id {}).PrivateMemorySize64", std::process::id())])
        .output()
        .ok();
    out.and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok()).unwrap_or(0.0) / 1_048_576.0
}

#[test]
fn comparaison_de_versions() {
    use socle::update::is_newer;
    assert!(is_newer("v0.3.0", "0.2.0"));
    assert!(is_newer("v0.2.1", "0.2.0"));
    assert!(is_newer("1.0", "0.9.9"));
    assert!(!is_newer("v0.2.0", "0.2.0"));
    assert!(!is_newer("v0.1.9", "0.2.0"));
    assert!(!is_newer("nimporte", "0.2.0"));
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

#[test]
fn miniature_avec_cache_disque() {
    let d = tmpdir("miniature");
    let png = d.join("a.png");
    image::RgbaImage::from_pixel(600, 300, image::Rgba([200, 50, 50, 255])).save(&png).unwrap();
    let disk = socle::images::DiskCache { dir: d.join("cache") };
    let img = crate::thumbs::decode(&png, 128, &disk).expect("miniature");
    assert_eq!(img.size, [128, 64]);
    // 2ᵉ passage : servie par le cache disque
    assert!(std::fs::read_dir(&disk.dir).unwrap().count() >= 1);
    assert_eq!(crate::thumbs::decode(&png, 128, &disk).unwrap().size, [128, 64]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn glisser_objet_de_donnees_du_shell() {
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    let d = tmpdir("glisser");
    let (a, b) = (d.join("a.txt"), d.join("sous").join("b.txt"));
    std::fs::create_dir_all(b.parent().unwrap()).unwrap();
    std::fs::write(&a, "a").unwrap();
    std::fs::write(&b, "b").unwrap();
    unsafe {
        let _ = windows::Win32::System::Ole::OleInitialize(None);
    }
    // Fichiers de deux dossiers différents : l'objet doit proposer CF_HDROP (ce que lisent Bureau, Discord…).
    let obj = crate::winx::shell_data_object(&[a, b]).expect("objet de données");
    let fmt = FORMATETC {
        cfFormat: windows::Win32::System::Ole::CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    assert!(unsafe { obj.QueryGetData(&fmt) }.is_ok());
    let _ = std::fs::remove_dir_all(&d);
}
