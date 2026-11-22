//! What the system clipboard holds for pasting into Files (⌘V / Ctrl+V): files
//! copied in Finder, Explorer or a Linux file manager (their paths), text, or
//! an image (a screenshot copied to the clipboard), which is written to a
//! private temporary PNG so it can be uploaded like any file. Read only when
//! the user pastes. Works on macOS, Windows and Linux (X11 and Wayland) via
//! arboard.

use serde::Serialize;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardContents {
    /// Local paths of copied files and folders.
    pub files: Vec<String>,
    pub text: Option<String>,
    /// A temporary PNG holding a copied image, when there are no files.
    pub image: Option<String>,
    /// False when the clipboard couldn't be opened (the page then uses the web view's own).
    pub native: bool,
}

pub fn read() -> ClipboardContents {
    let Ok(mut cb) = arboard::Clipboard::new() else {
        return ClipboardContents::default();
    };
    let files: Vec<String> = cb
        .get()
        .file_list()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.is_absolute())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let text = cb.get_text().ok().filter(|t| !t.is_empty());
    let image = if files.is_empty() {
        cb.get_image()
            .ok()
            .and_then(|i| encode_png(i.width, i.height, &i.bytes))
            .and_then(|png| save_image(&png))
    } else {
        None
    };
    ClipboardContents {
        files,
        text,
        image,
        native: true,
    }
}

/// RGBA pixels → PNG.
fn encode_png(width: usize, height: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || rgba.len() != width * height * 4 {
        return None;
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width as u32, height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().ok()?;
        w.write_image_data(rgba).ok()?;
    }
    Some(out)
}

/// Writes a pasted image to a private temporary folder; its name says when it was pasted.
fn save_image(png: &[u8]) -> Option<String> {
    if png.is_empty() || png.len() > 200 * 1024 * 1024 {
        return None;
    }
    let dir = std::env::temp_dir().join("orbital-paste");
    std::fs::create_dir_all(&dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let path = dir.join(format!("{}.png", image_name(secs)));
    std::fs::write(&path, png).ok()?;
    Some(path.to_string_lossy().into_owned())
}

/// "Pasted image 2026-11-22 at 10.15.42" (UTC), like a macOS screenshot's name.
fn image_name(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let rem = unix % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "Pasted image {y:04}-{m:02}-{d:02} at {:02}.{:02}.{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_names_carry_the_time() {
        assert_eq!(image_name(0), "Pasted image 1970-01-01 at 00.00.00");
        assert_eq!(image_name(1_795_336_716), "Pasted image 2026-11-22 at 08.38.36");
        assert_eq!(image_name(951_782_400), "Pasted image 2000-02-29 at 00.00.00");
    }

    #[test]
    fn copied_pixels_become_a_png() {
        let png = encode_png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let dec = png::Decoder::new(std::io::Cursor::new(png));
        let r = dec.read_info().unwrap();
        assert_eq!((r.info().width, r.info().height), (2, 1));
        assert!(encode_png(2, 2, &[0; 4]).is_none(), "wrong size");
    }

    #[test]
    fn images_go_to_a_private_folder() {
        let p = save_image(b"\x89PNG fake").unwrap();
        assert!(p.contains("orbital-paste") && p.ends_with(".png"), "{p}");
        assert_eq!(std::fs::read(&p).unwrap(), b"\x89PNG fake");
        assert!(save_image(b"").is_none());
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod live {
    /// Run by hand: `cargo test -p orbital clipboard::live -- --ignored --nocapture` after copying something.
    #[test]
    #[ignore]
    fn print_what_the_clipboard_holds() {
        println!("{:?}", super::read());
    }
}
