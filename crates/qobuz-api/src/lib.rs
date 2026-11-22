//! Locally maintained API client, derived from QBZ.
pub mod client;
pub mod cmaf;
pub mod endpoints;
pub mod error;
pub mod forbidden_breaker;
pub mod link_resolver;
pub mod lyrics;
pub mod net_diag;
pub mod offline_gate;
pub mod performers;
pub mod purchases;
pub mod retry;
pub mod signing;

// Re-export main types
pub use client::QobuzClient;
pub use cmaf::{
    decrypt_segments_into, download_full as cmaf_download_full,
    download_full_with_progress as cmaf_download_full_with_progress,
    download_raw as cmaf_download_raw,
    download_raw_with_progress as cmaf_download_raw_with_progress,
    setup_streaming as cmaf_setup_streaming, CmafProgressCallback, CmafProgressUpdate,
    CmafRawBundle, CmafStreamingInfo, CMAF_PREFETCH_CONCURRENCY,
};
pub use error::{ApiError, Result};
pub use link_resolver::{resolve_link, LinkResolverError, ResolvedLink};
pub use lyrics::{
    merge_translation_into, QobuzLyricsContent, QobuzLyricsDocument, QobuzLyricsLine,
    QobuzLyricsPlainLine, QobuzLyricsPublisher, QobuzLyricsTranslation, QobuzLyricsUrls,
    QobuzLyricsWord,
};
mod client_artist_page;
mod client_catalog;
mod client_cmaf_api;
mod client_detail;
mod client_headers;
mod client_labels;
mod client_library;
mod client_search;
mod client_streaming;
#[cfg(test)]
mod client_tests;

pub mod reporting;
