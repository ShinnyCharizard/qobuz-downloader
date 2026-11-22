//! API data types retained from QBZ.
mod discover;
pub mod lenient;
mod media;
mod metadata;
pub mod mixtape;
pub mod purchase_serde;
pub mod types;
pub use lenient::{parse_items_array, parse_items_lenient};
pub use types::*;
#[cfg(test)]
mod tests;
