//! **Carte de l'espace disque** (E3), façon WizTree : le dossier affiché en rectangles
//! proportionnels à la place occupée (« treemap » en carrés, algorithme de Bruls et al.).
//! - les tailles des dossiers viennent de `sizes.rs` (déjà calculées en parallèle et en cache) ;
//! - l'arbre n'est lu que sur la profondeur affichée (4 niveaux), et seulement pour les dossiers
//!   assez gros pour être visibles ; la lecture se fait sur un thread ;
//! - la disposition (quelques milliers de rectangles au plus) est recalculée quand la zone change.
use crate::app::{Action, App};
use crate::fsx;
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui};
use socle::theme::blend;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Niveaux lus sous le dossier courant (les plus gros dossiers seulement, voir `MAX_LISTED`).
const DEPTH: usize = 4;
/// Éléments gardés par dossier (le reste est regroupé).
const MAX_CHILDREN: usize = 150;
/// Dossiers lus au plus pour une carte (borne les E/S).
const MAX_LISTED: usize = 400;
/// Hauteur du bandeau-titre d'un dossier dans la carte.
const HEADER: f32 = 17.0;

#[derive(Clone)]
pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub ext: String,
    pub children: Vec<Node>,
    /// « 34 autres éléments » : regroupement des petits.
    pub rest: Option<u32>,
}

/// Lit `dir` et ses sous-dossiers visibles. `sizes` : tailles de dossiers déjà connues.
pub fn build(dir: &Path, sizes: &HashMap<PathBuf, u64>) -> Node {
    let mut budget = MAX_LISTED;
    let (children, total) = children_of(dir, sizes, 0, &mut budget);
    let name = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| dir.to_string_lossy().into_owned());
    Node { path: dir.to_path_buf(), name, size: total, is_dir: true, ext: String::new(), children, rest: None }
}

/// Enfants de `dir` (taille 0 pour un dossier pas encore mesuré) et leur total.
fn children_of(dir: &Path, sizes: &HashMap<PathBuf, u64>, depth: usize, budget: &mut usize) -> (Vec<Node>, u64) {
    *budget = budget.saturating_sub(1);
    let entries = fsx::list_dir(dir).unwrap_or_default();
    let mut v: Vec<Node> = entries
        .into_iter()
        .map(|e| {
            let size = if e.is_dir { sizes.get(&e.path).copied().unwrap_or(0) } else { e.size };
            Node {
                name: e.name,
                size,
                is_dir: e.is_dir,
                ext: e.ext,
                children: vec![],
                rest: None,
                path: e.path,
            }
        })
        .collect();
    let total: u64 = v.iter().map(|n| n.size).sum();
    v.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    // Petits éléments regroupés : moins de 0,2 % du dossier, ou au-delà de MAX_CHILDREN.
    let keep = v.iter().take(MAX_CHILDREN).take_while(|n| n.size > 0 && n.size * 500 >= total).count();
    if v.len() > keep {
        let tail = v.split_off(keep);
        let (n, s) = (tail.len() as u32, tail.iter().map(|x| x.size).sum::<u64>());
        if s > 0 {
            v.push(Node {
                path: dir.to_path_buf(),
                name: format!("{n} autres éléments"),
                size: s,
                is_dir: false,
                ext: String::new(),
                children: vec![],
                rest: Some(n),
            });
        }
    }
    // Sous-dossiers assez gros pour être visibles (≥ 2 %) : un niveau de plus, les plus gros d'abord.
    if depth + 1 < DEPTH {
        for c in v.iter_mut().filter(|c| c.is_dir && c.rest.is_none()) {
            if *budget == 0 || c.size * 50 < total {
                break;
            }
            c.children = children_of(&c.path, sizes, depth + 1, budget).0;
        }
    }
    (v, total)
}

/// Rectangles « en carrés » pour des valeurs triées par ordre décroissant (Bruls, Huizing, van Wijk).
pub fn squarify(values: &[f64], rect: Rect) -> Vec<Rect> {
    let n = values.len();
    let mut out = vec![Rect::NOTHING; n];
    let total: f64 = values.iter().sum();
    if total <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return out;
    }
    let scale = (rect.width() as f64 * rect.height() as f64) / total;
    let areas: Vec<f64> = values.iter().map(|v| v * scale).collect();
    let worst = |row: &[f64], side: f64| -> f64 {
        let s: f64 = row.iter().sum();
        let (mx, mn) = row.iter().fold((0f64, f64::MAX), |(a, b), &x| (a.max(x), b.min(x)));
        if mn <= 0.0 || s <= 0.0 {
            return f64::MAX;
        }
        (side * side * mx / (s * s)).max(s * s / (side * side * mn))
    };
    let mut r = rect;
    let mut i = 0;
    while i < n {
        let (w, h) = (r.width() as f64, r.height() as f64);
        let side = w.min(h).max(1e-6);
        let mut j = i + 1;
        let mut best = worst(&areas[i..j], side);
        while j < n {
            let next = worst(&areas[i..j + 1], side);
            if next > best {
                break;
            }
            best = next;
            j += 1;
        }
        let sum: f64 = areas[i..j].iter().sum();
        if w >= h {
            // colonne à gauche
            let cw = (sum / h.max(1e-6)) as f32;
            let mut y = r.top();
            for k in i..j {
                let kh = (areas[k] / cw.max(1e-6) as f64) as f32;
                out[k] = Rect::from_min_size(pos2(r.left(), y), vec2(cw, kh));
                y += kh;
            }
            r.min.x += cw;
        } else {
            // rangée en haut
            let rh = (sum / w.max(1e-6)) as f32;
            let mut x = r.left();
            for k in i..j {
                let kw = (areas[k] / rh.max(1e-6) as f64) as f32;
                out[k] = Rect::from_min_size(pos2(x, r.top()), vec2(kw, rh));
                x += kw;
            }
            r.min.y += rh;
        }
        i = j;
    }
    out
}

pub struct Tile {
    pub rect: Rect,
    pub depth: u8,
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub ext: String,
    pub rest: bool,
    /// Le contenu du dossier est dessiné dedans (bandeau-titre en haut).
    pub nested: bool,
}

/// Dispose l'arbre dans `area` (parents avant enfants : l'ordre de dessin).
pub fn layout(root: &Node, area: Rect) -> Vec<Tile> {
    let mut out = Vec::new();
    place(&root.children, area, 0, &mut out);
    out
}

fn place(nodes: &[Node], area: Rect, depth: u8, out: &mut Vec<Tile>) {
    let shown: Vec<&Node> = nodes.iter().filter(|n| n.size > 0).collect();
    let values: Vec<f64> = shown.iter().map(|n| n.size as f64).collect();
    for (n, r) in shown.into_iter().zip(squarify(&values, area)) {
        if r.width() < 1.0 || r.height() < 1.0 {
            continue;
        }
        let nested = n.is_dir && !n.children.is_empty() && r.width() > 46.0 && r.height() > HEADER + 18.0;
        out.push(Tile {
            rect: r,
            depth,
            path: n.path.clone(),
            name: n.name.clone(),
            size: n.size,
            is_dir: n.is_dir,
            ext: n.ext.clone(),
            rest: n.rest.is_some(),
            nested,
        });
        if nested {
            let inner = Rect::from_min_max(pos2(r.left() + 2.0, r.top() + HEADER), pos2(r.right() - 2.0, r.bottom() - 2.0));
            place(&n.children, inner, depth + 1, out);
        }
    }
}

/// Catégorie de couleur d'un élément.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Image,
    Video,
    Audio,
    Archive,
    Other,
}

pub fn kind_of(is_dir: bool, ext: &str) -> Kind {
    if is_dir {
        Kind::Folder
    } else if fsx::is_video_ext(ext) {
        Kind::Video
    } else if fsx::is_image_ext(ext) || matches!(ext, "psd" | "heic" | "raw" | "dng" | "cr2" | "nef" | "arw" | "exr" | "hdr") {
        Kind::Image
    } else if matches!(ext, "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "wma" | "opus" | "aiff") {
        Kind::Audio
    } else if matches!(ext, "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "zst" | "iso" | "img" | "vhd" | "vhdx" | "msi" | "exe" | "pak" | "bin") {
        Kind::Archive
    } else {
        Kind::Other
    }
}

/// État de la carte d'un onglet.
#[derive(Default)]
pub struct MapState {
    key: Option<(PathBuf, usize, u64)>,
    started: Option<Instant>,
    rx: Option<crossbeam_channel::Receiver<Node>>,
    pub tree: Option<Node>,
    tiles: Vec<Tile>,
    tiles_for: Option<(Rect, usize)>,
    built: usize,
    menu: Option<(PathBuf, bool)>,
}

impl App {
    /// Vue « Carte de l'espace » du dossier de l'onglet actif.
    pub fn draw_space_map(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let (tid, path, n_entries) = {
            let t = self.tab();
            (t.id, t.path.clone(), t.entries.len())
        };
        let version = self.sizes.version;
        let pending_sizes = self.sizes.pending_count();
        let st = self.space_maps.entry(tid).or_default();
        // Résultat de la lecture en tâche de fond
        if let Some(rx) = &st.rx {
            if let Ok(tree) = rx.try_recv() {
                st.tree = Some(tree);
                st.rx = None;
                st.built += 1;
            }
        }
        // (Re)lecture si le dossier, son contenu ou les tailles connues ont changé (au plus 2,5 fois/s).
        let key = (path.clone(), n_entries, version);
        let stale = st.key.as_ref() != Some(&key);
        let can_start = st.rx.is_none() && st.started.map(|t| t.elapsed() > Duration::from_millis(400)).unwrap_or(true);
        if stale && can_start {
            if st.key.as_ref().map(|k| k.0 != path).unwrap_or(true) {
                st.tree = None; // autre dossier : on ne montre pas l'ancienne carte
            }
            st.key = Some(key);
            st.started = Some(Instant::now());
            let sizes: HashMap<PathBuf, u64> =
                self.sizes.map.iter().filter(|(k, _)| k.starts_with(&path)).map(|(k, v)| (k.clone(), *v)).collect();
            let (tx, rx) = crossbeam_channel::bounded(1);
            st.rx = Some(rx);
            let ctx = ui.ctx().clone();
            let dir = path.clone();
            std::thread::spawn(move || {
                let _ = tx.send(build(&dir, &sizes));
                ctx.request_repaint();
            });
        } else if stale {
            ui.ctx().request_repaint_after(Duration::from_millis(420));
        }

        // En-tête : total, état du calcul, légende
        let total = st.tree.as_ref().map(|t| t.size).unwrap_or(0);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("Carte de l’espace").size(14.0).strong());
            if st.tree.is_some() {
                ui.label(RichText::new(fsx::human_size(total)).color(p.muted).size(12.5));
            }
            if pending_sizes > 0 || st.rx.is_some() {
                ui.spinner();
                let msg = if pending_sizes > 0 { format!("calcul de {pending_sizes} dossier(s)…") } else { "lecture…".into() };
                ui.label(RichText::new(msg).color(p.muted).size(12.0));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                for (k, label) in [(Kind::Other, "Autres"), (Kind::Archive, "Archives, programmes"), (Kind::Audio, "Audio"), (Kind::Video, "Vidéos"), (Kind::Image, "Images"), (Kind::Folder, "Dossiers")] {
                    ui.label(RichText::new(label).size(11.5).color(p.muted));
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(2), base_color(&p, k));
                    ui.add_space(6.0);
                }
            });
        });
        ui.add_space(4.0);
        let area = ui.available_rect_before_wrap().shrink2(vec2(10.0, 0.0));
        let area = Rect::from_min_max(area.min, pos2(area.max.x, area.max.y - 8.0));
        let resp = ui.allocate_rect(area, Sense::click());
        let Some(tree) = st.tree.as_ref() else {
            ui.painter().text(area.center(), Align2::CENTER_CENTER, "Lecture du dossier…", FontId::proportional(14.0), p.muted);
            return;
        };
        if total == 0 {
            let msg = if pending_sizes > 0 { "Calcul des tailles…" } else { "Ce dossier ne contient aucun octet" };
            ui.painter().text(area.center(), Align2::CENTER_CENTER, msg, FontId::proportional(14.0), p.muted);
            return;
        }
        if st.tiles_for != Some((area, st.built)) {
            st.tiles = layout(tree, area);
            st.tiles_for = Some((area, st.built));
        }

        // Dessin
        let painter = ui.painter_at(area);
        let hover_pos = resp.hover_pos();
        let hovered = hover_pos.and_then(|pos| st.tiles.iter().rposition(|t| t.rect.contains(pos)));
        for (i, t) in st.tiles.iter().enumerate() {
            let k = kind_of(t.is_dir, &t.ext);
            let base = base_color(&p, k);
            let strength = if t.rest { 0.18 } else if t.is_dir { 0.42 - t.depth as f32 * 0.08 } else { 0.62 - t.depth as f32 * 0.1 };
            let mut fill = blend(p.surface, base, strength.max(0.15));
            if Some(i) == hovered {
                fill = blend(fill, base, 0.45);
            }
            painter.rect(t.rect, CornerRadius::same(if t.depth == 0 { 4 } else { 2 }), fill, Stroke::new(1.0, p.surface), StrokeKind::Inside);
            let text_col = if strength > 0.5 { p.text } else { blend(p.text, p.muted, 0.3) };
            let (w, h) = (t.rect.width(), t.rect.height());
            if w > 56.0 && h > 16.0 {
                let font = FontId::proportional(if t.depth == 0 { 12.5 } else { 11.5 });
                let label = if t.nested || h < 32.0 { format!("{}  {}", t.name, fsx::human_size(t.size)) } else { t.name.clone() };
                let g = socle::ui::trunc(ui, &label, font, text_col, w - 10.0);
                let y = if t.nested { 1.5 } else { 3.0 };
                painter.galley(pos2(t.rect.left() + 5.0, t.rect.top() + y), g, text_col);
                if !t.nested && h >= 32.0 {
                    painter.text(pos2(t.rect.left() + 5.0, t.rect.top() + 18.0), Align2::LEFT_TOP, fsx::human_size(t.size), FontId::proportional(11.0), p.muted);
                }
            }
        }
        // Interactions
        let mut acts = Vec::new();
        if let Some(i) = hovered {
            let t = &st.tiles[i];
            let pct = t.size as f64 * 100.0 / total.max(1) as f64;
            let where_ = if t.rest { format!("dans {}", t.path.display()) } else { t.path.display().to_string() };
            let resp = resp.clone().on_hover_text(format!("{}\n{} · {:.1} % du dossier\n{}", t.name, fsx::human_size(t.size), pct, where_));
            if !t.rest {
                if resp.clicked() {
                    acts.push(if t.is_dir { Action::Navigate(t.path.clone()) } else { Action::SelectOnly(t.path.clone()) });
                }
                if resp.double_clicked() && !t.is_dir {
                    acts.push(Action::Open(t.path.clone()));
                }
                if resp.secondary_clicked() {
                    st.menu = Some((t.path.clone(), t.is_dir));
                }
            }
        }
        let menu = st.menu.clone();
        resp.context_menu(|ui| {
            let Some((path, is_dir)) = &menu else { return };
            ui.label(RichText::new(path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()).strong());
            ui.separator();
            if ui.button(if *is_dir { "Ouvrir le dossier" } else { "Ouvrir" }).clicked() {
                acts.push(if *is_dir { Action::Navigate(path.clone()) } else { Action::Open(path.clone()) });
                ui.close();
            }
            if ui.button("Afficher dans la liste").clicked() {
                acts.push(Action::ShowInFolder(path.clone()));
                ui.close();
            }
            if ui.button("Mettre à la corbeille").clicked() {
                acts.push(Action::TrashPaths(vec![path.clone()]));
                ui.close();
            }
        });
        self.actions.extend(acts);
    }
}

fn base_color(p: &socle::theme::Palette, k: Kind) -> Color32 {
    match k {
        Kind::Folder => p.accent,
        Kind::Image => p.add,
        Kind::Video => p.live,
        Kind::Audio => blend(p.accent, p.del, 0.5),
        Kind::Archive => p.del,
        Kind::Other => p.muted,
    }
}
