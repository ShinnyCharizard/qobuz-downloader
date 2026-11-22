use qobuz_api::QobuzClient;
use qobuz_models::{PurchaseAlbum, PurchaseTrack};

pub(super) async fn all_albums(client: &QobuzClient) -> Result<Vec<PurchaseAlbum>, String> {
    let mut items = Vec::new();
    loop {
        let offset = items.len() as u32;
        let page = client
            .get_user_purchases_page_typed_checked(Some("albums"), 100, offset)
            .await
            .map_err(|e| e.to_string())?;
        let count = page.albums.items.len();
        let total = page.albums.total;
        items.extend(page.albums.items);
        if count == 0 || items.len() >= total as usize {
            return Ok(items);
        }
    }
}

pub(super) async fn all_tracks(client: &QobuzClient) -> Result<Vec<PurchaseTrack>, String> {
    let mut items = Vec::new();
    loop {
        let offset = items.len() as u32;
        let page = client
            .get_user_purchases_page_typed_checked(Some("tracks"), 100, offset)
            .await
            .map_err(|e| e.to_string())?;
        let count = page.tracks.items.len();
        let total = page.tracks.total;
        items.extend(page.tracks.items);
        if count == 0 || items.len() >= total as usize {
            return Ok(items);
        }
    }
}
