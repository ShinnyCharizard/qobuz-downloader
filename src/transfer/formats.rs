use qobuz_models::{PurchaseAlbum, PurchaseTrack};

pub fn formats(purchase: &PurchaseAlbum) -> Vec<u32> {
    let mut ids = purchase.downloadable_format_ids.clone();
    if ids.is_empty() {
        if purchase.hires && purchase.maximum_sampling_rate.unwrap_or(0.0) > 96.0 {
            ids.push(27);
        }
        if purchase.hires {
            ids.push(7);
        }
        ids.extend([6, 5]);
    }
    ids.sort_by_key(|id| {
        std::cmp::Reverse(match id {
            56 => 6,
            55 => 5,
            27 => 4,
            7 => 3,
            6 => 2,
            5 => 1,
            _ => 0,
        })
    });
    ids.dedup();
    ids
}

pub fn track_formats(purchase: &PurchaseTrack) -> Vec<u32> {
    if !purchase.downloadable_format_ids.is_empty() {
        let mut ids = purchase.downloadable_format_ids.clone();
        ids.sort_by_key(|id| {
            std::cmp::Reverse(match id {
                56 => 6,
                55 => 5,
                27 => 4,
                7 => 3,
                6 => 2,
                5 => 1,
                _ => 0,
            })
        });
        ids.dedup();
        return ids;
    }
    let mut ids = Vec::new();
    if purchase.hires && purchase.maximum_sampling_rate.unwrap_or(0.0) > 96.0 {
        ids.push(27);
    }
    if purchase.hires {
        ids.push(7);
    }
    ids.extend([6, 5]);
    ids
}

pub fn label(id: u32) -> String {
    match id {
        56 => "[DSF][DSD128]".into(),
        55 => "[DSF][DSD64]".into(),
        27 => "[FLAC][24-bit,192kHz]".into(),
        7 => "[FLAC][24-bit,96kHz]".into(),
        6 => "[FLAC][16-bit,44.1kHz]".into(),
        5 => "[MP3][320kbps]".into(),
        id => format!("[Format {id}]"),
    }
}
