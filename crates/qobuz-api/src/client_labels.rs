use super::client::*;

impl QobuzClient {
    pub async fn get_label_page(&self, label_id: u64) -> Result<LabelPageData> {
        let url = endpoints::build_url(paths::LABEL_PAGE);

        log::debug!("[API] get_label_page({})", label_id);
        let response: serde_json::Value = self
            .signed_get(&url, "labelpage", &[("label_id", label_id.to_string())])
            .await?
            .json()
            .await?;

        Ok(serde_json::from_value(response)?)
    }

    /// Enumerate the award catalog (/award/explore). Paginated.
    /// Returns raw JSON items so callers can extract just (id, name).
    pub async fn get_award_explore(&self, limit: u32, offset: u32) -> Result<serde_json::Value> {
        let url = endpoints::build_url(paths::AWARD_EXPLORE);
        log::debug!(
            "[API] get_award_explore(limit={}, offset={})",
            limit,
            offset
        );
        let response: serde_json::Value = self
            .signed_get_auth(
                &url,
                "awardexplore",
                &[("limit", limit.to_string()), ("offset", offset.to_string())],
            )
            .await?
            .json()
            .await?;
        Ok(response)
    }

    /// Get award page — hero info + categorized award-winning releases.
    /// Mirrors Android's AwardDto shape from /award/page. Uses auth
    /// headers (app_id + user token) — region-gated catalog.
    pub async fn get_award_page(&self, award_id: &str) -> Result<qobuz_models::AwardPageData> {
        let url = endpoints::build_url(paths::AWARD_PAGE);
        log::debug!("[API] get_award_page({})", award_id);
        let response: serde_json::Value = self
            .signed_get_auth(&url, "awardpage", &[("award_id", award_id.to_string())])
            .await?
            .json()
            .await?;
        Ok(serde_json::from_value(response)?)
    }

    /// Get paginated albums for a single award (/award/getAlbums).
    /// Response is V2AlbumGenericListDto = {has_more, items:[DiscographyAlbumDto]}
    /// per k20/b.java. DiscographyAlbumDto carries artists[] plural and
    /// artist singular as an optional DiscographyArtistDto — we backfill
    /// artist from artists[0] when missing (same trick as release watch).
    pub async fn get_award_albums(
        &self,
        award_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<SearchResultsPage<Album>> {
        let url = endpoints::build_url(paths::AWARD_GET_ALBUMS);
        log::debug!(
            "[API] get_award_albums({}, limit={}, offset={})",
            award_id,
            limit,
            offset
        );
        let http_response = self
            .signed_get_auth(
                &url,
                "awardgetAlbums",
                &[
                    ("award_id", award_id.to_string()),
                    ("limit", limit.to_string()),
                    ("offset", offset.to_string()),
                ],
            )
            .await?;
        let status = http_response.status();
        let body_text = http_response.text().await?;
        if !status.is_success() {
            log::warn!(
                "[API] get_award_albums non-success body (first 400 chars): {}",
                body_text.chars().take(400).collect::<String>()
            );
            return Err(ApiError::ApiResponse(format!(
                "get_award_albums status {}",
                status
            )));
        }
        let response: serde_json::Value = serde_json::from_str(&body_text)?;

        // Legacy shape some endpoints use: {albums: {items, total, offset, limit}}
        if let Some(albums_obj) = response.get("albums") {
            return Ok(serde_json::from_value(albums_obj.clone())?);
        }

        // Current shape: V2AlbumGenericListDto {has_more, items}.
        let mut items_value = response
            .get("items")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        if let Some(items_arr) = items_value.as_array_mut() {
            for item in items_arr {
                if let Some(obj) = item.as_object_mut() {
                    let needs_backfill = obj.get("artist").map(|v| v.is_null()).unwrap_or(true);
                    if needs_backfill {
                        if let Some(first_artist) = obj
                            .get("artists")
                            .and_then(|a| a.as_array())
                            .and_then(|arr| arr.first())
                            .cloned()
                        {
                            obj.insert("artist".to_string(), first_artist);
                        }
                    }
                }
            }
        }
        let items: Vec<Album> = serde_json::from_value(items_value).map_err(|e| {
            log::warn!(
                "[API] get_award_albums items parse error: {}. Body (first 600 chars): {}",
                e,
                body_text.chars().take(600).collect::<String>()
            );
            ApiError::ApiResponse(format!("get_award_albums items parse: {}", e))
        })?;

        let has_more = response
            .get("has_more")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let total_hint = if has_more {
            offset + items.len() as u32 + 1
        } else {
            offset + items.len() as u32
        };
        log::info!(
            "[API] get_award_albums({}) parsed {} items (has_more={})",
            award_id,
            items.len(),
            has_more
        );
        Ok(SearchResultsPage::<Album> {
            items,
            total: total_hint,
            offset,
            limit,
        })
    }

    /// Get label explore (discover more labels)
    pub async fn get_label_explore(&self, limit: u32, offset: u32) -> Result<LabelExploreResponse> {
        let url = endpoints::build_url(paths::LABEL_EXPLORE);

        log::debug!(
            "[API] get_label_explore(limit={}, offset={})",
            limit,
            offset
        );
        let response: serde_json::Value = self
            .signed_get(
                &url,
                "labelexplore",
                &[("limit", limit.to_string()), ("offset", offset.to_string())],
            )
            .await?
            .json()
            .await?;

        Ok(serde_json::from_value(response)?)
    }

    /// Get a label's album catalog (paginated, filterable).
    ///
    /// Replaces the legacy `/label/get?extra=albums` path.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_label_albums(
        &self,
        label_id: u64,
        limit: u32,
        offset: u32,
        sort: Option<&str>,
        order: Option<&str>,
        genre_ids: Option<&str>,
        from_date: Option<&str>,
        to_date: Option<&str>,
    ) -> Result<LabelListPage<Album>> {
        let url = endpoints::build_url(paths::LABEL_GET_ALBUMS);
        let mut params: Vec<(&str, String)> = vec![
            ("label_id", label_id.to_string()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        if let Some(v) = sort {
            params.push(("sort", v.to_string()));
        }
        if let Some(v) = order {
            params.push(("order", v.to_string()));
        }
        if let Some(v) = genre_ids {
            params.push(("genre_ids", v.to_string()));
        }
        if let Some(v) = from_date {
            params.push(("from_date", v.to_string()));
        }
        if let Some(v) = to_date {
            params.push(("to_date", v.to_string()));
        }

        log::debug!(
            "[API] get_label_albums({}, limit={}, offset={})",
            label_id,
            limit,
            offset
        );
        let response: Value = self
            .signed_get(&url, "labelgetalbums", &params)
            .await?
            .json()
            .await?;
        Ok(serde_json::from_value(response)?)
    }

    /// Get a label's upcoming releases.
    pub async fn get_label_next_releases(
        &self,
        label_id: u64,
        limit: u32,
        offset: u32,
        genre_ids: Option<&str>,
    ) -> Result<LabelListPage<Album>> {
        let url = endpoints::build_url(paths::LABEL_GET_NEXT_RELEASES);
        let mut params: Vec<(&str, String)> = vec![
            ("label_id", label_id.to_string()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        if let Some(v) = genre_ids {
            params.push(("genre_ids", v.to_string()));
        }

        log::debug!("[API] get_label_next_releases({})", label_id);
        let response: Value = self
            .signed_get(&url, "labelgetnextreleases", &params)
            .await?
            .json()
            .await?;
        Ok(serde_json::from_value(response)?)
    }

    /// Get a label's press-awarded releases.
    pub async fn get_label_awarded_releases(
        &self,
        label_id: u64,
        limit: u32,
        offset: u32,
        sort: Option<&str>,
        order: Option<&str>,
        genre_ids: Option<&str>,
    ) -> Result<LabelListPage<Album>> {
        let url = endpoints::build_url(paths::LABEL_GET_AWARDED_RELEASES);
        let mut params: Vec<(&str, String)> = vec![
            ("label_id", label_id.to_string()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        if let Some(v) = sort {
            params.push(("sort", v.to_string()));
        }
        if let Some(v) = order {
            params.push(("order", v.to_string()));
        }
        if let Some(v) = genre_ids {
            params.push(("genre_ids", v.to_string()));
        }

        log::debug!("[API] get_label_awarded_releases({})", label_id);
        let response: Value = self
            .signed_get(&url, "labelgetawardedreleases", &params)
            .await?
            .json()
            .await?;
        Ok(serde_json::from_value(response)?)
    }

    /// Get a label's curated playlists (paginated).
    pub async fn get_label_playlists(
        &self,
        label_id: u64,
        limit: u32,
        offset: u32,
    ) -> Result<LabelListPage<Playlist>> {
        let url = endpoints::build_url(paths::LABEL_GET_PLAYLISTS);
        log::debug!("[API] get_label_playlists({})", label_id);
        let response: Value = self
            .signed_get(
                &url,
                "labelgetplaylists",
                &[
                    ("label_id", label_id.to_string()),
                    ("limit", limit.to_string()),
                    ("offset", offset.to_string()),
                ],
            )
            .await?
            .json()
            .await?;
        Ok(serde_json::from_value(response)?)
    }

    /// Get a label's top artists (paginated).
    pub async fn get_label_top_artists(
        &self,
        label_id: u64,
        limit: u32,
        offset: u32,
    ) -> Result<LabelListPage<Artist>> {
        let url = endpoints::build_url(paths::LABEL_GET_TOP_ARTISTS);
        log::debug!("[API] get_label_top_artists({})", label_id);
        let response: Value = self
            .signed_get(
                &url,
                "labelgettopartists",
                &[
                    ("label_id", label_id.to_string()),
                    ("limit", limit.to_string()),
                    ("offset", offset.to_string()),
                ],
            )
            .await?
            .json()
            .await?;
        Ok(serde_json::from_value(response)?)
    }

    /// Get a label's editorial / story content.
    pub async fn get_label_story(
        &self,
        label_id: u64,
        limit: u32,
        offset: u32,
    ) -> Result<LabelStoryResponse> {
        let url = endpoints::build_url(paths::LABEL_STORY);
        log::debug!("[API] get_label_story({})", label_id);
        let response: Value = self
            .signed_get(
                &url,
                "labelstory",
                &[
                    ("label_id", label_id.to_string()),
                    ("limit", limit.to_string()),
                    ("offset", offset.to_string()),
                ],
            )
            .await?
            .json()
            .await?;
        Ok(serde_json::from_value(response)?)
    }

    /// Bulk lookup a set of labels by ID (POST).
    ///
    /// Follows the same signing convention as `get_tracks_batch` (see
    /// `track/getList`): the sig covers the joined ID list as a query
    /// string key, and the JSON body carries the list itself.
    pub async fn get_label_list(&self, label_ids: &[u64]) -> Result<LabelGetListResponse> {
        let url = endpoints::build_url(paths::LABEL_GET_LIST);
        let headers = self.api_headers().await?;
        let timestamp = get_timestamp();
        let secret = self.secret().await?;
        let ids_str: String = label_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let sig = sign_request(
            "labelgetList",
            &[("label_ids", &ids_str)],
            timestamp,
            &secret,
        );

        let body = serde_json::json!({ "label_ids": label_ids });
        log::debug!("[API] get_label_list POST ({} ids)", label_ids.len());

        let response: Value = self
            .http()?
            .post(&url)
            .headers(headers)
            .query(&[("request_ts", timestamp.to_string()), ("request_sig", sig)])
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        Ok(serde_json::from_value(response)?)
    }
}
