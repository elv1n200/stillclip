//! Captures the application's actual rendered WebView, without capturing the desktop.
use std::path::PathBuf;
use tao::event_loop::EventLoopProxy;

#[cfg(windows)]
pub fn capture(
    view: &wry::WebView,
    path: PathBuf,
    proxy: EventLoopProxy<crate::AppEvent>,
) -> Result<(), String> {
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    use windows::Win32::UI::Shell::SHCreateMemStream;
    use wry::WebViewExtWindows;

    let stream = unsafe { SHCreateMemStream(None) }
        .ok_or("Der Screenshot-Speicher konnte nicht erstellt werden.")?;
    // Keep the stream alive until WebView2 has completed its asynchronous capture.
    let completed_stream = stream.clone();
    let callback = CapturePreviewCompletedHandler::create(Box::new(move |status| {
        let result = status
            .map_err(|error| format!("WebView2 konnte den Screenshot nicht erstellen: {error}"))
            .and_then(|()| save_stream(&completed_stream, &path));
        let _ = proxy.send_event(crate::AppEvent::ScreenshotFinished(result));
        Ok(())
    }));
    unsafe {
        view.webview().CapturePreview(
            COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
            &stream,
            &callback,
        )
    }
    .map_err(|error| format!("Der Screenshot konnte nicht gestartet werden: {error}"))
}

#[cfg(windows)]
fn save_stream(
    stream: &windows::Win32::System::Com::IStream,
    path: &std::path::Path,
) -> Result<(), String> {
    use windows::Win32::System::Com::{STREAM_SEEK_END, STREAM_SEEK_SET};

    let mut length = 0_u64;
    unsafe { stream.Seek(0, STREAM_SEEK_END, Some(&mut length)) }
        .map_err(|error| format!("Die Screenshot-Größe konnte nicht gelesen werden: {error}"))?;
    if length < 24 || length > 256 * 1024 * 1024 {
        return Err(format!(
            "WebView2 hat eine ungültige Screenshot-Größe geliefert: {length}"
        ));
    }
    unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }
        .map_err(|error| format!("Der Screenshot konnte nicht gelesen werden: {error}"))?;
    let mut png = vec![0_u8; length as usize];
    let mut bytes_read = 0_u32;
    unsafe {
        stream.Read(
            png.as_mut_ptr().cast(),
            length as u32,
            Some(&mut bytes_read),
        )
    }
    .ok()
    .map_err(|error| format!("Der Screenshot konnte nicht gelesen werden: {error}"))?;
    if u64::from(bytes_read) != length || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("WebView2 hat keinen vollständigen PNG-Screenshot geliefert.".into());
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!("Der Screenshot-Ordner konnte nicht erstellt werden: {error}")
        })?;
    }
    std::fs::write(path, png)
        .map_err(|error| format!("Der Screenshot konnte nicht gespeichert werden: {error}"))
}

#[cfg(not(windows))]
pub fn capture(
    _view: &wry::WebView,
    _path: PathBuf,
    _proxy: EventLoopProxy<crate::AppEvent>,
) -> Result<(), String> {
    Err("Der Screenshot-Export benötigt Windows mit WebView2.".into())
}
