use serde::{Deserialize, Serialize};

/// Whether the app may look at the photo library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Every photo and video.
    Full,
    /// Only the ones the person picked (iOS's limited access, Android 14's
    /// "select photos").
    Limited,
    /// Refused; only the system's Settings can change it now.
    Denied,
    /// Not asked yet.
    Ask,
}

/// Which photos and videos to count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    All,
    Videos,
    Screenshots,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub kind: Kind,
    /// Only what is older than this many days; zero means everything.
    pub older_than_days: u32,
}

/// How many, and how many bytes.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct Tally {
    pub count: u64,
    pub bytes: u64,
}

/// One year of the library.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Year {
    pub year: i32,
    pub count: u64,
    pub bytes: u64,
}

/// The library as the filter sees it. Favourites are counted apart from the
/// years, because by default they stay on the phone.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Summary {
    pub years: Vec<Year>,
    pub favourites: Tally,
}
