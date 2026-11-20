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


#[test]
fn apercu_rapide_repli_silencieux() {
    let ctx = egui::Context::default();
    let f = PathBuf::from(r"C:\Windows\win.ini");
    // 1. lecteur introuvable → repli immédiat
    let (tx, rx) = crossbeam_channel::unbounded();
    let wait = std::time::Duration::from_secs(5); // large : le PC peut être chargé pendant les tests
    crate::viewer::quick_look_within(Path::new(r"C:\nexiste\pas\lecteur.exe"), &f, tx, ctx.clone(), wait);
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(2)).ok(), Some(f.clone()));
    // 2. « lecteur » qui sort en erreur tout de suite (where.exe refuse --apercu) → repli
    let (tx, rx) = crossbeam_channel::unbounded();
    crate::viewer::quick_look_within(Path::new(r"C:\Windows\System32\where.exe"), &f, tx, ctx, wait);
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(8)).ok(), Some(f));
}

/// Mesure sans fenêtre (règle n°0 : rien à l'écran, aucun focus pris) : l'app complète tourne
/// avec egui + le rendu CPU dans un tampon en mémoire, sur un dossier de 10 000 fichiers.
/// Mesure le temps jusqu'à la première image où la liste est affichée, et la mémoire du processus.
/// `cargo test --release bench_affichage -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_affichage() {
    // Mode test : nom d'app à part (journal de plantage, caches), jamais l'état de l'utilisateur.
    unsafe { std::env::set_var("SOCLE_TEST", "1") };
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

/// git dans un dossier de test (identité locale à la commande, rien dans la config de l'utilisateur).
fn git_t(dir: &std::path::Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git");
    assert!(o.status.success(), "git {args:?} : {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn rafale_instantanes_diff_restauration() {
    use crate::git::{burst_restore, snapshot};
    let d = tmpdir("rafale");
    git_t(&d, &["init", "-q"]);
    std::fs::write(d.join("a.txt"), "v1\n").unwrap();
    std::fs::write(d.join("c.txt"), "garde\n").unwrap();
    git_t(&d, &["add", "-A"]);
    git_t(&d, &["commit", "-q", "-m", "init"]);
    // Changement non commité AVANT la passe : il doit survivre à l'annulation de la passe.
    std::fs::write(d.join("a.txt"), "v2\n").unwrap();
    let (base, _) = snapshot(&d, None, &[]).expect("instantané de base");
    let index_avant = git_t(&d, &["diff", "--cached", "--stat"]);

    // La passe : a modifié, b créé, c supprimé.
    std::fs::write(d.join("a.txt"), "v3\n").unwrap();
    std::fs::write(d.join("b.txt"), "nouveau\n").unwrap();
    std::fs::remove_file(d.join("c.txt")).unwrap();
    let files: Vec<String> = ["a.txt", "b.txt", "c.txt"].iter().map(|s| s.to_string()).collect();
    let (after, skipped) = snapshot(&d, Some(&base), &files).expect("instantané de fin");
    assert!(skipped.is_empty());
    let diff = git_t(&d, &["diff", &base, &after]);
    assert!(diff.contains("-v2") && diff.contains("+v3"), "{diff}");
    assert!(diff.contains("+nouveau") && diff.contains("-garde"), "{diff}");
    let lines = crate::git::parse_diff(&diff);
    assert_eq!(lines.iter().filter(|l| l.kind == crate::git::LineKind::File).count(), 3);

    // Annuler la passe : retour à l'état d'avant la passe (v2, pas le commit v1).
    let (undo, post) = burst_restore(&d, &base, &after, &files).expect("restauration");
    assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap().trim(), "v2");
    assert!(!d.join("b.txt").exists());
    assert_eq!(std::fs::read_to_string(d.join("c.txt")).unwrap().trim(), "garde");
    // Rétablir.
    burst_restore(&d, &undo, &post, &files).expect("rétablir");
    assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap().trim(), "v3");
    assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap().trim(), "nouveau");
    assert!(!d.join("c.txt").exists());
    // L'index et l'historique du dépôt n'ont pas bougé.
    assert_eq!(git_t(&d, &["diff", "--cached", "--stat"]), index_avant);
    assert_eq!(git_t(&d, &["rev-list", "--count", "HEAD"]).trim(), "1");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn rafale_regroupement() {
    use crate::app::Act;
    let mut b = crate::bursts::Bursts::default();
    let root = std::path::Path::new("C:/depot");
    assert!(b.watch_repo(root).is_some());
    assert!(b.watch_repo(root).is_none(), "base demandée une seule fois");
    b.record(root, "a.rs".into(), Act::Modified);
    b.record(root, "tmp.rs".into(), Act::Created);
    b.record(root, "tmp.rs".into(), Act::Removed); // créé puis supprimé : rien de net
    b.record(root, "n.rs".into(), Act::Created);
    b.record(root, "n.rs".into(), Act::Modified); // reste « créé »
    assert_eq!(b.list.len(), 1, "une seule passe tant qu'il n'y a pas 20 s de silence");
    let p = &b.list[0];
    assert_eq!(p.files.len(), 2);
    assert!(matches!(p.files["n.rs"], Act::Created));
    assert!(b.tick().is_empty(), "pas encore close");
    b.mute(root, &["a.rs".to_string()]);
    b.record(root, "a.rs".into(), Act::Removed);
    assert!(matches!(b.list[0].files["a.rs"], Act::Modified), "événements d'une restauration ignorés");
}

/// Rendu sans fenêtre d'une image de l'app en PNG (règle n° 0 : rien à l'écran).
/// `prepare` règle l'app ; `ready` dit quand l'image est prête.
#[cfg(test)]
pub fn capture_png(
    out: &Path,
    s: crate::app::Settings,
    w: usize,
    h: usize,
    mut prepare: impl FnMut(&mut crate::app::App, u32),
    ready: impl Fn(&crate::app::App) -> bool,
) {
    use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
    // Mode test : nom d'app à part (journal de plantage, cache), jamais l'état de l'utilisateur.
    unsafe { std::env::set_var("SOCLE_TEST", "1") };
    let mut buf = vec![[0u8; 4]; w * h];
    let mut renderer = EguiSoftwareRender::new(ColorFieldOrder::Rgba);
    let ctx = egui::Context::default();
    let mut app = crate::app::App::new(&ctx, s);
    let t0 = std::time::Instant::now();
    let mut n = 0u32;
    loop {
        prepare(&mut app, n);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))),
            ..Default::default()
        };
        let o = ctx.run_ui(raw, |ui| {
            app.tick();
            app.draw(ui);
            app.apply_actions();
        });
        let prims = ctx.tessellate(o.shapes, o.pixels_per_point);
        buf.iter_mut().for_each(|px| *px = [app.pal.bg.r(), app.pal.bg.g(), app.pal.bg.b(), 255]);
        renderer.render(&mut BufferMutRef::new(&mut buf, w, h), &prims, &o.textures_delta, o.pixels_per_point);
        n += 1;
        if (ready(&app) && n > 3) || t0.elapsed().as_secs() > 20 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let raw: Vec<u8> = buf.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    image::RgbaImage::from_raw(w as u32, h as u32, raw).unwrap().save(out).unwrap();
}

/// Capture de la vue Dev avec des passes d'agent (E1).
/// `CAPTURE_DIR=<dossier> cargo test --release capture_passes -- --ignored`
#[test]
#[ignore]
fn capture_passes() {
    use crate::app::Act;
    let outdir = PathBuf::from(std::env::var("CAPTURE_DIR").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned()));
    let d = tmpdir("capture-passes");
    git_t(&d, &["init", "-q"]);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("src").join("main.rs"), "fn main() {\n    println!(\"bonjour\");\n}\n").unwrap();
    std::fs::write(d.join("README.md"), "# Démo\n").unwrap();
    git_t(&d, &["add", "-A"]);
    git_t(&d, &["commit", "-q", "-m", "init"]);
    let root = PathBuf::from(git_t(&d, &["rev-parse", "--show-toplevel"]).trim().replace('/', "\\"));
    for dark in [false, true] {
        let s = crate::app::Settings { tabs: vec![d.clone()], dev: true, dark, global_index: false, auto_update: false, folder_sizes: false, ..Default::default() };
        let r2 = root.clone();
        let d2 = d.clone();
        let mut stage = 0;
        capture_png(
            &outdir.join(format!("passes-{}.png", if dark { "sombre" } else { "clair" })),
            s,
            1400,
            860,
            move |app, _| {
                // 1. attendre le dépôt et son instantané de base ; 2. simuler une passe ; 3. l'afficher.
                if stage == 0 && app.repo.is_some() && app.bursts.snapshot_of(&r2).is_some() {
                    std::fs::write(d2.join("src").join("main.rs"), "fn main() {\n    println!(\"bonjour, monde\");\n    let x = 42;\n}\n").unwrap();
                    std::fs::write(d2.join("src").join("lib.rs"), "pub fn aide() {}\n").unwrap();
                    app.bursts.record(&r2, "src/main.rs".into(), Act::Modified);
                    app.bursts.record(&r2, "src/lib.rs".into(), Act::Created);
                    app.bursts.record(&r2, "README.md".into(), Act::Modified);
                    let id = app.bursts.list[0].id;
                    app.burst_expanded.insert(id);
                    app.actions.push(crate::app::Action::BurstView(id));
                    stage = 1;
                }
            },
            |app| app.burst_diff.as_ref().map(|d| !d.1.is_empty()).unwrap_or(false),
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// Fixe la date de modification d'un fichier (il y a `mins` minutes).
fn set_age(p: &Path, mins: u64) {
    let t = std::time::SystemTime::now() - std::time::Duration::from_secs(mins * 60);
    std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
}

#[test]
fn recents_index_dates_et_zones_bruyantes() {
    let d = tmpdir("recents-index");
    let w = |rel: &str, mins: u64| {
        let p = d.join(rel.replace('/', "\\"));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        set_age(&p, mins);
    };
    w("Vidéos/export final.mp4", 3);
    w("Téléchargements/facture.pdf", 30);
    w("Projets/vieux.txt", 60 * 24 * 40);
    w("AppData/Local/cache.bin", 1); // zone bruyante
    w("Projets/.git/index", 1); // dossier caché
    w("Projets/build.log", 1); // fichier technique
    w("Projets/target/release/app.exe", 1); // build
    let ix = crate::index::scan_dir_for_test(&d);
    let r = ix.recent(10);
    let names: Vec<String> = r.iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["export final.mp4", "facture.pdf", "vieux.txt"], "du plus récent au plus ancien, sans le bruit");
    // Les dates survivent à la sauvegarde (format v2).
    let f = d.join("index.bin");
    let back = crate::index::roundtrip(&ix, &f).expect("relecture");
    assert_eq!(back.recent(10).len(), 3);
    assert_eq!(back.recent(1)[0].1, r[0].1);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn recents_filtres() {
    use crate::recents::noisy_path;
    assert!(!noisy_path(Path::new(r"Users\moi\Vidéos\montage.mp4")));
    assert!(noisy_path(Path::new(r"Users\moi\AppData\Roaming\x.json")));
    assert!(noisy_path(Path::new(r"Program Files\App\data.bin")));
    assert!(noisy_path(Path::new(r"Dev\projet\node_modules\a.js")));
    assert!(noisy_path(Path::new(r"Users\moi\Documents\~$rapport.docx")));
    assert!(noisy_path(Path::new(r"Users\moi\Downloads\film.mkv.crdownload")));
    assert!(!noisy_path(Path::new(r"Users\moi\Downloads\film.mkv")));
    assert!(crate::recents::is_recents(&crate::recents::path()));
    assert!(!crate::recents::path().is_dir(), "chemin virtuel, jamais un vrai dossier");
}

#[test]
fn recents_surveillance_en_direct() {
    let d = tmpdir("recents-live");
    std::fs::create_dir_all(d.join("Captures")).unwrap();
    std::fs::create_dir_all(d.join(".cache")).unwrap();
    // Surveillance d'un dossier de test (jamais des vrais disques dans un test).
    let live = crate::recents::Live::start(&[d.clone()]);
    std::thread::sleep(std::time::Duration::from_millis(200));
    std::fs::write(d.join("Captures").join("partie 1.png"), b"x").unwrap();
    std::fs::write(d.join(".cache").join("bruit.bin"), b"x").unwrap();
    let t0 = std::time::Instant::now();
    while live.generation.load(std::sync::atomic::Ordering::Relaxed) == 0 && t0.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    let v = crate::recents::collect(None, Some(&live.map()), 50);
    let names: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["partie 1.png"]);
    assert!(v[0].display.as_deref().unwrap().contains("Captures"));
    drop(live);
    let _ = std::fs::remove_dir_all(&d);
}

/// Capture de l'onglet « Récents » (E2), sans fenêtre.
/// `CAPTURE_DIR=<dossier> cargo test --release capture_recents -- --ignored`
#[test]
#[ignore]
fn capture_recents() {
    let outdir = PathBuf::from(std::env::var("CAPTURE_DIR").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned()));
    let d = tmpdir("capture-recents");
    for (rel, mins) in [
        ("Vidéos/Exports/montage v3 final.mp4", 2u64),
        ("Images/Captures/Capture 2026-11-20 191502.png", 5),
        ("Téléchargements/facture-novembre.pdf", 18),
        ("Documents/notes réunion.docx", 47),
        ("Dev/explorateur/src/recents.rs", 65),
        ("Musique/mix.flac", 300),
    ] {
        let p = d.join(rel.replace('/', "\\"));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, vec![0u8; 1200 * (mins as usize + 1)]).unwrap();
        set_age(&p, mins);
    }
    let live = crate::recents::Live::start(&[d.clone()]);
    for f in ["Vidéos/Exports/montage v3 final.mp4", "Images/Captures/Capture 2026-11-20 191502.png"] {
        let p = d.join(f.replace('/', "\\"));
        std::fs::write(&p, b"maj").unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    // Les autres fichiers viennent d'un « index » de test.
    let ix = std::sync::Arc::new(std::sync::RwLock::new(crate::index::scan_dir_for_test(&d)));
    let s = crate::app::Settings { tabs: vec![crate::recents::path()], global_index: false, auto_update: false, folder_sizes: false, ..Default::default() };
    let mut live = Some(live);
    capture_png(
        &outdir.join("recents.png"),
        s,
        1400,
        700,
        move |app, n| {
            if n == 0 {
                app.s.global_index = true; // sans lancer le vrai index
                app.recents_live = live.take();
                app.index = Some(crate::index::Index { data: ix.clone(), scanning: Default::default() });
                app.actions.push(crate::app::Action::Refresh);
            }
        },
        |app| app.tab().view.len() >= 6,
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn carte_disposition_en_carres() {
    use crate::spacemap::squarify;
    let area = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 500.0));
    let vals = [500.0, 250.0, 120.0, 60.0, 40.0, 20.0, 6.0, 3.0, 1.0];
    let rs = squarify(&vals, area);
    let total: f64 = vals.iter().sum();
    let a_tot = (area.width() * area.height()) as f64;
    for (v, r) in vals.iter().zip(&rs) {
        // aire proportionnelle (à 1 % près) et dans la zone
        let a = (r.width() * r.height()) as f64;
        assert!((a - v / total * a_tot).abs() < a_tot * 0.01, "{v} → {r:?}");
        assert!(area.expand(0.01).contains_rect(*r), "{r:?} hors de {area:?}");
    }
    for i in 0..rs.len() {
        for j in i + 1..rs.len() {
            let inter = rs[i].intersect(rs[j]);
            assert!(inter.width() <= 0.01 || inter.height() <= 0.01, "chevauchement {i}/{j}");
        }
    }
    // « en carrés » : le plus gros rectangle n'est pas une bande fine
    let r0 = rs[0];
    assert!(r0.width().max(r0.height()) / r0.width().min(r0.height()) < 3.0, "{r0:?}");
}

#[test]
fn carte_arbre_et_regroupement() {
    let d = tmpdir("carte");
    std::fs::create_dir_all(d.join("Jeux").join("Gros")).unwrap();
    std::fs::write(d.join("Jeux").join("Gros").join("data.pak"), vec![0u8; 400_000]).unwrap();
    std::fs::write(d.join("Jeux").join("lanceur.exe"), vec![0u8; 50_000]).unwrap();
    std::fs::write(d.join("film.mkv"), vec![0u8; 300_000]).unwrap();
    for i in 0..30 {
        std::fs::write(d.join(format!("miette{i}.txt")), b"x").unwrap();
    }
    let mut sizes = std::collections::HashMap::new();
    sizes.insert(d.join("Jeux"), 450_000u64);
    sizes.insert(d.join("Jeux").join("Gros"), 400_000u64);
    let t = crate::spacemap::build(&d, &sizes);
    assert_eq!(t.size, 450_000 + 300_000 + 30);
    let names: Vec<&str> = t.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names[..2], ["Jeux", "film.mkv"], "du plus gros au plus petit");
    assert_eq!(t.children.last().unwrap().rest, Some(30), "les miettes sont regroupées : {names:?}");
    let jeux = &t.children[0];
    assert_eq!(jeux.children[0].name, "Gros", "un niveau de plus pour les gros dossiers");
    assert_eq!(jeux.children[0].children[0].name, "data.pak");
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0));
    let tiles = crate::spacemap::layout(&t, area);
    assert!(tiles.iter().any(|x| x.name == "data.pak" && x.depth == 2), "contenu imbriqué dessiné");
    let _ = std::fs::remove_dir_all(&d);
}

/// Capture de la carte de l'espace (E3), sans fenêtre.
/// `CAPTURE_DIR=<dossier> cargo test --release capture_carte -- --ignored`
#[test]
#[ignore]
fn capture_carte() {
    let outdir = PathBuf::from(std::env::var("CAPTURE_DIR").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned()));
    let d = tmpdir("capture-carte");
    let files: [(&str, usize); 14] = [
        ("SteamLibrary/Jeu A/data/textures.pak", 9_000_000),
        ("SteamLibrary/Jeu A/data/audio.pak", 3_500_000),
        ("SteamLibrary/Jeu A/jeu.exe", 900_000),
        ("SteamLibrary/Jeu B/contenu.pak", 6_000_000),
        ("SteamLibrary/Jeu B/films/intro.mp4", 2_500_000),
        ("Vidéos/rush 01.mp4", 5_000_000),
        ("Vidéos/rush 02.mp4", 3_200_000),
        ("Vidéos/export.mp4", 1_400_000),
        ("Musique/album.flac", 1_800_000),
        ("Images/photo 1.png", 700_000),
        ("Images/photo 2.jpg", 500_000),
        ("Archives/sauvegarde.zip", 2_200_000),
        ("notes.txt", 20_000),
        ("installeur.msi", 1_100_000),
    ];
    for (rel, n) in files {
        let p = d.join(rel.replace('/', "\\"));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, vec![0u8; n]).unwrap();
    }
    for dark in [false, true] {
        let s = crate::app::Settings { tabs: vec![d.clone()], space_map: true, dark, preview: false, global_index: false, auto_update: false, ..Default::default() };
        capture_png(
            &outdir.join(format!("carte-{}.png", if dark { "sombre" } else { "clair" })),
            s,
            1400,
            820,
            |_, _| {},
            |app| {
                app.sizes.pending_count() == 0
                    && app.space_maps.values().any(|m| m.tree.as_ref().map(|t| t.size > 30_000_000).unwrap_or(false))
            },
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// Affiche si le processus de test est dans un conteneur MSIX (diagnostic, toujours vert).
#[test]
fn diagnostic_conteneur() {
    println!("écritures virtualisées (conteneur MSIX) : {}", crate::winx::virtualized());
}
