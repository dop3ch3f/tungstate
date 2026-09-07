//! Types shared by the tungstate daemon and every client (CLI, TUI, web).
//!
//! Everything here is a wire contract. Changing a type in this crate must keep
//! older clients working, so additions are safe and removals are not.

use serde::{Deserialize, Serialize};

/// Version of the tungstate binary, filled in by Cargo at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Identifier for a governed folder.
///
/// A newtype rather than a bare `String` so a folder id can never be passed
/// where a link id is expected. This costs nothing at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderId(pub String);

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn folder_id_round_trips_through_json(s in ".*") {
            let original = FolderId(s);
            let json = serde_json::to_string(&original).unwrap();
            let parsed: FolderId = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(original, parsed);
        }
    }
}
