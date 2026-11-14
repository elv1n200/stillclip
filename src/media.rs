//! Range-based local video serving; paths never come from a web request.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
use wry::http::{Request, Response};

const CHUNK: u64 = 4 * 1024 * 1024;

fn range(value: &str, size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        return (suffix > 0).then_some((size.saturating_sub(suffix), size - 1));
    }
    let start: u64 = start.parse().ok()?;
    let end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().ok()?.min(size - 1)
    };
    (start <= end && start < size).then_some((start, end))
}

pub fn serve(path: &Path, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let fail = |status| {
        Response::builder()
            .status(status)
            .header("Access-Control-Allow-Origin", "http://app.localhost")
            .body(Vec::new())
            .unwrap()
    };
    if request.method() != "GET" && request.method() != "HEAD" {
        return fail(405);
    }
    let Ok(mut file) = File::open(path) else {
        return fail(404);
    };
    let Ok(info) = file.metadata() else {
        return fail(500);
    };
    let size = info.len();
    if size == 0 {
        return fail(416);
    }
    let (start, end, partial) = if let Some(value) = request.headers().get("Range") {
        let Some((start, end)) = value.to_str().ok().and_then(|v| range(v, size)) else {
            return Response::builder()
                .status(416)
                .header("Content-Range", format!("bytes */{size}"))
                .body(Vec::new())
                .unwrap();
        };
        (start, end.min(start.saturating_add(CHUNK - 1)), true)
    } else {
        if size > CHUNK && request.method() != "HEAD" {
            return fail(413);
        }
        (0, size - 1, false)
    };
    let length = end - start + 1;
    let mut bytes = Vec::new();
    if request.method() != "HEAD" {
        if file.seek(SeekFrom::Start(start)).is_err()
            || file.take(length).read_to_end(&mut bytes).is_err()
        {
            return fail(500);
        }
    }
    let mime = match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        _ => "video/mp4",
    };
    let mut builder = Response::builder()
        .status(if partial { 206 } else { 200 })
        .header("Content-Type", mime)
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", length.to_string())
        .header("Cache-Control", if mime.starts_with("image/") { "private, max-age=86400" } else { "no-store" })
        .header("Access-Control-Allow-Origin", "http://app.localhost");
    if partial {
        builder = builder.header("Content-Range", format!("bytes {start}-{end}/{size}"));
    }
    builder.body(bytes).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_ranges_support_seeking_and_reject_invalid_requests() {
        assert_eq!(range("bytes=3-8", 10), Some((3, 8)));
        assert_eq!(range("bytes=3-", 10), Some((3, 9)));
        assert_eq!(range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(range("bytes=0-99", 10), Some((0, 9)));
        for invalid in [
            "bytes=10-",
            "bytes=8-3",
            "bytes=-0",
            "bytes=1-2,4-5",
            "other=0-1",
        ] {
            assert_eq!(range(invalid, 10), None);
        }
        assert_eq!(range("bytes=0-", 0), None);
    }
}
