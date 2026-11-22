use super::*;

impl Purchases {
    pub(super) async fn start_download(&self, params: &Value) -> Result<(), String> {
        let format_id = params["format_id"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("Choose a format")?;
        let root = self
            .output
            .as_ref()
            .filter(|output| output.is_dir())
            .ok_or("Choose a download folder in the plugin settings first")?
            .clone();
        let mut state = self.state.lock().await;
        if state.working {
            return Err("A download is already running".into());
        }
        let selected = state.selected.clone().ok_or("Choose a purchase")?;
        let client = self.client.clone();
        let shared = self.state.clone();
        match selected {
            Selection::Album(id) => {
                let purchase = state
                    .albums
                    .iter()
                    .find(|a| a.id == id)
                    .cloned()
                    .ok_or("Unknown album")?;
                if !purchase.downloadable || !download::formats(&purchase).contains(&format_id) {
                    return Err("This format is not included in the purchase".into());
                }
                let track_id = params["track_id"]
                    .as_str()
                    .map(|s| s.parse::<u64>().map_err(|_| "Invalid track ID".to_string()))
                    .transpose()?;
                let one_track = if let Some(track_id) = track_id {
                    let album = state.detail.as_ref().ok_or("Open the album first")?;
                    let track = album
                        .tracks
                        .as_ref()
                        .and_then(|tracks| tracks.items.iter().find(|track| track.id == track_id))
                        .cloned()
                        .ok_or("This track is not in the purchased album")?;
                    Some((album.artist.name.clone(), album.title.clone(), track))
                } else {
                    None
                };
                state.working = true;
                state.completed = 0;
                state.total = if one_track.is_some() {
                    1
                } else {
                    purchase.tracks_count.unwrap_or(0) as usize
                };
                state.current = one_track
                    .as_ref()
                    .map(|(_, _, track)| track.title.clone())
                    .unwrap_or_default();
                state.report.clear();
                state.job = Some(tokio::spawn(async move {
                    let progress_state = shared.clone();
                    let result = if let Some((artist, album, listed)) = one_track {
                        let fetched = client
                            .get_track_for_purchase(listed.id)
                            .await
                            .map_err(|e| e.to_string());
                        match fetched {
                            Ok(track) => download::save_track(
                                &client, &artist, &album, &track, format_id, &root,
                            )
                            .await
                            .map(|()| 1),
                            Err(error) => Err(error),
                        }
                    } else {
                        download::album(
                            &client,
                            &purchase,
                            format_id,
                            &root,
                            move |done, total, title| {
                                let shared = progress_state.clone();
                                async move {
                                    let mut state = shared.lock().await;
                                    state.completed = done;
                                    state.total = total;
                                    state.current = title;
                                }
                            },
                        )
                        .await
                    };
                    let mut state = shared.lock().await;
                    state.working = false;
                    if result.is_ok() && state.total == 1 {
                        state.completed = 1;
                    }
                    state.current.clear();
                    state.report = match result {
                        Ok(count) => format!("Downloaded {count} tracks"),
                        Err(error) => format!("Download stopped: {error}"),
                    };
                }));
            }
            Selection::Track(id) => {
                let purchase = state
                    .tracks
                    .iter()
                    .find(|t| t.id == id)
                    .cloned()
                    .ok_or("Unknown track")?;
                if !download::track_formats(&purchase).contains(&format_id) {
                    return Err("This format is not included in the purchase".into());
                }
                state.working = true;
                state.completed = 0;
                state.total = 1;
                state.current = purchase.title.clone();
                state.report.clear();
                state.job = Some(tokio::spawn(async move {
                    let result = download::single(&client, &purchase, format_id, &root).await;
                    let mut state = shared.lock().await;
                    state.working = false;
                    state.completed = usize::from(result.is_ok());
                    state.current.clear();
                    state.report = match result {
                        Ok(()) => "Downloaded track".into(),
                        Err(error) => format!("Download stopped: {error}"),
                    };
                }));
            }
        }
        Ok(())
    }
}
