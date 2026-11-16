//! Native icon using the same pixels embedded in the executable.
#[path = "icon_pixels.rs"]
mod pixels;
pub use pixels::{rgba, SIZE};

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
    unsafe {
        CreateIcon(
            std::ptr::null_mut(),
            size as i32,
            size as i32,
            1,
            32,
            mask.as_ptr(),
            colour.as_ptr(),
        )
    }
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
        assert!(
            pixels[centre] > 200 && pixels[centre + 2] > 150,
            "centre dot uses the accent colour"
        );
        // The design is symmetric, so the icon reads correctly in either row order.
        let top = &pixels[(4 * SIZE * 4) as usize..(5 * SIZE * 4) as usize];
        let bottom = &pixels[((SIZE - 5) * SIZE * 4) as usize..((SIZE - 4) * SIZE * 4) as usize];
        assert_eq!(top, bottom);
    }
}
