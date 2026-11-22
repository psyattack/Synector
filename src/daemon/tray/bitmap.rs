#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP,
};

#[cfg(windows)]
unsafe fn create_pargb_bitmap(width: i32, height: i32, rgba_pixels: &[u8]) -> Option<HBITMAP> {
    if width <= 0 || height <= 0 || rgba_pixels.len() < (width * height * 4) as usize {
        return None;
    }

    let hdc_screen = GetDC(None);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let hbmp = CreateDIBSection(Some(hdc_screen), &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0).ok()?;
    ReleaseDC(None, hdc_screen);

    if bits_ptr.is_null() {
        let _ = DeleteObject(hbmp.into());
        return None;
    }

    let dest = std::slice::from_raw_parts_mut(bits_ptr as *mut u8, (width * height * 4) as usize);

    #[allow(clippy::chunks_exact_to_as_chunks)]
    for (src, dst) in rgba_pixels.chunks_exact(4).zip(dest.chunks_exact_mut(4)) {
        let r = src[0] as u32;
        let g = src[1] as u32;
        let b = src[2] as u32;
        let a = src[3] as u32;

        if a == 0 {
            dst[0] = 0;
            dst[1] = 0;
            dst[2] = 0;
            dst[3] = 0;
        } else if a == 255 {
            dst[0] = b as u8;
            dst[1] = g as u8;
            dst[2] = r as u8;
            dst[3] = 255;
        } else {
            dst[0] = ((b * a + 127) / 255) as u8;
            dst[1] = ((g * a + 127) / 255) as u8;
            dst[2] = ((r * a + 127) / 255) as u8;
            dst[3] = a as u8;
        }
    }

    Some(hbmp)
}

#[cfg(windows)]
fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let wx = p.0 - a.0;
    let wy = p.1 - a.1;
    let c1 = wx * vx + wy * vy;
    if c1 <= 0.0 {
        return (wx * wx + wy * wy).sqrt();
    }
    let c2 = vx * vx + vy * vy;
    if c2 <= c1 {
        let dx = p.0 - b.0;
        let dy = p.1 - b.1;
        return (dx * dx + dy * dy).sqrt();
    }
    let t = c1 / c2;
    let qx = a.0 + t * vx;
    let qy = a.1 + t * vy;
    let dx = p.0 - qx;
    let dy = p.1 - qy;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(windows)]
fn sd_rounded_rect(p: (f32, f32), min_pt: (f32, f32), max_pt: (f32, f32), r: f32) -> f32 {
    let cx = (min_pt.0 + max_pt.0) * 0.5;
    let cy = (min_pt.1 + max_pt.1) * 0.5;
    let hx = (max_pt.0 - min_pt.0) * 0.5 - r;
    let hy = (max_pt.1 - min_pt.1) * 0.5 - r;
    let qx = (p.0 - cx).abs() - hx;
    let qy = (p.1 - cy).abs() - hy;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    let out_d = (ox * ox + oy * oy).sqrt();
    let in_d = qx.max(qy).min(0.0);
    out_d + in_d - r
}

#[cfg(windows)]
pub(super) fn generate_open_menu_bitmap(size: u32, is_dark: bool) -> Option<HBITMAP> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;

    let min_pt = (s * 0.14, s * 0.16);
    let max_pt = (s * 0.86, s * 0.84);
    let r = (s * 0.08).clamp(1.0, 2.0);
    let hw = (s * 0.045).clamp(0.65, 1.0);
    let y_hdr = s * 0.38;

    // Monochrome theme adaptation: crisp white on dark, crisp black on light
    let (r_col, g_col, b_col) = if is_dark {
        (245u8, 245u8, 250u8) // Crisp white
    } else {
        (25u8, 25u8, 30u8)    // Crisp black/charcoal
    };

    let offsets = [(-0.25f32, -0.25f32), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];

    for y in 0..size {
        for x in 0..size {
            let mut cov = 0.0f32;
            for (ox, oy) in offsets {
                let p = (x as f32 + 0.5 + ox, y as f32 + 0.5 + oy);

                let d_frame = sd_rounded_rect(p, min_pt, max_pt, r).abs() - hw;
                let d_hdr = dist_to_segment(p, (min_pt.0 + hw, y_hdr), (max_pt.0 - hw, y_hdr)) - hw;

                let min_d = d_frame.min(d_hdr);
                cov += (0.5 - min_d).clamp(0.0, 1.0);
            }
            cov *= 0.25;

            if cov > 0.015 {
                let idx = ((y * size + x) * 4) as usize;
                let a_byte = (cov * 255.0) as u8;
                rgba[idx] = r_col;
                rgba[idx + 1] = g_col;
                rgba[idx + 2] = b_col;
                rgba[idx + 3] = a_byte;
            }
        }
    }

    unsafe { create_pargb_bitmap(size as i32, size as i32, &rgba) }
}

#[cfg(windows)]
pub(super) fn generate_exit_menu_bitmap(size: u32, is_dark: bool) -> Option<HBITMAP> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let cx = s * 0.5;
    let cy = s * 0.5;
    let r = s * 0.32;
    let hw = (s * 0.055).clamp(0.85, 1.4);

    let rad_left = (-130.0f32).to_radians();
    let rad_right = (-50.0f32).to_radians();
    let p_left = (cx + r * rad_left.cos(), cy + r * rad_left.sin());
    let p_right = (cx + r * rad_right.cos(), cy + r * rad_right.sin());

    // Monochrome theme adaptation: crisp white on dark, crisp black on light
    let (r_col, g_col, b_col) = if is_dark {
        (245u8, 245u8, 250u8) // Crisp white
    } else {
        (25u8, 25u8, 30u8)    // Crisp black/charcoal
    };

    let offsets = [(-0.25f32, -0.25f32), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];

    for y in 0..size {
        for x in 0..size {
            let mut cov = 0.0f32;
            for (ox, oy) in offsets {
                let p = (x as f32 + 0.5 + ox, y as f32 + 0.5 + oy);
                let dx = p.0 - cx;
                let dy = p.1 - cy;
                let dist_from_c = (dx * dx + dy * dy).sqrt();
                let angle_deg = dy.atan2(dx).to_degrees();

                let in_top_gap = angle_deg < -50.0 && angle_deg > -130.0;
                let d_arc = if in_top_gap {
                    let d_left = ((p.0 - p_left.0).powi(2) + (p.1 - p_left.1).powi(2)).sqrt() - hw;
                    let d_right = ((p.0 - p_right.0).powi(2) + (p.1 - p_right.1).powi(2)).sqrt() - hw;
                    d_left.min(d_right)
                } else {
                    (dist_from_c - r).abs() - hw
                };

                let d_line = dist_to_segment(p, (cx, cy - s * 0.38), (cx, cy - s * 0.04)) - hw;
                let min_d = d_arc.min(d_line);
                cov += (0.5 - min_d).clamp(0.0, 1.0);
            }
            cov *= 0.25;

            if cov > 0.015 {
                let idx = ((y * size + x) * 4) as usize;
                let a_byte = (cov * 255.0) as u8;
                rgba[idx] = r_col;
                rgba[idx + 1] = g_col;
                rgba[idx + 2] = b_col;
                rgba[idx + 3] = a_byte;
            }
        }
    }

    unsafe { create_pargb_bitmap(size as i32, size as i32, &rgba) }
}

#[cfg(windows)]
pub fn generate_update_menu_bitmap(size: u32, is_dark: bool) -> Option<HBITMAP> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let cx = s * 0.5;
    let cy = s * 0.5;
    let hw = (s * 0.055).clamp(0.8, 1.3);

    // Accent emerald green color for update action
    let (r_col, g_col, b_col) = if is_dark {
        (0u8, 215u8, 120u8)
    } else {
        (0u8, 140u8, 80u8)
    };

    let p_shaft_top = (cx, cy - s * 0.30);
    let p_shaft_bot = (cx, cy + s * 0.14);
    let p_tip = (cx, cy + s * 0.14);
    let p_left = (cx - s * 0.22, cy - s * 0.04);
    let p_right = (cx + s * 0.22, cy - s * 0.04);
    let p_tray_left = (cx - s * 0.28, cy + s * 0.32);
    let p_tray_right = (cx + s * 0.28, cy + s * 0.32);

    let offsets = [(-0.25f32, -0.25f32), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];

    for y in 0..size {
        for x in 0..size {
            let mut cov = 0.0f32;
            for (ox, oy) in offsets {
                let p = (x as f32 + 0.5 + ox, y as f32 + 0.5 + oy);
                let d_shaft = dist_to_segment(p, p_shaft_top, p_shaft_bot) - hw;
                let d_head1 = dist_to_segment(p, p_left, p_tip) - hw;
                let d_head2 = dist_to_segment(p, p_right, p_tip) - hw;
                let d_tray = dist_to_segment(p, p_tray_left, p_tray_right) - hw;

                let min_d = d_shaft.min(d_head1).min(d_head2).min(d_tray);
                cov += (0.5 - min_d).clamp(0.0, 1.0);
            }
            cov *= 0.25;

            if cov > 0.015 {
                let idx = ((y * size + x) * 4) as usize;
                let a_byte = (cov * 255.0) as u8;
                rgba[idx] = r_col;
                rgba[idx + 1] = g_col;
                rgba[idx + 2] = b_col;
                rgba[idx + 3] = a_byte;
            }
        }
    }

    unsafe { create_pargb_bitmap(size as i32, size as i32, &rgba) }
}
