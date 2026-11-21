//! Win32 process icon extraction and GPU texture caching.
//!
//! Extraction (OpenProcess / ExtractIconExW / GDI conversion) runs on a worker thread
//! so the first paint of a long process list does not stall the UI thread. Requested
//! keys are queued once, the queue is bounded, and a failed load is cached as `None`
//! so a permanent failure (icon-less binary, inaccessible process) is never retried.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use egui::{ColorImage, Context, TextureHandle, TextureOptions};

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::CloseHandle;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC,
    BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::ExtractIconExW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

/// Extractions kept in flight at once: the picker fills its list over a few frames
/// instead of queueing every visible row's Win32 work in one paint.
const MAX_ICON_LOADS_IN_FLIGHT: usize = 8;

/// Hard bound on the worker request channel.
const ICON_REQUEST_QUEUE: usize = 16;

struct IconLoadResult {
    key: String,
    texture_name: String,
    image: Option<ColorImage>,
}

enum IconLoadRequest {
    Process {
        key: String,
        texture_name: String,
        pid: u32,
        name: String,
    },
    Path {
        key: String,
        texture_name: String,
        path: String,
    },
}

impl IconLoadRequest {
    fn key(&self) -> &str {
        match self {
            Self::Process { key, .. } | Self::Path { key, .. } => key,
        }
    }
}

pub struct ProcessIconCache {
    cache: HashMap<String, Option<TextureHandle>>,
    pending: HashSet<String>,
    requests: SyncSender<IconLoadRequest>,
    results: Receiver<IconLoadResult>,
}

impl Default for ProcessIconCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessIconCache {
    pub fn new() -> Self {
        let (requests, request_rx) = mpsc::sync_channel::<IconLoadRequest>(ICON_REQUEST_QUEUE);
        let (result_tx, results) = mpsc::channel::<IconLoadResult>();
        let _ = std::thread::Builder::new()
            .name("synector-icon-loader".to_owned())
            .spawn(move || icon_worker(request_rx, result_tx));
        Self {
            cache: HashMap::new(),
            pending: HashSet::new(),
            requests,
            results,
        }
    }

    pub fn get_icon(&mut self, ctx: &Context, pid: u32, name: &str) -> Option<&TextureHandle> {
        let key = name.to_lowercase();
        self.collect_ready(ctx);
        if !self.cache.contains_key(&key) && !self.pending.contains(&key) {
            self.enqueue(IconLoadRequest::Process {
                texture_name: format!("proc_icon_{}", name),
                key: key.clone(),
                pid,
                name: name.to_owned(),
            });
        }
        self.cache.get(&key).and_then(|opt| opt.as_ref())
    }

    pub fn get_icon_from_path(&mut self, ctx: &Context, path: &str) -> Option<&TextureHandle> {
        let key = format!("path:{}", path.to_lowercase());
        self.collect_ready(ctx);
        if !self.cache.contains_key(&key) && !self.pending.contains(&key) {
            self.enqueue(IconLoadRequest::Path {
                texture_name: format!("path_icon_{}", path),
                key: key.clone(),
                path: path.to_owned(),
            });
        }
        self.cache.get(&key).and_then(|opt| opt.as_ref())
    }

    pub fn get_app_icon(&mut self, ctx: &Context) -> Option<&TextureHandle> {
        let key = "__app_logo_icon__".to_string();
        self.collect_ready(ctx);
        if !self.cache.contains_key(&key) {
            let loaded = load_embedded_app_icon(ctx);
            self.cache.insert(key.clone(), loaded);
        }
        self.cache.get(&key).and_then(|opt| opt.as_ref())
    }

    /// Moves finished worker results into GPU textures. A failed load is stored as
    /// `None`, which both hides the placeholder and marks the key as permanently done.
    fn collect_ready(&mut self, ctx: &Context) {
        while let Ok(result) = self.results.try_recv() {
            self.pending.remove(&result.key);
            let texture = result
                .image
                .map(|image| ctx.load_texture(result.texture_name, image, TextureOptions::LINEAR));
            self.cache.insert(result.key, texture);
        }
    }

    fn enqueue(&mut self, request: IconLoadRequest) {
        let key = request.key().to_owned();
        if self.pending.len() >= MAX_ICON_LOADS_IN_FLIGHT || self.pending.contains(&key) {
            return;
        }
        match self.requests.try_send(request) {
            Ok(()) => {
                self.pending.insert(key);
            }
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

fn icon_worker(requests: Receiver<IconLoadRequest>, results: mpsc::Sender<IconLoadResult>) {
    while let Ok(request) = requests.recv() {
        let result = match request {
            IconLoadRequest::Process {
                key,
                texture_name,
                pid,
                name,
            } => IconLoadResult {
                key,
                texture_name,
                image: load_process_image(pid, &name),
            },
            IconLoadRequest::Path {
                key,
                texture_name,
                path,
            } => IconLoadResult {
                key,
                texture_name,
                image: load_path_image(&path),
            },
        };
        if results.send(result).is_err() {
            break;
        }
    }
}

fn load_embedded_app_icon(ctx: &Context) -> Option<TextureHandle> {
    const APP_PNG: &[u8] = include_bytes!("../../../assets/app.png");
    if let Ok(img) = image::load_from_memory(APP_PNG) {
        let rgba = img.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let pixels = rgba.into_raw();
        let color_image = ColorImage::from_rgba_unmultiplied(size, &pixels);
        Some(ctx.load_texture("app_logo_icon", color_image, TextureOptions::LINEAR))
    } else {
        None
    }
}

fn load_process_image(pid: u32, name: &str) -> Option<ColorImage> {
    #[cfg(windows)]
    {
        extract_process_icon(pid, name)
    }
    #[cfg(not(windows))]
    {
        let _ = (pid, name);
        None
    }
}

/// Executables, libraries and shortcuts are binary containers: their icons come from
/// the shell extractor and the file bytes are never fed to the image decoder.
fn is_shell_icon_container(extension: &str) -> bool {
    matches!(extension, "exe" | "dll" | "lnk")
}

fn load_path_image(path: &str) -> Option<ColorImage> {
    let path_buf = Path::new(path);
    if !path_buf.exists() {
        return None;
    }

    let ext = path_buf
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let shell_only = is_shell_icon_container(&ext);

    #[cfg(windows)]
    {
        if shell_only || ext == "ico" {
            if let Some(color_image) = extract_icon_from_file_path(path) {
                return Some(color_image);
            }
            if shell_only {
                return None;
            }
        }
    }
    #[cfg(not(windows))]
    {
        if shell_only {
            return None;
        }
    }

    if let Ok(img) = image::open(path_buf) {
        let rgba = img.to_rgba8();
        let final_img = if rgba.width() > 64 || rgba.height() > 64 {
            image::imageops::thumbnail(&rgba, 64, 64)
        } else {
            rgba
        };
        let size = [final_img.width() as usize, final_img.height() as usize];
        let pixels = final_img.into_raw();
        return Some(ColorImage::from_rgba_unmultiplied(size, &pixels));
    }

    #[cfg(windows)]
    {
        // A single shell extraction fallback for non-container paths whose image
        // decoding failed; there is deliberately no second attempt.
        if !shell_only && ext != "ico" {
            return extract_icon_from_file_path(path);
        }
    }

    None
}

#[cfg(windows)]
fn extract_icon_from_file_path(path: &str) -> Option<ColorImage> {
    unsafe {
        let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut large_icon = HICON::default();
        let mut small_icon = HICON::default();
        let count = ExtractIconExW(PCWSTR(path_w.as_ptr()), 0, Some(&mut large_icon), Some(&mut small_icon), 1);
        if count > 0 {
            let hicon = if !large_icon.is_invalid() {
                if !small_icon.is_invalid() {
                    let _ = DestroyIcon(small_icon);
                }
                large_icon
            } else if !small_icon.is_invalid() {
                small_icon
            } else {
                return None;
            };

            let result = hicon_to_color_image(hicon);
            let _ = DestroyIcon(hicon);
            return result;
        }
    }
    None
}

#[cfg(windows)]
fn extract_process_icon(pid: u32, name: &str) -> Option<ColorImage> {
    unsafe {
        let hicon = get_process_hicon(pid, name)?;
        let result = hicon_to_color_image(hicon);
        let _ = DestroyIcon(hicon);
        result
    }
}

#[cfg(windows)]
unsafe fn get_process_hicon(pid: u32, name: &str) -> Option<HICON> {
    // 1. Try to query full executable path from PID
    let mut exe_path: Option<String> = None;
    if let Ok(h_proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        if QueryFullProcessImageNameW(
            h_proc,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .is_ok()
        {
            exe_path = Some(String::from_utf16_lossy(&buf[..size as usize]));
        }
        let _ = CloseHandle(h_proc);
    }

    // 2. Extract 16x16 icon from path
    if let Some(path) = exe_path {
        let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut small_icon = HICON::default();
        let count = ExtractIconExW(PCWSTR(path_w.as_ptr()), 0, None, Some(&mut small_icon), 1);
        if count > 0 && !small_icon.is_invalid() {
            return Some(small_icon);
        }
    }

    // 3. Fallback: Check System32
    let sys_root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("C:\\Windows"));
    let sys32_candidate = sys_root.join("System32").join(name);
    if sys32_candidate.exists() {
        let path_w: Vec<u16> = sys32_candidate.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
        let mut small_icon = HICON::default();
        let count = ExtractIconExW(PCWSTR(path_w.as_ptr()), 0, None, Some(&mut small_icon), 1);
        if count > 0 && !small_icon.is_invalid() {
            return Some(small_icon);
        }
    }

    None
}

#[cfg(windows)]
unsafe fn hicon_to_color_image(hicon: HICON) -> Option<ColorImage> {
    let mut icon_info: ICONINFO = std::mem::zeroed();
    if GetIconInfo(hicon, &mut icon_info).is_err() {
        return None;
    }

    let hdc_screen = GetDC(None);
    let hdc_mem = CreateCompatibleDC(Some(hdc_screen));

    let hbm = if !icon_info.hbmColor.is_invalid() {
        icon_info.hbmColor
    } else {
        icon_info.hbmMask
    };

    let mut bm: BITMAP = std::mem::zeroed();
    if GetObjectW(
        hbm.into(),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bm as *mut _ as *mut _),
    ) == 0
    {
        if !icon_info.hbmColor.is_invalid() {
            let _ = DeleteObject(icon_info.hbmColor.into());
        }
        if !icon_info.hbmMask.is_invalid() {
            let _ = DeleteObject(icon_info.hbmMask.into());
        }
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);
        return None;
    }

    let width = bm.bmWidth;
    let height = if icon_info.hbmColor.is_invalid() {
        bm.bmHeight / 2
    } else {
        bm.bmHeight
    };

    if width <= 0 || height <= 0 {
        if !icon_info.hbmColor.is_invalid() {
            let _ = DeleteObject(icon_info.hbmColor.into());
        }
        if !icon_info.hbmMask.is_invalid() {
            let _ = DeleteObject(icon_info.hbmMask.into());
        }
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);
        return None;
    }

    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = width;
    bmi.bmiHeader.biHeight = -height; // Top-down DIB
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;

    let mut buf = vec![0u8; (width * height * 4) as usize];

    let lines = GetDIBits(
        hdc_mem,
        hbm,
        0,
        height as u32,
        Some(buf.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );

    if lines == 0 {
        if !icon_info.hbmColor.is_invalid() {
            let _ = DeleteObject(icon_info.hbmColor.into());
        }
        if !icon_info.hbmMask.is_invalid() {
            let _ = DeleteObject(icon_info.hbmMask.into());
        }
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);
        return None;
    }

    // Convert BGRA to RGBA
    for pixel in buf.as_chunks_mut::<4>().0 {
        let b = pixel[0];
        let r = pixel[2];
        pixel[0] = r;
        pixel[2] = b;
    }

    // If icon has 0 alpha everywhere (common in 24-bit/16-bit Win32 icons), extract mask or set to 255
    let has_alpha = buf.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
    if !has_alpha {
        let mut mask_applied = false;
        if !icon_info.hbmMask.is_invalid() {
            let mut mask_bmi: BITMAPINFO = std::mem::zeroed();
            mask_bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            mask_bmi.bmiHeader.biWidth = width;
            mask_bmi.bmiHeader.biHeight = -height;
            mask_bmi.bmiHeader.biPlanes = 1;
            mask_bmi.bmiHeader.biBitCount = 32;
            mask_bmi.bmiHeader.biCompression = BI_RGB.0;
            let mut mask_buf = vec![0u8; (width * height * 4) as usize];
            let mask_lines = GetDIBits(
                hdc_mem,
                icon_info.hbmMask,
                0,
                height as u32,
                Some(mask_buf.as_mut_ptr() as *mut _),
                &mut mask_bmi,
                DIB_RGB_COLORS,
            );
            if mask_lines > 0 {
                for (pixel, mask_pixel) in buf.as_chunks_mut::<4>().0.iter_mut().zip(mask_buf.as_chunks::<4>().0.iter()) {
                    // AND mask bit is 0 for opaque content, non-zero for transparent
                    pixel[3] = if mask_pixel[0] == 0 { 255 } else { 0 };
                }
                mask_applied = true;
            }
        }
        if !mask_applied {
            for pixel in buf.as_chunks_mut::<4>().0 {
                pixel[3] = 255;
            }
        }
    }

    // Clean up GDI handles
    if !icon_info.hbmColor.is_invalid() {
        let _ = DeleteObject(icon_info.hbmColor.into());
    }
    if !icon_info.hbmMask.is_invalid() {
        let _ = DeleteObject(icon_info.hbmMask.into());
    }
    let _ = DeleteDC(hdc_mem);
    ReleaseDC(None, hdc_screen);

    let img = image::RgbaImage::from_raw(width as u32, height as u32, buf)?;
    let final_img = if width > 64 || height > 64 {
        image::imageops::thumbnail(&img, 64, 64)
    } else {
        img
    };

    let (w, h) = final_img.dimensions();
    Some(ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        final_img.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_icon_containers_are_not_decoded_as_images() {
        for extension in ["exe", "dll", "lnk"] {
            assert!(
                is_shell_icon_container(extension),
                "{extension} must be extracted through the shell"
            );
        }
        for extension in ["png", "ico", "bmp", "jpg"] {
            assert!(
                !is_shell_icon_container(extension),
                "{extension} may use the image decoder"
            );
        }
    }

    #[test]
    fn test_icon_request_queue_is_bounded_and_deduplicated() {
        let mut cache = ProcessIconCache::new();
        let missing = |index: usize| {
            format!(
                "{}synector-icon-test-{}-{}.png",
                std::env::temp_dir().display(),
                std::process::id(),
                index
            )
        };
        let enqueue_missing = |cache: &mut ProcessIconCache, index: usize| {
            let path = missing(index);
            cache.enqueue(IconLoadRequest::Path {
                key: format!("path:{}", path),
                texture_name: format!("path_icon_{}", path),
                path,
            });
        };

        enqueue_missing(&mut cache, 0);
        assert_eq!(cache.pending.len(), 1);
        enqueue_missing(&mut cache, 0);
        assert_eq!(cache.pending.len(), 1, "the same key must not be queued twice");

        for index in 1..(MAX_ICON_LOADS_IN_FLIGHT * 3) {
            enqueue_missing(&mut cache, index);
        }
        assert_eq!(cache.pending.len(), MAX_ICON_LOADS_IN_FLIGHT);
        assert!(cache.cache.is_empty());
    }

    #[test]
    fn test_failed_icon_load_is_cached_and_not_retried() {
        let ctx = Context::default();
        let mut cache = ProcessIconCache::new();
        let path = format!(
            "{}synector-icon-test-missing-{}.png",
            std::env::temp_dir().display(),
            std::process::id()
        );
        let key = format!("path:{}", path.to_lowercase());

        assert!(cache.get_icon_from_path(&ctx, &path).is_none());
        assert!(cache.pending.contains(&key));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while cache.pending.contains(&key) && std::time::Instant::now() < deadline {
            cache.collect_ready(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!cache.pending.contains(&key), "worker did not finish the load");
        assert!(
            matches!(cache.cache.get(&key), Some(None)),
            "a failed load must be cached as None"
        );

        assert!(cache.get_icon_from_path(&ctx, &path).is_none());
        assert!(
            !cache.pending.contains(&key),
            "a cached failure must not be retried"
        );
    }
}
