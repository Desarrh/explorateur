//! **Icônes Windows** (réglage « Icônes des fichiers ») : celles de l'Explorateur, lues par le Shell
//! sur un thread et mises en cache. Une icône par **extension** (et une pour les dossiers), sauf
//! pour les fichiers qui ont chacun la leur (`.exe`, `.lnk`, `.ico`, `.url`, `.msc`) : une par
//! fichier. En attendant, l'icône vectorielle de l'app s'affiche : le défilement ne ralentit jamais.
use crate::fsx::Entry;
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::PathBuf;

/// Taille lue (px) : nette en liste (16) comme en grille (44), une fois réduite.
const SIZE: u32 = 48;
/// Icônes propres à chaque fichier gardées au plus (les icônes par extension restent).
const MAX_PER_FILE: usize = 1500;

enum Slot {
    Pending,
    Ready(TextureHandle),
    Failed,
}

pub struct SysIcons {
    tx: Sender<(String, PathBuf)>,
    rx: Receiver<(String, Option<egui::ColorImage>)>,
    cache: HashMap<String, Slot>,
    per_file: usize,
}

/// Extensions dont chaque fichier a sa propre icône.
fn per_file(ext: &str) -> bool {
    matches!(ext, "exe" | "lnk" | "ico" | "url" | "msc" | "appref-ms")
}

impl SysIcons {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, req_rx) = unbounded::<(String, PathBuf)>();
        let (res_tx, rx) = unbounded();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("icones".into())
            .spawn(move || {
                #[cfg(windows)]
                unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
                }
                while let Ok((key, path)) = req_rx.recv() {
                    #[cfg(windows)]
                    let img = crate::thumbs::shell_icon(&path, SIZE);
                    #[cfg(not(windows))]
                    let img = {
                        let _ = path;
                        None
                    };
                    if res_tx.send((key, img)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok();
        SysIcons { tx, rx, cache: HashMap::new(), per_file: 0 }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((key, img)) = self.rx.try_recv() {
            let slot = match img {
                Some(img) => Slot::Ready(ctx.load_texture(format!("icone:{key}"), img, TextureOptions::LINEAR)),
                None => Slot::Failed,
            };
            self.cache.insert(key, slot);
        }
    }

    /// Icône Windows de l'élément si elle est prête ; sinon la demande (et `None`).
    pub fn get(&mut self, e: &Entry) -> Option<TextureHandle> {
        let (key, path) = if e.is_dir {
            // Icône de dossier ordinaire (celle du Bureau ou de Documents est spéciale).
            ("dossier".to_string(), std::env::temp_dir())
        } else if per_file(&e.ext) {
            (format!("fichier:{}", e.path.to_string_lossy().to_lowercase()), e.path.clone())
        } else {
            (format!("ext:{}", e.ext), e.path.clone())
        };
        match self.cache.get(&key) {
            Some(Slot::Ready(t)) => Some(t.clone()),
            Some(_) => None,
            None => {
                if key.starts_with("fichier:") {
                    self.per_file += 1;
                    if self.per_file > MAX_PER_FILE {
                        self.cache.retain(|k, s| !k.starts_with("fichier:") || matches!(s, Slot::Pending));
                        self.per_file = 0;
                    }
                }
                self.cache.insert(key.clone(), Slot::Pending);
                let _ = self.tx.send((key, path));
                None
            }
        }
    }

    /// Un fichier a changé : son icône propre (exe, raccourci…) est peut-être différente.
    pub fn invalidate(&mut self, path: &std::path::Path) {
        self.cache.remove(&format!("fichier:{}", path.to_string_lossy().to_lowercase()));
    }
}
