//! The application icon, rasterized at runtime so no image assets ship.
//! A rounded square outline with a dot in the middle, matching the brand mark.

pub const SIZE: u32 = 32;

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Signed distance to a rounded rectangle centred at the origin.
fn rounded_box(x: f32, y: f32, half: f32, radius: f32) -> f32 {
    let qx = x.abs() - half + radius;
    let qy = y.abs() - half + radius;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

/// Premultiplied-free RGBA pixels, row-major, top row first.
pub fn rgba(size: u32) -> Vec<u8> {
    let mut pixels = vec![0_u8; (size * size * 4) as usize];
    let scale = size as f32 / 32.0;
    let centre = size as f32 / 2.0;
    let background = (21.0, 22.0, 27.0);
    let accent = (190.0, 176.0, 255.0);
    for y in 0..size {
        for x in 0..size {
            let px = (x as f32 + 0.5 - centre) / scale;
            let py = (y as f32 + 0.5 - centre) / scale;
            // Outer rounded square: 26 px wide with a 7 px corner radius.
            let outer = rounded_box(px, py, 13.0, 7.0);
            let inner = rounded_box(px, py, 10.6, 5.0);
            let dot = (px * px + py * py).sqrt() - 4.0;
            let shape = 1.0 - smoothstep(-0.6, 0.6, outer);
            let ring = (1.0 - smoothstep(-0.6, 0.6, outer)) * smoothstep(-0.6, 0.6, inner);
            let dot_alpha = 1.0 - smoothstep(-0.6, 0.6, dot);
            let accent_mix = ring.max(dot_alpha);
            let r = background.0 + (accent.0 - background.0) * accent_mix;
            let g = background.1 + (accent.1 - background.1) * accent_mix;
            let b = background.2 + (accent.2 - background.2) * accent_mix;
            let alpha = shape;
            let offset = ((y * size + x) * 4) as usize;
            pixels[offset] = r.round() as u8;
            pixels[offset + 1] = g.round() as u8;
            pixels[offset + 2] = b.round() as u8;
            pixels[offset + 3] = (alpha * 255.0).round() as u8;
        }
    }
    pixels
}

/// Same image as BGRA bytes for Win32 `CreateIcon`.
pub fn bgra(size: u32) -> Vec<u8> {
    let mut pixels = rgba(size);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    pixels
}

#[cfg(windows)]
pub fn hicon(size: u32) -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    use windows_sys::Win32::UI::WindowsAndMessaging::CreateIcon;
    let colour = bgra(size);
    let mask = vec![0_u8; ((size + 7) / 8 * size) as usize];
    unsafe { CreateIcon(std::ptr::null_mut(), size as i32, size as i32, 1, 32, mask.as_ptr(), colour.as_ptr()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_transparent_corners_and_an_accent_centre() {
        let pixels = rgba(SIZE);
        assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(pixels[3], 0, "corner must be transparent");
        let centre = (((SIZE / 2) * SIZE + SIZE / 2) * 4) as usize;
        assert_eq!(pixels[centre + 3], 255);
        assert!(pixels[centre + 2] > 200, "centre dot uses the accent colour");
        // The design is symmetric, so the icon reads correctly in either row order.
        let top = &pixels[(4 * SIZE * 4) as usize..(5 * SIZE * 4) as usize];
        let bottom = &pixels[((SIZE - 5) * SIZE * 4) as usize..((SIZE - 4) * SIZE * 4) as usize];
        assert_eq!(top, bottom);
    }
}
