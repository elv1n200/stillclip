//! The application icon, rasterized at runtime so no image assets ship.
//! A square outline with a square in the middle, matching the brand mark.

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
    let background = (9.0, 9.0, 9.0);
    let accent = (229.0, 160.0, 183.0);
    for y in 0..size {
        for x in 0..size {
            let px = (x as f32 + 0.5 - centre) / scale;
            let py = (y as f32 + 0.5 - centre) / scale;
            // Square outline, matching the terminal interface.
            let outer = rounded_box(px, py, 13.0, 0.0);
            let inner = rounded_box(px, py, 10.6, 0.0);
            let dot = rounded_box(px, py, 4.0, 0.0);
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

