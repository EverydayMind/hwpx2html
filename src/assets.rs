use base64::Engine;

use crate::model::AssetRef;

pub fn data_uri(asset: &AssetRef) -> Option<String> {
    let mime = raster_mime(asset)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&asset.data);
    Some(format!("data:{mime};base64,{encoded}"))
}

pub fn raster_mime(asset: &AssetRef) -> Option<&str> {
    let mime = sniff_mime(&asset.data).or({
        if asset.mime_type.is_empty() {
            None
        } else {
            Some(asset.mime_type.as_str())
        }
    })?;
    if !matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/bmp") {
        return None;
    }
    Some(mime)
}

pub fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.starts_with(b"\xD7\xCD\xC6\x9A") {
        Some("image/wmf")
    } else {
        None
    }
}
