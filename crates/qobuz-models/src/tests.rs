use super::*;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_session_deserializes_pre_v10_json() {
        // Sessions persisted before the country/language capture must still
        // load: both new fields default to None (feature stays Auto-off).
        let json = r#"{
            "user_auth_token": "token",
            "user_id": 1705826,
            "email": "a@b.c",
            "display_name": "Tester",
            "subscription_label": "Studio",
            "subscription_valid_until": null
        }"#;
        let session: UserSession = serde_json::from_str(json).expect("old session json loads");
        assert_eq!(session.country_code, None);
        assert_eq!(session.language_code, None);
        assert_eq!(session.user_id, 1705826);
    }

    #[test]
    fn user_session_round_trips_country_and_language() {
        let json = r#"{
            "user_auth_token": "token",
            "user_id": 1705826,
            "email": "a@b.c",
            "display_name": "Tester",
            "subscription_label": "Studio",
            "subscription_valid_until": null,
            "country_code": "FR",
            "language_code": "fr"
        }"#;
        let session: UserSession = serde_json::from_str(json).expect("v10 session json loads");
        assert_eq!(session.country_code.as_deref(), Some("FR"));
        assert_eq!(session.language_code.as_deref(), Some("fr"));
        let back: UserSession =
            serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!(back.language_code.as_deref(), Some("fr"));
    }

    fn image_set() -> ImageSet {
        ImageSet {
            small: Some("s50".into()),
            thumbnail: Some("t150".into()),
            large: Some("l300".into()),
            extralarge: Some("x600".into()),
            mega: Some("m3000".into()),
            back: None,
            medium: None,
        }
    }

    #[test]
    fn for_px_picks_the_smallest_covering_variant() {
        let set = image_set();
        assert_eq!(set.for_px(72).map(String::as_str), Some("t150"));
        assert_eq!(set.for_px(150).map(String::as_str), Some("t150"));
        assert_eq!(set.for_px(151).map(String::as_str), Some("l300"));
        assert_eq!(set.for_px(300).map(String::as_str), Some("l300"));
        assert_eq!(set.for_px(660).map(String::as_str), Some("m3000"));
        // Past mega there is nothing bigger: best() is the ceiling.
        assert_eq!(set.for_px(4000).map(String::as_str), Some("m3000"));
    }

    #[test]
    fn for_px_skips_missing_variants_upward_on_partial_sets() {
        // Discover endpoints return only small/thumbnail/large.
        let discover = ImageSet {
            small: Some("s50".into()),
            thumbnail: Some("t150".into()),
            large: Some("l300".into()),
            ..ImageSet::default()
        };
        assert_eq!(discover.for_px(72).map(String::as_str), Some("t150"));
        // Nothing >= 600: best-available (large), never an upscale promise.
        assert_eq!(discover.for_px(600).map(String::as_str), Some("l300"));
        // A gap below the request keeps scanning UP: {small, mega} at 200px
        // is served by mega — the smallest variant that COVERS the slot (the
        // "never upscale into a slot" rule wins over download size; full
        // album/track sets populate all five variants, so this only bites on
        // genuinely sparse sets).
        let gapped = ImageSet {
            small: Some("s50".into()),
            mega: Some("m3000".into()),
            ..ImageSet::default()
        };
        assert_eq!(gapped.for_px(200).map(String::as_str), Some("m3000"));
        assert_eq!(gapped.for_px(72).map(String::as_str), Some("m3000"));
    }

    #[test]
    fn for_px_never_serves_worse_than_best() {
        let tiny = ImageSet {
            small: Some("s50".into()),
            ..ImageSet::default()
        };
        assert_eq!(tiny.for_px(1600).map(String::as_str), Some("s50"));
        assert_eq!(ImageSet::default().for_px(300), None);
    }

    #[test]
    fn bucket_for_px_maps_slots_onto_the_variant_table() {
        assert_eq!(ImageSet::bucket_for_px(0), IMAGE_VARIANT_PX[0]);
        assert_eq!(ImageSet::bucket_for_px(72), 150);
        assert_eq!(ImageSet::bucket_for_px(150), 150);
        assert_eq!(ImageSet::bucket_for_px(151), 300);
        assert_eq!(ImageSet::bucket_for_px(660), 3000);
        // Beyond the table the bucket saturates at mega, so a 4K window does
        // not invent a size no variant serves.
        assert_eq!(ImageSet::bucket_for_px(5000), 3000);
    }

    #[test]
    fn qobuz_cover_at_px_rewrites_the_size_suffix() {
        let small = "https://static.qobuz.com/images/covers/pb/ap/gxy13gb56appb_50.jpg";
        assert_eq!(
            qobuz_cover_at_px(small, 600).as_deref(),
            Some("https://static.qobuz.com/images/covers/pb/ap/gxy13gb56appb_600.jpg")
        );
        // Slots bucket UP to the smallest covering variant.
        assert_eq!(
            qobuz_cover_at_px(small, 250).as_deref(),
            Some("https://static.qobuz.com/images/covers/pb/ap/gxy13gb56appb_300.jpg")
        );
        assert_eq!(
            qobuz_cover_at_px(small, 5000).as_deref(),
            Some("https://static.qobuz.com/images/covers/pb/ap/gxy13gb56appb_max.jpg")
        );
        // The mega bucket emits the literal `_max` suffix, never `_3000`
        // (a 404 on the live CDN, measured 2026-08-15).
        assert_eq!(
            qobuz_cover_at_px(small, 700).as_deref(),
            Some("https://static.qobuz.com/images/covers/pb/ap/gxy13gb56appb_max.jpg")
        );
        // Down-tiering works too (a mega `_max` carried into a 150px slot).
        let mega = "https://static.qobuz.com/images/covers/pb/ap/gxy13gb56appb_max.jpg";
        assert_eq!(
            qobuz_cover_at_px(mega, 100).as_deref(),
            Some("https://static.qobuz.com/images/covers/pb/ap/gxy13gb56appb_150.jpg")
        );
    }

    #[test]
    fn qobuz_cover_at_px_leaves_unrecognized_urls_alone() {
        // Not a cover url, no size suffix, an unknown size, a query string —
        // all None, so callers keep the original.
        assert_eq!(qobuz_cover_at_px("https://example.com/a_50.jpg", 600), None);
        assert_eq!(
            qobuz_cover_at_px(
                "https://static.qobuz.com/images/covers/pb/ap/gxy13gb56appb.jpg",
                600
            ),
            None
        );
        assert_eq!(
            qobuz_cover_at_px(
                "https://static.qobuz.com/images/covers/pb/ap/gxy13gb56appb_123.jpg",
                600
            ),
            None
        );
        assert_eq!(
            qobuz_cover_at_px(
                "https://static.qobuz.com/images/covers/pb/ap/x_50.jpg?token=1",
                600
            ),
            None
        );
        assert_eq!(qobuz_cover_at_px("/home/u/Music/cover.jpg", 600), None);
    }
}

// ─── §12-4: purchase deserializers, one payload per missing optional field ────
//
// The whole point of this module is that the purchase path cannot be smoke-
// tested: Qobuz Purchases is not sold in the owner's region, so the only
// populated payloads anyone will ever see belong to end users. Every default
// below is therefore asserted rather than assumed, and the ones that DIFFER
// between two structs with the same field name get their own test, because that
// is the trap most likely to invert a screen's behaviour silently.
#[cfg(test)]
mod purchase_deserializer_tests {
    use super::*;

    // ── §2.6: the `streamable` split ─────────────────────────────────────────
    //
    // Two structs, same field name, three semantics. `PurchaseTrack.streamable`
    // is a `bool` defaulting TRUE (`serde_true`) — the purchases endpoints are
    // terse and that screen gates click-to-play on the flag, so a default of
    // false would make every purchased album unclickable. Catalog
    // `Track.streamable` is now `Option<bool>`: it does not guess in EITHER
    // direction, because the greyout and the queue filter key on it and a wrong
    // guess is visible on every screen in the app. Absence is resolved in
    // exactly one function, `Track::is_streamable`, and nowhere else.

    /// A purchases-list track defaults `streamable` to **true**.
    #[test]
    fn purchase_track_streamable_defaults_true() {
        let t: PurchaseTrack = serde_json::from_str(r#"{"id":1,"title":"T"}"#).unwrap();
        assert!(
            t.streamable,
            "PurchaseTrack.streamable defaults TRUE (serde_true)"
        );
    }

    /// A CATALOG track that does not mention `streamable` deserializes to
    /// `None`, and `None` reads as **available**.
    ///
    /// This is the regression that would otherwise grey out the whole
    /// catalogue. While the field was a `bool` with a bare `#[serde(default)]`,
    /// an endpoint that omits the key produced `false` — "pulled from Qobuz" —
    /// for every track it returned. `/album/get` does send the key (captured
    /// 2026-08-17), but `/playlist/get?extra=tracks` never has been, and that is
    /// the surface where dead tracks actually show up.
    #[test]
    fn catalog_track_streamable_absent_is_none_and_reads_available() {
        let t: Track = serde_json::from_str(r#"{"id":1,"title":"T"}"#).unwrap();
        assert_eq!(
            t.streamable, None,
            "an absent key must stay absent — never collapse into Some(false)"
        );
        assert!(
            t.is_streamable(),
            "absence is NOT a claim of unavailability: absent reads as available"
        );
    }

    /// An explicit `"streamable": false` is honoured, and it is the ONLY shape
    /// that marks a track as pulled.
    ///
    /// This is the real wire signal, captured from
    /// `/album/get?album_id=0886443985094` on 2026-08-17: track 1 carries
    /// `false` (and keeps its `isrc`, which is what makes the replacement
    /// search's ISRC short-circuit reachable) while the album around it is
    /// `true`.
    #[test]
    fn catalog_track_streamable_false_is_honoured() {
        let t: Track = serde_json::from_str(r#"{"id":1,"title":"T","streamable":false}"#).unwrap();
        assert_eq!(t.streamable, Some(false));
        assert!(
            !t.is_streamable(),
            "Some(false) is the ONLY shape that marks a track unavailable"
        );
    }

    #[test]
    fn catalog_album_streamable_absent_reads_available() {
        let album: Album = serde_json::from_str(r#"{"id":"a","title":"A"}"#).unwrap();
        assert_eq!(album.streamable, None);
        assert!(album.is_streamable());
    }

    #[test]
    fn catalog_album_streamable_false_is_honoured() {
        let album: Album =
            serde_json::from_str(r#"{"id":"a","title":"A","streamable":false}"#).unwrap();
        assert_eq!(album.streamable, Some(false));
        assert!(!album.is_streamable());
    }

    #[test]
    fn embedded_album_availability_keeps_unknown_distinct_from_false() {
        let absent: AlbumSummary = serde_json::from_str(r#"{"id":"a","title":"A"}"#).unwrap();
        assert_eq!(absent.streamable, None);
        assert!(absent.is_streamable());

        let withdrawn: AlbumSummary = serde_json::from_str(
            r#"{"id":"a","title":"A","version":"Remastered","streamable":false}"#,
        )
        .unwrap();
        assert_eq!(withdrawn.streamable, Some(false));
        assert!(!withdrawn.is_streamable());
        assert!(!withdrawn.is_upcoming());
        assert_eq!(withdrawn.version.as_deref(), Some("Remastered"));
    }

    #[test]
    fn embedded_future_album_is_upcoming_not_withdrawn() {
        let upcoming: AlbumSummary = serde_json::from_str(
            r#"{"id":"a","title":"A","streamable":false,"release_date_stream":"2999-01-01"}"#,
        )
        .unwrap();
        assert!(upcoming.is_upcoming());
    }

    /// `downloadable` drives three list behaviours (the hide-unavailable filter,
    /// the album click gate, the unavailable marker), so its default is not
    /// decoration: getting it wrong ships clickable unavailable albums.
    #[test]
    fn purchase_album_downloadable_defaults_true() {
        let a: PurchaseAlbum = serde_json::from_str(r#"{"id":"x","title":"A"}"#).unwrap();
        assert!(a.downloadable);
        assert!(
            !a.downloaded,
            "downloaded is local-only, never from the wire"
        );
        assert!(a.tracks.is_none());
        assert!(a.purchased_at.is_none());
    }

    // ── §2.5: leniency is load-bearing, and has a documented limit ────────────

    /// Wrong-typed optional fields degrade to `None` instead of failing the item.
    #[test]
    fn wrong_typed_optionals_degrade_to_none() {
        let json = r#"{
            "id": 7, "title": "T",
            "media_number": "not a number",
            "maximum_sampling_rate": {"nope": true},
            "maximum_bit_depth": [],
            "album": 12345,
            "purchased_at": "yesterday",
            "version": []
        }"#;
        let t: PurchaseTrack = serde_json::from_str(json).unwrap();
        assert_eq!(t.id, 7);
        assert_eq!(t.media_number, None);
        assert_eq!(t.maximum_sampling_rate, None);
        assert_eq!(t.maximum_bit_depth, None);
        assert!(t.album.is_none());
        assert_eq!(t.purchased_at, None);
        assert_eq!(t.version, None);
    }

    /// A malformed page collapses to an EMPTY page while its sibling still
    /// parses. This is why a 401/403 body shaped like JSON and "you own nothing"
    /// are indistinguishable — stated here so nobody later mistakes the empty
    /// state for proof that the request succeeded.
    #[test]
    fn a_malformed_page_empties_itself_without_taking_its_sibling() {
        let json = r#"{
            "albums": "this is not a page",
            "tracks": {"offset":0,"limit":50,"total":1,"items":[{"id":9,"title":"Nine"}]}
        }"#;
        let r: PurchaseResponse = serde_json::from_str(json).unwrap();
        assert!(r.albums.items.is_empty());
        assert_eq!(r.albums.total, 0);
        assert_eq!(r.tracks.items.len(), 1);
        assert_eq!(r.tracks.items[0].title, "Nine");
    }

    /// §2.5b-2, measured against a live account: `?type=albums` OMITS the
    /// `tracks` key entirely — it is not present-and-zero. A port that models a
    /// present-but-empty sibling page is modelling the wrong thing.
    #[test]
    fn typed_response_with_an_absent_sibling_page_parses() {
        let json = r#"{
            "albums": {"offset":0,"limit":500,"total":0,"items":[]},
            "user": {"id": 1, "login": "someone"}
        }"#;
        let r: PurchaseResponse = serde_json::from_str(json).unwrap();
        assert!(r.albums.items.is_empty());
        assert!(
            r.tracks.items.is_empty(),
            "absent sibling defaults, never fails"
        );
        assert_eq!(r.tracks.total, 0);
    }

    /// §2.5b-3: the IDS envelope has a DIFFERENT page shape — `{total, items}`
    /// with no `offset` and no `limit`. If those scalars were required, the
    /// lenient page wrapper would swallow the failure and hand back an empty
    /// page, and both tab counters would read 0 forever with nothing logged.
    #[test]
    fn ids_response_page_shape_preserves_total_without_offset_or_limit() {
        let json = r#"{
            "albums": {"total": 42, "items": [1,2,3]},
            "tracks": {"total": 7, "items": []},
            "user": {"id": 1, "login": "someone"}
        }"#;
        let r: PurchaseIdsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.albums.total, 42, "the tab counter comes from here");
        assert_eq!(r.tracks.total, 7);
        assert_eq!(r.albums.offset, 0);
        assert_eq!(r.albums.limit, 0);
    }

    /// The documented LIMIT of the leniency: a non-object top level still fails.
    /// Recorded so the empty-list-on-everything claim is not overstated.
    /// The documented LIMIT of the leniency — MEASURED 2026-08-16, and it is
    /// wider than the contract stated.
    ///
    /// The contract (§2.5) says "a transport error, a non-JSON body and an
    /// invalid top-level shape still FAIL". The first two hold. The third does
    /// not, for ARRAYS: serde's derived `Deserialize` accepts a struct from a
    /// JSON sequence as well as a map, and since both fields carry
    /// `#[serde(default)]` plus a lenient page wrapper, `[]` — and even
    /// `[1, 2]`, whose elements are consumed positionally and swallowed —
    /// deserialize into a perfectly valid EMPTY response.
    ///
    /// What genuinely fails is a SCALAR top level: string, number, bool, null.
    ///
    /// This does not change the contract's conclusion (a JSON-shaped 401/403
    /// body is an object, and it collapses to "you own nothing" exactly as
    /// described) but it does widen the silent-empty surface by one shape, and
    /// on a feature nobody can smoke-test the exact boundary is worth pinning
    /// down rather than approximating.
    #[test]
    fn only_scalar_top_levels_fail_arrays_collapse_to_empty() {
        for ok in ["{}", "[]", "[1,2]"] {
            let parsed = serde_json::from_str::<PurchaseResponse>(ok)
                .unwrap_or_else(|e| panic!("expected {ok} to collapse to empty, got {e}"));
            assert!(parsed.albums.items.is_empty());
            assert!(parsed.tracks.items.is_empty());
        }

        for err in ["\"nope\"", "7", "null", "true"] {
            assert!(
                serde_json::from_str::<PurchaseResponse>(err).is_err(),
                "a scalar top level must still fail: {err}"
            );
        }
    }

    // ── §10-C: the version field that closes the missing purchase-version regression ───────

    #[test]
    fn purchase_track_carries_a_version_when_the_catalog_supplies_one() {
        let t: PurchaseTrack =
            serde_json::from_str(r#"{"id":1,"title":"Song","version":"Live"}"#).unwrap();
        assert_eq!(t.version.as_deref(), Some("Live"));
    }

    // ── §14.3: goodies must never take an album down ──────────────────────────

    /// The shape of a POPULATED goodies list has never been observed — it comes
    /// back empty on every album nobody owns. So an unexpected shape has to
    /// degrade to "no goodies", not to "this album will not open". `/album/get`
    /// is parsed with the strict structs, which is exactly where that would bite.
    #[test]
    fn a_surprising_goodies_shape_does_not_fail_the_album() {
        for weird in [
            r#"{"id":"1","title":"A","goodies":"a string"}"#,
            r#"{"id":"1","title":"A","goodies":{"unexpected":"object"}}"#,
            r#"{"id":"1","title":"A","goodies":[{"totally":"different"}]}"#,
            r#"{"id":"1","title":"A","goodies":7}"#,
        ] {
            let album: Album = serde_json::from_str(weird)
                .unwrap_or_else(|e| panic!("goodies shape took the album down: {weird} → {e}"));
            assert_eq!(album.title, "A");
        }
    }

    #[test]
    fn a_well_formed_goodie_parses_and_reads_defensively() {
        let album: Album = serde_json::from_str(
            r#"{"id":"1","title":"A","goodies":[
                {"id":5,"name":"Booklet","url":"https://u/b.pdf","original_url":"https://o/b.pdf"}
            ]}"#,
        )
        .unwrap();
        let goodies = album.goodies.expect("goodies present");
        assert_eq!(goodies.len(), 1);
        assert_eq!(goodies[0].best_url(), Some("https://o/b.pdf"));
        assert_eq!(goodies[0].display_name(), "Booklet");
    }
}

#[cfg(test)]
mod upcoming_tests {
    use super::{upcoming_from, Track};
    use chrono::{TimeZone, Utc};

    #[test]
    fn future_streamable_at_wins_over_the_date() {
        let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
        assert!(upcoming_from(Some(now.timestamp() + 60), None, now));
        assert!(upcoming_from(
            Some(now.timestamp() + 60),
            Some("2000-01-01"),
            now
        ));
        assert!(!upcoming_from(
            Some(now.timestamp() - 60),
            Some("2099-01-01"),
            now
        ));
        // The captured dead track: `streamable_at: null`, past release date.
        assert!(!upcoming_from(None, Some("2013-06-21"), now));
    }

    #[test]
    fn release_date_is_the_fallback_and_absence_is_not_upcoming() {
        let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
        assert!(upcoming_from(None, Some("2026-10-16"), now));
        assert!(
            !upcoming_from(None, Some("2026-09-02"), now),
            "today is not upcoming"
        );
        assert!(!upcoming_from(None, Some("not a date"), now));
        assert!(!upcoming_from(None, None, now));
    }

    #[test]
    fn a_streamable_track_is_never_upcoming() {
        let track = Track {
            streamable: Some(true),
            release_date_stream: Some("2099-01-01".into()),
            ..Default::default()
        };
        assert!(!track.is_upcoming());
        let track = Track {
            streamable: Some(false),
            release_date_stream: Some("2099-01-01".into()),
            ..Default::default()
        };
        assert!(track.is_upcoming());
    }
}
