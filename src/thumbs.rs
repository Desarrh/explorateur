//! Miniatures décodées en tâche de fond : l'interface ne bloque jamais.
//! Images (PNG, JPEG, WebP, AVIF, TIFF, SVG, PSD, HEIC via WIC…) : décodeurs du socle, avec
//! cache disque QOI (%LOCALAPPDATA%\Explorateur\miniatures). Le reste (vidéos, PDF, Office, PSD, HEIC, pochettes MP3…) :
//! le fournisseur de miniatures de Windows, exactement comme l'Explorateur.
use crossbeam_channel::{unbounded, Receiver};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

type Key = (PathBuf, u32);

enum Slot {
    Pending,
    Ready(TextureHandle),
    Failed,
}

/// Les textures vivent en RAM (rendu CPU) : on garde un cache raisonnable.
const MAX_SLOTS: usize = 700;
/// Demandes en attente au plus : au-delà (défilement rapide), les plus anciennes sont oubliées
/// et seront redemandées si elles reviennent à l'écran.
const MAX_QUEUE: usize = 200;

/// Pile de demandes : les workers prennent **la plus récente** (ce qui est à l'écran maintenant).
type Queue = Arc<(Mutex<VecDeque<Key>>, Condvar)>;

pub struct Thumbs {
    queue: Queue,
    rx: Receiver<(Key, Option<ColorImage>)>,
    cache: HashMap<Key, Slot>,
}

impl Thumbs {
    pub fn new(ctx: &egui::Context) -> Self {
        let queue: Queue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let (res_tx, rx) = unbounded();
        let disk = socle::images::DiskCache::for_app(crate::app::app_name());
        // Ménage du cache disque (500 Mo max), sans retarder le démarrage.
        let d2 = disk.clone();
        std::thread::spawn(move || {
            let _ = d2.prune(500 * 1024 * 1024);
        });
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 4);
        for _ in 0..workers {
            let queue = queue.clone();
            let res_tx = res_tx.clone();
            let ctx = ctx.clone();
            let disk = disk.clone();
            std::thread::spawn(move || {
                #[cfg(windows)]
                unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                }
                loop {
                    let key = {
                        let (lock, cv) = &*queue;
                        let Ok(mut q) = lock.lock() else { break };
                        loop {
                            if let Some(k) = q.pop_back() {
                                break k;
                            }
                            q = match cv.wait(q) {
                                Ok(q) => q,
                                Err(_) => return,
                            };
                        }
                    };
                    let img = decode(&key.0, key.1, &disk);
                    if res_tx.send((key, img)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Thumbs { queue, rx, cache: HashMap::new() }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((key, img)) = self.rx.try_recv() {
            // Demande abandonnée entre-temps (cache vidé) : on jette.
            if !matches!(self.cache.get(&key), Some(Slot::Pending)) {
                continue;
            }
            let slot = match img {
                Some(img) => {
                    let name = format!("thumb:{}:{}", key.0.display(), key.1);
                    // Petites images (pixel art) : agrandies sans flou
                    let opts = if img.size[0].max(img.size[1]) <= 128 { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
                    Slot::Ready(ctx.load_texture(name, img, opts))
                }
                None => Slot::Failed,
            };
            self.cache.insert(key, slot);
        }
    }

    /// Renvoie la miniature si prête, sinon la demande.
    pub fn get(&mut self, path: &Path, size: u32) -> Option<TextureHandle> {
        let key = (path.to_path_buf(), size);
        match self.cache.get(&key) {
            Some(Slot::Ready(t)) => Some(t.clone()),
            Some(_) => None,
            None => {
                if self.cache.len() > MAX_SLOTS {
                    // Libère les textures prêtes (elles seront redemandées si on y revient).
                    self.cache.retain(|_, s| matches!(s, Slot::Pending));
                }
                self.cache.insert(key.clone(), Slot::Pending);
                let (lock, cv) = &*self.queue;
                if let Ok(mut q) = lock.lock() {
                    q.push_back(key);
                    while q.len() > MAX_QUEUE {
                        if let Some(old) = q.pop_front() {
                            self.cache.remove(&old); // plus « en attente » : redemandée si revue
                        }
                    }
                }
                cv.notify_one();
                None
            }
        }
    }

    pub fn invalidate(&mut self, path: &Path) {
        self.cache.retain(|k, _| k.0 != path);
    }
}

pub(crate) fn decode(path: &Path, size: u32, disk: &socle::images::DiskCache) -> Option<ColorImage> {
    if socle::images::is_supported(path) {
        if let Ok(img) = socle::images::thumbnail_cached(disk, path, size, &socle::Cancel::new()) {
            return Some(socle::images::to_color_image(&img));
        }
    }
    #[cfg(windows)]
    {
        shell_thumb(path, size)
    }
    #[cfg(not(windows))]
    None
}

/// Miniature fournie par Windows (IShellItemImageFactory). Pas d'icône générique :
/// seulement une vraie miniature, sinon `None`.
#[cfg(windows)]
fn shell_thumb(path: &Path, size: u32) -> Option<ColorImage> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC,
    };
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY,
    };
    use windows::core::HSTRING;
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let hbmp = factory
            .GetImage(SIZE { cx: size as i32, cy: size as i32 }, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK)
            .ok()?;
        let mut bm = BITMAP::default();
        GetObjectW(hbmp.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut BITMAP as *mut _));
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
            let _ = DeleteObject(hbmp.into());
            return None;
        }
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // de haut en bas
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let hdc = GetDC(None);
        let lines = GetDIBits(hdc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(None, hdc);
        let _ = DeleteObject(hbmp.into());
        if lines == 0 {
            return None;
        }
        // BGRA → RGBA. Beaucoup de miniatures (vidéos, JPEG) ont un alpha à 0 : opaques en vrai.
        let no_alpha = buf.chunks_exact(4).all(|p| p[3] == 0);
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            if no_alpha {
                px[3] = 255;
            }
        }
        let sz = [w as usize, h as usize];
        Some(if no_alpha { ColorImage::from_rgba_unmultiplied(sz, &buf) } else { ColorImage::from_rgba_premultiplied(sz, &buf) })
    }
}
