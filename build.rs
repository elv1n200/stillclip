#[allow(dead_code)]
#[path = "src/icon_pixels.rs"]
mod pixels;

fn main() {
    println!("cargo:rerun-if-changed=src/icon_pixels.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") { return; }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let sizes = [16u32, 32, 48, 256];
    let mut ico = vec![0, 0, 1, 0, sizes.len() as u8, 0];
    let mut images = Vec::new();
    let mut offset = 6 + 16 * sizes.len() as u32;
    for size in sizes {
        let rgba = pixels::rgba(size);
        let mask = ((size + 31) / 32) * 4 * size;
        let length = 40 + size * size * 4 + mask;
        ico.extend_from_slice(&[size as u8, size as u8, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&length.to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += length;
        let mut dib = Vec::new();
        for value in [40u32, size, size * 2] { dib.extend_from_slice(&value.to_le_bytes()); }
        dib.extend_from_slice(&[1, 0, 32, 0]);
        for value in [0, size * size * 4, 0, 0, 0, 0] { dib.extend_from_slice(&u32::to_le_bytes(value)); }
        for row in rgba.chunks_exact((size * 4) as usize).rev() {
            for pixel in row.chunks_exact(4) { dib.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]); }
        }
        dib.resize(length as usize, 0);
        images.extend(dib);
    }
    ico.extend(images);
    let icon = out.join("stillclip.ico");
    std::fs::write(&icon, ico).unwrap();
    winresource::WindowsResource::new()
        .set_icon(icon.to_str().unwrap())
        .set("FileDescription", "Stillclip — local game clips")
        .set("ProductName", "Stillclip")
        .set("OriginalFilename", "stillclip.exe")
        .compile().expect("Windows executable resources");
}
