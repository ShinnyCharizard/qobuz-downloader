use super::label;
use qobuz_models::Track;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub(super) fn safe(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if "\\/:*?\"<>|".contains(c) || !(c.is_ascii() || c.is_alphanumeric()) {
                '-'
            } else {
                c
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out = out.trim_matches('-').trim().to_string();
    if out.len() > 200 {
        let end = (0..=200)
            .rev()
            .find(|&i| out.is_char_boundary(i))
            .unwrap_or(0);
        out.truncate(end);
        out = out.trim_matches('-').trim().to_string();
    }
    if out.is_empty() || out == "." || out == ".." {
        out = "track".into();
    }
    out
}

pub(super) fn extension(format_id: u32, mime: &str) -> &'static str {
    let mime = mime.to_ascii_lowercase();
    if format_id == 5 || mime.contains("mpeg") {
        "mp3"
    } else if mime.contains("dsf") || mime.is_empty() && matches!(format_id, 55 | 56) {
        "dsf"
    } else if mime.contains("dff") || mime.contains("dsdiff") {
        "dff"
    } else if mime.contains("wav") {
        "wav"
    } else if mime.contains("aiff") || mime.contains("x-aif") {
        "aiff"
    } else if mime.contains("mp4") || mime.contains("m4a") || mime.contains("alac") {
        "m4a"
    } else {
        "flac"
    }
}

pub(super) fn valid_container(path: &Path, ext: &str) -> Result<(), String> {
    let mut head = [0u8; 12];
    let size = std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .read(&mut head)
        .map_err(|e| e.to_string())?;
    let head = &head[..size];
    let has = |signature: &[u8]| head.starts_with(signature);
    let valid = match ext {
        "dsf" => has(b"DSD "),
        "dff" => has(b"FRM8"),
        "flac" => has(b"fLaC") || has(b"ID3"),
        "mp3" => has(b"ID3") || head.len() >= 2 && head[0] == 0xff && head[1] & 0xe0 == 0xe0,
        "wav" => has(b"RIFF"),
        "aiff" => has(b"FORM"),
        "m4a" => head.len() >= 8 && &head[4..8] == b"ftyp",
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("Downloaded file is not {ext} audio"))
    }
}

pub(super) struct Staged(pub(super) PathBuf);
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(super) fn target(
    root: &Path,
    artist: &str,
    album: &str,
    track: &Track,
    format_id: u32,
    ext: &str,
) -> PathBuf {
    let folder =
        root.join(safe(artist))
            .join(format!("{} {}", safe(album), safe(&label(format_id))));
    let title = safe(&track.title);
    let number = if track.media_number.unwrap_or(1) > 1 && track.track_number > 0 {
        format!(
            "{:02}-{:02}",
            track.media_number.unwrap_or(1),
            track.track_number
        )
    } else if track.track_number > 0 {
        format!("{:02}", track.track_number)
    } else {
        String::new()
    };
    if number.is_empty() {
        folder.join(format!("{title}.{ext}"))
    } else {
        folder.join(format!("{number} - {title}.{ext}"))
    }
}

pub(crate) fn file_exists(
    root: &Path,
    artist: &str,
    album: &str,
    track: &Track,
    format_id: u32,
) -> bool {
    let ext = extension(format_id, "");
    let path = target(root, artist, album, track, format_id, ext);
    path.is_file() && valid_container(&path, ext).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_a_transfer_removes_its_partial_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("track.flac.part");
        let partial = path.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let transfer = tokio::spawn(async move {
            let _staged = Staged(partial.clone());
            std::fs::write(partial, b"fLaCpartial").unwrap();
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        assert!(path.is_file());
        transfer.abort();
        assert!(transfer.await.unwrap_err().is_cancelled());
        assert!(!path.exists());
    }

    #[test]
    fn an_error_page_is_not_treated_as_a_completed_download() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("track.flac");
        std::fs::write(&path, b"<!DOCTYPE html>not audio").unwrap();
        assert!(valid_container(&path, "flac").is_err());
        let track = Track {
            title: "Song".into(),
            ..Default::default()
        };
        let destination = target(directory.path(), "Artist", "Album", &track, 6, "flac");
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&destination, b"<html>denied</html>").unwrap();
        assert!(!file_exists(directory.path(), "Artist", "Album", &track, 6));
        std::fs::write(&destination, b"fLaCfixture").unwrap();
        assert!(file_exists(directory.path(), "Artist", "Album", &track, 6));
    }
}
