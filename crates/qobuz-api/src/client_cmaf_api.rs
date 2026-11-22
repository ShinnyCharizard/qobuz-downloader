use super::client::*;

impl QobuzClient {
    // === CMAF streaming endpoints ===

    /// Ensure we have a valid CMAF session, renewing if expired.
    /// Returns `(session_id, infos)` for use with file/url and key derivation.
    ///
    /// Concurrency note: this method serializes concurrent session
    /// renewals. Without that, two overlapping callers could both see
    /// "no session" on the read side, each POST /session/start, each get
    /// DIFFERENT `infos`, and the second one to finish would overwrite
    /// the first in the cache. Any `get_file_url` response whose wrapped
    /// key was tied to the first session then unwrapped with the second
    /// session's key and blew up with AES-CBC "Unpad Error" — which
    /// manifested as prefetch CMAF failures + downloaded-but-gappy
    /// transitions between offline tracks.
    ///
    /// Fix: a double-checked lock pattern on the write guard. Fast path
    /// uses a read guard; slow path acquires the write guard, re-checks
    /// under exclusive ownership, and only one caller hits the network.
    pub async fn ensure_cmaf_session(&self) -> Result<(String, String)> {
        let now = get_timestamp();

        // Fast path: existing session with > 60s left.
        {
            let guard = self.cmaf_session.read().await;
            if let Some(ref cs) = *guard {
                if cs.expires_at > now + 60 {
                    return Ok((cs.session_id.clone(), cs.infos.clone()));
                }
            }
        }

        // Slow path: take the write lock and re-check. Concurrent callers
        // end up here one at a time; after the first finishes POST
        // session/start, the rest find the freshly-populated cache and
        // return without hitting the network.
        let mut guard = self.cmaf_session.write().await;
        if let Some(ref cs) = *guard {
            if cs.expires_at > now + 60 {
                return Ok((cs.session_id.clone(), cs.infos.clone()));
            }
        }

        // We're the one task that actually starts a session. Back off before
        // the network if the 403 breaker is open — a cached
        // session above is still served; only the network POST is gated.
        self.forbidden_guard()?;
        log::info!("[CMAF] Starting new session");
        let timestamp = get_timestamp();
        let sig = sign_session_start(timestamp);

        let url = endpoints::build_url(paths::SESSION_START);
        let response = self
            .http()?
            .post(&url)
            .headers(self.authenticated_headers().await?)
            .form(&[
                ("profile", "qbz-1"),
                ("request_ts", &timestamp.to_string()),
                ("request_sig", &sig),
            ])
            .send()
            .await?;

        let status = response.status();
        // Feed the breaker: a 403 here counts toward opening it; success resets.
        self.note_forbidden_status(status);
        if !status.is_success() {
            if status == StatusCode::FORBIDDEN {
                let preview = body_preview(response).await;
                log::warn!("[CMAF] session/start 403{}", preview);
                return Err(ApiError::Forbidden(preview));
            }
            return Err(ApiError::ApiResponse(format!(
                "session/start failed with status {}",
                status
            )));
        }

        let resp: SessionStartResponse = response.json().await?;
        let infos = resp.infos.unwrap_or_default();
        log::info!(
            "[CMAF] Session started: id={}..., expires_at={}",
            &resp.session_id[..resp.session_id.len().min(8)],
            resp.expires_at
        );

        let session_id = resp.session_id.clone();
        let infos_clone = infos.clone();

        *guard = Some(CmafSession {
            session_id: resp.session_id,
            infos,
            expires_at: resp.expires_at,
        });

        Ok((session_id, infos_clone))
    }

    /// Get CMAF segmented file URL for a track.
    ///
    /// This is the new streaming endpoint that returns encrypted CMAF segments
    /// instead of a direct file URL.
    pub async fn get_file_url(&self, track_id: u64, quality: Quality) -> Result<TrackFileUrl> {
        let format_id = quality.id();
        let url = endpoints::build_url(paths::FILE_URL);

        // Back off before the network if the 403 breaker is open.
        self.forbidden_guard()?;

        // Retry transient failures (5xx / 429 / network blips) with backoff so
        // a momentary hiccup on the next track's file/url is not turned into a
        // queue skip. A real 404 → TrackUnavailable is terminal and returns
        // immediately to the (bounded) skip path.
        crate::retry::retry_transient(
            crate::retry::DEFAULT_MAX_ATTEMPTS,
            "CMAF file/url",
            ApiError::is_transient,
            |_attempt| {
                let url = url.clone();
                async move {
                    let (session_id, _infos) = self.ensure_cmaf_session().await?;

                    // Fresh timestamp + signature per attempt — the request
                    // signature is time-bound and would expire across retries.
                    let timestamp = get_timestamp();
                    let sig = sign_file_url(track_id, format_id, timestamp);

                    let mut headers = self.authenticated_headers().await?;
                    headers.insert(
                        "X-Session-Id",
                        reqwest::header::HeaderValue::from_str(&session_id).map_err(|_| {
                            ApiError::ApiResponse("Invalid session ID format".into())
                        })?,
                    );

                    let response = self
                        .http()?
                        .get(&url)
                        .headers(headers)
                        .query(&[
                            ("track_id", track_id.to_string()),
                            ("format_id", format_id.to_string()),
                            ("intent", "stream".to_string()),
                            ("request_ts", timestamp.to_string()),
                            ("request_sig", sig),
                        ])
                        .send()
                        .await?;

                    let status = response.status();
                    log::info!(
                        "[CMAF] file/url track_id={} format_id={} status={}",
                        track_id,
                        format_id,
                        status
                    );
                    // Feed the breaker: 403 counts toward opening it; 2xx resets.
                    self.note_forbidden_status(status);

                    if !status.is_success() {
                        let code = status.as_u16();
                        if status == reqwest::StatusCode::FORBIDDEN {
                            let preview = body_preview(response).await;
                            log::warn!("[CMAF] file/url 403 for track {}{}", track_id, preview);
                            return Err(ApiError::Forbidden(preview));
                        }
                        return Err(if code == 404 {
                            ApiError::TrackUnavailable(track_id)
                        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            // Honor Retry-After (seconds) when the server sends it.
                            let retry_after = response
                                .headers()
                                .get(reqwest::header::RETRY_AFTER)
                                .and_then(|v| v.to_str().ok())
                                .and_then(|s| s.trim().parse::<u64>().ok())
                                .unwrap_or(2);
                            ApiError::RateLimited(retry_after)
                        } else if status.is_server_error() {
                            ApiError::ServerError(code)
                        } else {
                            ApiError::ApiResponse(format!("file/url failed with status {}", status))
                        });
                    }

                    let file_url: TrackFileUrl = response.json().await?;
                    log::info!(
                        "[CMAF] file/url result: segments={}, mime={:?}, sampling_rate={:?}",
                        file_url.n_segments,
                        file_url.mime_type,
                        file_url.sampling_rate
                    );

                    Ok(file_url)
                }
            },
        )
        .await
    }
}
