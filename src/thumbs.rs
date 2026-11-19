//! Miniatures décodées en tâche de fond : l'interface ne bloque jamais.
//! Images courantes : décodeur Rust. Le reste (vidéos, PDF, Office, PSD, HEIC, pochettes MP3…) :
//! le fournisseur de miniatures de Windows, exactement comme l'Explorateur.
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type Key = (PathBuf, u32);

enum Slot {
    Pending,
    Ready(TextureHandle),
    Failed,
}

/// Les textures vivent en RAM (rendu CPU) : on garde un cache raisonnable.
const MAX_SLOTS: usize = 700;

pub struct Thumbs {
    tx: Sender<Key>,
    rx: Receiver<(Key, Option<ColorImage>)>,
    cache: HashMap<Key, Slot>,
}

impl Thumbs {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, req_rx) = unbounded::<Key>();
        let (res_tx, rx) = unbounded();
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 4);
        for _ in 0..workers {
            let req_rx = req_rx.clone();
            let res_tx = res_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                #[cfg(windows)]
                unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                }
                while let Ok(key) = req_rx.recv() {
                    let img = decode(&key.0, key.1);
                    if res_tx.send((key, img)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Thumbs { tx, rx, cache: HashMap::new() }
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
                let _ = self.tx.send(key);
                None
            }
        }
    }

    pub fn invalidate(&mut self, path: &Path) {
        self.cache.retain(|k, _| k.0 != path);
    }
}

fn decode(path: &Path, size: u32) -> Option<ColorImage> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if crate::fsx::is_image_ext(&ext) {
        if let Some(img) = decode_rust(path, size) {
            return Some(img);
        }
    }
    #[cfg(windows)]
    {
        shell_thumb(path, size)
    }
    #[cfg(not(windows))]
    None
}

fn decode_rust(path: &Path, size: u32) -> Option<ColorImage> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 120 * 1024 * 1024 {
        return None;
    }
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let img = if img.width() > size || img.height() > size { img.thumbnail(size, size) } else { img };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw()))
}

/// Miniature fournie par Windows (IShellItemImageFactory). Pas d'icône générique :
/// seulement une vraie miniature, sinon `None`.
#[cfg(windows)]
fn shell_thumb(path: &Path, size: u32) -> Option<ColorImage> {
    use windows::Win32::Foundation::{HWND, SIZE};
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
        GetObjectW(hbmp, std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut BITMAP as *mut _));
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
            let _ = DeleteObject(hbmp);
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
        let hdc = GetDC(HWND::default());
        let lines = GetDIBits(hdc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(HWND::default(), hdc);
        let _ = DeleteObject(hbmp);
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
