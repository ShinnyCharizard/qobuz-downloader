//! A catalogue document for a host-rendered purchase screen. No library writes.
mod document;
mod jobs;
mod paging;

use crate::transfer as download;
use paging::{all_albums, all_tracks};
use qobuz_api::QobuzClient;
use qobuz_models::{Album, PurchaseAlbum, PurchaseTrack};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

pub struct Purchases {
    client: Arc<QobuzClient>,
    output: Option<PathBuf>,
    state: Arc<Mutex<State>>,
}

#[derive(Clone)]
enum Selection {
    Album(String),
    Track(u64),
}

#[derive(Default)]
struct State {
    albums: Vec<PurchaseAlbum>,
    tracks: Vec<PurchaseTrack>,
    detail: Option<Album>,
    selected: Option<Selection>,
    loaded: bool,
    loading: bool,
    list_job: Option<tokio::task::JoinHandle<()>>,
    generation: u64,
    error: String,
    working: bool,
    job: Option<tokio::task::JoinHandle<()>>,
    completed: usize,
    total: usize,
    current: String,
    report: String,
}

impl Purchases {
    pub fn new(client: Arc<QobuzClient>, output: Option<PathBuf>) -> Self {
        Self {
            client,
            output,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    pub async fn clear(&self) {
        let mut state = self.state.lock().await;
        if let Some(job) = state.job.take() {
            job.abort();
        }
        if let Some(job) = state.list_job.take() {
            job.abort();
        }
        let generation = state.generation.wrapping_add(1);
        *state = State {
            generation,
            ..State::default()
        };
    }

    pub async fn open(&self, signed_in: bool) -> Value {
        if signed_in {
            self.load_if_needed().await;
        }
        self.document(signed_in).await
    }

    async fn load_if_needed(&self) {
        let mut state = self.state.lock().await;
        if state.loaded || state.loading {
            return;
        }
        state.loading = true;
        state.error.clear();
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        let client = self.client.clone();
        let shared = self.state.clone();
        state.list_job = Some(tokio::spawn(async move {
            let (albums, tracks) = tokio::join!(all_albums(&client), all_tracks(&client));
            let mut state = shared.lock().await;
            if state.generation != generation {
                return;
            }
            state.loading = false;
            match (albums, tracks) {
                (Ok(albums), Ok(tracks)) => {
                    state.albums = albums;
                    state.tracks = tracks;
                    state.loaded = true;
                }
                (Err(error), _) | (_, Err(error)) => {
                    state.error = error;
                    state.loaded = true;
                }
            }
        }));
    }

    pub async fn action(&self, params: Value) -> Result<Value, String> {
        match params["action"].as_str().unwrap_or("") {
            "refresh" => {
                let mut state = self.state.lock().await;
                if let Some(job) = state.list_job.take() {
                    job.abort();
                }
                state.loaded = false;
                state.loading = false;
                state.selected = None;
                state.detail = None;
                drop(state);
                self.load_if_needed().await;
            }
            "back" => {
                let mut state = self.state.lock().await;
                state.selected = None;
                state.detail = None;
            }
            "select_album" => {
                let id = params["id"].as_str().ok_or("Choose an album")?;
                let known = self.state.lock().await.albums.iter().any(|a| a.id == id);
                if !known {
                    return Err("This album is not in your purchases".into());
                }
                let detail = self
                    .client
                    .get_album_for_purchase(id)
                    .await
                    .map_err(|e| e.to_string())?;
                let mut state = self.state.lock().await;
                state.selected = Some(Selection::Album(id.into()));
                state.detail = Some(detail);
            }
            "select_track" => {
                let id = params["id"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .ok_or("Choose a track")?;
                let mut state = self.state.lock().await;
                if !state.tracks.iter().any(|t| t.id == id) {
                    return Err("This track is not in your purchases".into());
                }
                state.selected = Some(Selection::Track(id));
                state.detail = None;
            }
            "cancel" => {
                let mut state = self.state.lock().await;
                if let Some(job) = state.job.take() {
                    job.abort();
                }
                state.working = false;
                state.current.clear();
                state.report = "Download canceled".into();
            }
            "download" => self.start_download(&params).await?,
            _ => return Err("Unknown download action".into()),
        }
        Ok(self.document(true).await)
    }
}
