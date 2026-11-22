//! Purchase grants and verified files, independent of the host library.
mod destination;
mod formats;

pub(crate) use destination::file_exists;
use destination::{extension, target, valid_container, Staged};
pub use formats::{formats, label, track_formats};
use qobuz_api::QobuzClient;
use qobuz_models::{PurchaseAlbum, PurchaseTrack, Track};
use std::{future::Future, path::Path};

pub async fn album<F, Fut>(
    client: &QobuzClient,
    purchase: &PurchaseAlbum,
    format_id: u32,
    root: &Path,
    mut progress: F,
) -> Result<usize, String>
where
    F: FnMut(usize, usize, String) -> Fut,
    Fut: Future<Output = ()>,
{
    if !purchase.downloadable || !formats(purchase).contains(&format_id) {
        return Err("This format is not included in the purchase".into());
    }
    let album = client
        .get_album_for_purchase(&purchase.id)
        .await
        .map_err(|e| e.to_string())?;
    let tracks = album
        .tracks
        .as_ref()
        .ok_or("The purchased album has no track list")?;
    if tracks.items.is_empty() || tracks.items.len() < tracks.total as usize {
        return Err("The purchased album has an incomplete track list".into());
    }
    let mut completed = 0;
    let total = tracks.items.len();
    for listed in &tracks.items {
        progress(completed, total, listed.title.clone()).await;
        let track = client
            .get_track_for_purchase(listed.id)
            .await
            .map_err(|e| e.to_string())?;
        save_track(
            client,
            &album.artist.name,
            &album.title,
            &track,
            format_id,
            root,
        )
        .await?;
        completed += 1;
        progress(completed, total, track.title.clone()).await;
    }
    Ok(completed)
}

pub async fn single(
    client: &QobuzClient,
    purchase: &PurchaseTrack,
    format_id: u32,
    root: &Path,
) -> Result<(), String> {
    if !track_formats(purchase).contains(&format_id) {
        return Err("This format is not included in the purchase".into());
    }
    let track = client
        .get_track_for_purchase(purchase.id)
        .await
        .map_err(|e| e.to_string())?;
    let album_title = purchase
        .album
        .as_ref()
        .map(|a| a.title.as_str())
        .unwrap_or("Singles");
    save_track(
        client,
        &purchase.performer.name,
        album_title,
        &track,
        format_id,
        root,
    )
    .await
}

pub(crate) async fn save_track(
    client: &QobuzClient,
    artist: &str,
    album: &str,
    track: &Track,
    format_id: u32,
    root: &Path,
) -> Result<(), String> {
    let grant = client
        .get_purchase_file_url(track.id, format_id)
        .await
        .map_err(|e| e.to_string())?;
    if grant.sample {
        return Err("The service returned a preview instead of the purchase".into());
    }
    let ext = extension(grant.format_id, &grant.mime_type);
    let path = target(root, artist, album, track, format_id, ext);
    let parent = path.parent().ok_or("Invalid download path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let staged = path.with_extension(format!("{ext}.part"));
    let _cleanup = Staged(staged.clone());
    QobuzClient::download_audio_to_path(&grant.url, &staged).await?;
    valid_container(&staged, ext)?;
    std::fs::rename(&staged, &path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::destination::safe;
    use super::*;
    #[test]
    fn only_entitled_formats_are_offered() {
        let purchase = PurchaseAlbum {
            downloadable_format_ids: vec![55, 55, 6],
            ..Default::default()
        };
        assert_eq!(formats(&purchase), vec![55, 6]);
    }
    #[test]
    fn filenames_cannot_escape_the_destination() {
        assert!(!safe("../../Album: name").contains('/'));
    }
}
