use super::*;

impl Purchases {
    pub(super) async fn document(&self, signed_in: bool) -> Value {
        let state = self.state.lock().await;
        let selected = match &state.selected {
            Some(Selection::Album(id)) => state.albums.iter().find(|a| &a.id == id).map(|purchase| {
                let format_ids = download::formats(purchase);
                let rows: Vec<Value> = state.detail.as_ref().and_then(|a| a.tracks.as_ref()).map(|tracks| tracks.items.iter().map(|t| json!({
                    "id":t.id.to_string(),"title":t.title,"version":t.version,
                    "artist":t.performer.as_ref().map(|a| a.name.as_str()).unwrap_or(""),
                    "disc":t.media_number.unwrap_or(1),"number":t.track_number,"duration":t.duration,
                    "downloaded_formats":format_ids.iter().filter(|format_id| self.output.as_ref().is_some_and(|output| download::file_exists(output, &purchase.artist.name, &purchase.title, t, **format_id))).collect::<Vec<_>>()
                })).collect()).unwrap_or_default();
                json!({"kind":"album","id":id,"title":purchase.title,"artist":purchase.artist.name,
                    "image":state.detail.as_ref().and_then(|a| a.image.best()).or_else(|| purchase.image.best()),
                    "year":purchase.release_date_original,"tracks":rows,
                    "formats":format_rows(download::formats(purchase)),"downloadable":purchase.downloadable})
            }),
            Some(Selection::Track(id)) => state.tracks.iter().find(|t| &t.id == id).map(|purchase| json!({
                "kind":"track","id":id.to_string(),"title":purchase.title,"artist":purchase.performer.name,
                "album":purchase.album.as_ref().map(|a| a.title.as_str()).unwrap_or("Singles"),
                "image":purchase.album.as_ref().and_then(|a| a.image.best()),
                "formats":format_rows(download::track_formats(purchase)),"duration":purchase.duration
            })),
            None => None,
        };
        json!({"signed_in":signed_in,"loading":state.loading,"loaded":state.loaded,"error":state.error,
            "albums":state.albums.iter().map(album_card).collect::<Vec<_>>(),
            "tracks":state.tracks.iter().map(track_card).collect::<Vec<_>>(),
            "selected":selected,"working":state.working,
            "progress":{"completed":state.completed,"total":state.total,"current":state.current},
            "report":state.report,"destination_ready":self.output.as_ref().is_some_and(|output| output.is_dir())})
    }
}

fn format_rows(ids: Vec<u32>) -> Vec<Value> {
    ids.into_iter()
        .map(|id| json!({"id":id,"label":download::label(id)}))
        .collect()
}

fn album_card(album: &PurchaseAlbum) -> Value {
    json!({"id":album.id,"title":album.title,"artist":album.artist.name,"image":album.image.best(),
        "year":album.release_date_original,"tracks_count":album.tracks_count,"downloadable":album.downloadable,
        "purchased_at":album.purchased_at,"formats":format_rows(download::formats(album))})
}

fn track_card(track: &PurchaseTrack) -> Value {
    json!({"id":track.id.to_string(),"title":track.title,"version":track.version,"artist":track.performer.name,
        "album":track.album.as_ref().map(|a| a.title.as_str()).unwrap_or("Singles"),
        "image":track.album.as_ref().and_then(|a| a.image.best()),"duration":track.duration,
        "purchased_at":track.purchased_at,"formats":format_rows(download::track_formats(track))})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn track_ids_remain_strings_for_qml() {
        let track = PurchaseTrack {
            id: u64::MAX,
            ..Default::default()
        };
        assert_eq!(track_card(&track)["id"], u64::MAX.to_string());
    }
}
