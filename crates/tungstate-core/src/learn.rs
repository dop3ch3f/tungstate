//! Reading a folder's own shape back out of it.
//!
//! Everything else in this crate goes one way: rules in, plan out. This goes
//! the other way — files in, rules out — so that somebody who has already
//! organised a folder by hand does not have to describe it again in a
//! language they have just met. What comes out is a `policy.toml` they can
//! read, keep, and edit.
//!
//! Two policies come out, not one: the shape the folder **already has**, and
//! the same shape with every suggestion applied. Being able to read both and
//! pick is the point — a suggestion nobody can see is just an opinion.
//!
//! I/O-free like the rest of core: this reads a [`Snapshot`] somebody else
//! gathered.

use std::collections::{BTreeMap, BTreeSet};

use jiff::tz::TimeZone;

use crate::attrs::Attributes;
use crate::snapshot::Snapshot;

/// At or above this percentage in one directory, a level is not sorting
/// anything — everything lands in the same place.
///
/// Whole percentages rather than a float: these are ratios of counted files,
/// and integer arithmetic keeps them exact.
const LOPSIDED_PERCENT: usize = 95;

/// A directory holding more than this is hard to look through, which is the
/// whole reason somebody would want it split.
const CROWDED: usize = 150;

/// Splitting past this many files a directory wants a second level, not one.
const VERY_CROWDED: usize = 1000;

/// A level whose directories hold this few files each is costing a click per
/// file and saving nothing.
const THIN: usize = 2;

/// Fewer leaves than this and "most of them are nearly empty" is not evidence.
const ENOUGH_LEAVES: usize = 8;

/// Enough of the files have to agree before a level means what it looks like:
/// four in five.
const AGREE_OF: (usize, usize) = (4, 5);

/// The policy to survey a folder with when it has none yet.
///
/// Learning a shape needs the same attributes the rules will: the media type
/// to tell a Photo directory from a directory called Photo, and a date to tell
/// a year from a project named `2019`. A survey's cost comes from the policy
/// it is given, so there has to be *a* policy before there is one — this is
/// it, and it asks for `meta` because that is what those two questions cost.
pub const PROBE: &str = "\
[folder]
name = \"probe\"
ignore = [\".DS_Store\", \"Thumbs.db\", \"*.part\", \"*.crdownload\"]

[defaults]
cooldown = \"0s\"

[[rule]]
name = \"probe\"
path = \"{date:%Y}\"
match = { mime = \"*\" }
vars.date = { from = [\"exif.DateTimeOriginal\", \"mtime\"] }
";

/// What one level of a folder's structure turned out to mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Level {
    /// Four-digit years that agree with the files' own dates.
    Year,
    /// Two-digit months, sitting under a year.
    Month,
    /// A word for what a file *is* — Photo, Video — agreeing with its type.
    Kind(Vec<String>),
    /// The file's extension.
    Ext,
    /// A size band.
    Size,
    /// Names nothing about a file can predict: an app, a client, a project.
    /// These become literals in the path and a glob in the match, which is the
    /// only shape of theirs that survives its own move.
    Named(Vec<String>),
}

impl Level {
    /// How this level is written in a rule's `path`.
    fn template(&self, value: Option<&str>) -> String {
        match self {
            Self::Year => "{date:%Y}".to_string(),
            Self::Month => "{date:%m}".to_string(),
            Self::Ext => "{ext}".to_string(),
            Self::Size => "{band}".to_string(),
            Self::Kind(_) | Self::Named(_) => value.unwrap_or("?").to_string(),
        }
    }

    /// A word for the level, for a sentence somebody reads.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Self::Year => "year",
            Self::Month => "month",
            Self::Kind(_) => "kind",
            Self::Ext => "extension",
            Self::Size => "size",
            Self::Named(_) => "name",
        }
    }
}

/// Something that would make the shape easier to live in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// What to do, in plain words.
    pub headline: String,
    /// The counts that justify it. Never an opinion on its own.
    pub why: String,
}

/// A folder's shape, and what could be better about it.
#[derive(Debug, Clone)]
pub struct Learned {
    /// The levels, outermost first. Empty when the folder has no shape.
    pub levels: Vec<Level>,
    /// Files the shape accounts for.
    pub explains: usize,
    /// Files looked at.
    pub of: usize,
    /// Files sitting loose at the root, which no level explains.
    pub loose: usize,
    /// Rules that keep the shape the folder already has.
    pub as_is: String,
    /// The same rules with every suggestion applied, when there are any.
    pub improved: Option<String>,
    /// What would be better, and by how much.
    pub suggestions: Vec<Suggestion>,
}

impl Learned {
    /// Whether anything recognisable was found.
    #[must_use]
    pub fn found_a_shape(&self) -> bool {
        !self.levels.is_empty()
    }
}

/// What a single path segment looks like it means, for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Guess {
    Year,
    Month,
    Kind(String),
    Ext,
    Size,
    Named(String),
}

/// A filename convention that names the app that wrote the file.
///
/// The name is the one handle on a file that does **not** change when the file
/// moves, so a rule keyed on it settles trivially and — unlike a directory —
/// works on a file sitting loose at the top of the folder.
///
/// Deliberately a short table of distinctive markers. `-WA0001` is the
/// convention `WhatsApp` uses and means nothing else; that is a different kind of claim
/// from reading a *date* out of a filename, which has a long tail of wrong
/// answers and is not done here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Convention {
    /// What to call the directory.
    pub app: &'static str,
    /// Matched against the file's name.
    pub pattern: &'static str,
}

/// The conventions this recognises.
pub const CONVENTIONS: &[Convention] = &[
    Convention {
        app: "WhatsApp",
        pattern: r"-WA[0-9]+\.",
    },
    Convention {
        app: "Telegram",
        pattern: r"^(photo|video)_[0-9]{4}-[0-9]{2}-[0-9]{2}",
    },
    Convention {
        app: "Screenshots",
        pattern: r"^(Screenshot|Screen Shot|Bildschirmfoto)[ _-]",
    },
];

/// The kind words this recognises, and the media type each implies.
const KINDS: &[(&str, &str)] = &[
    ("photo", "image"),
    ("photos", "image"),
    ("image", "image"),
    ("images", "image"),
    ("picture", "image"),
    ("pictures", "image"),
    ("video", "video"),
    ("videos", "video"),
    ("movie", "video"),
    ("movies", "video"),
    ("audio", "audio"),
    ("music", "audio"),
];

/// The year a file believes it has, in UTC.
///
/// UTC rather than the policy's zone on purpose: this is only ever asked
/// whether a directory *named* `2024` plausibly holds 2024's files, and a
/// zone argument either side of midnight on New Year cannot change that
/// answer for a whole directory.
fn year_of(file: &Attributes) -> Option<i16> {
    file.exif
        .get("DateTimeOriginal")
        .and_then(|raw| raw.get(0..4))
        .and_then(|y| y.parse::<i16>().ok())
        .or_else(|| file.mtime.map(|t| t.to_zoned(TimeZone::UTC).year()))
}

/// The media type's first word — `image` out of `image/jpeg`.
fn class_of(file: &Attributes) -> Option<&str> {
    file.mime.as_deref().and_then(|m| m.split('/').next())
}

/// A file's extension, lowercased, without the dot.
fn ext_of(file: &Attributes) -> String {
    file.name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether a segment reads as a size band this tool would have written.
fn looks_like_a_band(segment: &str) -> bool {
    let s = segment.to_ascii_lowercase();
    ["under-", "over-", "up-to-", "from-", "exactly-"]
        .iter()
        .any(|p| s.starts_with(p))
        || (s.contains('-')
            && s.split('-').all(|part| {
                !part.is_empty() && part.chars().next().is_some_and(|c| c.is_ascii_digit())
            }))
}

/// What this segment looks like it means, for this file.
fn guess(segment: &str, position: usize, previous: Option<&Guess>, file: &Attributes) -> Guess {
    // A year has to agree with the file, or `2019` is just a project name.
    if segment.len() == 4
        && segment.chars().all(|c| c.is_ascii_digit())
        && let Ok(n) = segment.parse::<i16>()
        && (1900..=2100).contains(&n)
        && year_of(file) == Some(n)
    {
        return Guess::Year;
    }
    if position > 0
        && matches!(previous, Some(Guess::Year))
        && segment.len() == 2
        && segment.chars().all(|c| c.is_ascii_digit())
        && matches!(segment.parse::<u8>(), Ok(1..=12))
    {
        return Guess::Month;
    }
    if !ext_of(file).is_empty() && segment.eq_ignore_ascii_case(&ext_of(file)) {
        return Guess::Ext;
    }
    let lower = segment.to_ascii_lowercase();
    if let Some((_, class)) = KINDS.iter().find(|(word, _)| *word == lower)
        && class_of(file) == Some(class)
    {
        return Guess::Kind(segment.to_string());
    }
    if looks_like_a_band(segment) {
        return Guess::Size;
    }
    Guess::Named(segment.to_string())
}

/// Which conventions the files nobody has filed match, and how many each.
///
/// Only the unhomed ones: a file already sitting in the shape is filed, and
/// re-filing it by its name would be this tool second-guessing the person.
fn conventions_among(files: &[&Attributes]) -> Vec<(Convention, usize, bool, bool)> {
    let mut out = Vec::new();
    for convention in CONVENTIONS {
        let Ok(re) = regex::Regex::new(convention.pattern) else {
            continue;
        };
        let hits: Vec<&&Attributes> = files.iter().filter(|f| re.is_match(&f.name)).collect();
        if hits.is_empty() {
            continue;
        }
        let images = hits.iter().any(|f| class_of(f) == Some("image"));
        let videos = hits.iter().any(|f| class_of(f) == Some("video"));
        out.push((*convention, hits.len(), images, videos));
    }
    out
}

/// Every file the shape should be read from: real files, below the root.
fn files(snapshot: &Snapshot) -> impl Iterator<Item = &Attributes> {
    snapshot
        .entries
        .iter()
        .filter(|e| !e.is_dir && !e.name.starts_with('.'))
}

/// Read a folder's shape, and say what would improve it.
///
/// Reads nothing; the snapshot has already been gathered.
#[must_use]
pub fn learn(snapshot: &Snapshot, folder_name: &str) -> Learned {
    let all: Vec<&Attributes> = files(snapshot).collect();
    let of = all.len();
    let loose = all.iter().filter(|f| f.parent.is_empty()).count();

    // The shape is read from the depth most files sit at. A handful of files
    // one level shallower is a stray, not a second structure.
    let mut by_depth: BTreeMap<usize, Vec<&Attributes>> = BTreeMap::new();
    for file in all.iter().filter(|f| !f.parent.is_empty()) {
        by_depth
            .entry(file.parent.split('/').count())
            .or_default()
            .push(file);
    }
    let Some((&depth, deepest)) = by_depth.iter().max_by_key(|(_, v)| v.len()) else {
        return nothing_found(folder_name, &all, of, loose);
    };

    // One column per level, holding what each file thought that segment meant.
    let mut columns: Vec<Vec<Guess>> = vec![Vec::new(); depth];
    for file in deepest {
        let segments: Vec<&str> = file.parent.split('/').collect();
        let mut previous: Option<Guess> = None;
        for (i, segment) in segments.iter().enumerate() {
            let g = guess(segment, i, previous.as_ref(), file);
            columns[i].push(g.clone());
            previous = Some(g);
        }
    }

    let levels: Vec<Level> = columns.iter().map(|c| settle(c, deepest.len())).collect();
    let explains = deepest.len();
    let combos = combinations(&levels, deepest);

    // Files the shape does not account for: loose at the top, or at some other
    // depth. These are the ones a filename convention can still find a home for.
    let homed: BTreeSet<String> = deepest.iter().map(|f| f.relative_path()).collect();
    let unhomed: Vec<&Attributes> = all
        .iter()
        .copied()
        .filter(|f| !homed.contains(&f.relative_path()))
        .collect();
    let found = conventions_among(&unhomed);

    let suggestions = advise(&levels, deepest, loose, of, &found);
    let as_is = write_policy(folder_name, &levels, &combos, &[]);
    let improved = improve(&levels, &combos, &suggestions).map_or_else(
        || (!found.is_empty()).then(|| write_policy(folder_name, &levels, &combos, &found)),
        |(levels, combos)| Some(write_policy(folder_name, &levels, &combos, &found)),
    );

    Learned {
        levels,
        explains,
        of,
        loose,
        as_is,
        improved,
        suggestions,
    }
}

/// What a column of guesses agrees on.
fn settle(column: &[Guess], total: usize) -> Level {
    let mut tally: BTreeMap<&'static str, usize> = BTreeMap::new();
    for g in column {
        let key = match g {
            Guess::Year => "year",
            Guess::Month => "month",
            Guess::Kind(_) => "kind",
            Guess::Ext => "ext",
            Guess::Size => "size",
            Guess::Named(_) => "named",
        };
        *tally.entry(key).or_default() += 1;
    }
    let winner = tally.iter().max_by_key(|(_, n)| **n).map(|(k, _)| *k);
    let agreed = winner.map_or(0, |w| tally[w]);
    // Anything the files do not agree on is a name. A level half of whose
    // directories are years and half of which are words is not a date level
    // being untidy, it is somebody's own vocabulary.
    if total == 0 || agreed * AGREE_OF.1 < total * AGREE_OF.0 {
        return Level::Named(named_values(column));
    }
    match winner {
        Some("year") => Level::Year,
        Some("month") => Level::Month,
        Some("ext") => Level::Ext,
        Some("size") => Level::Size,
        Some("kind") => Level::Kind(distinct(column.iter().filter_map(|g| match g {
            Guess::Kind(w) => Some(w.clone()),
            _ => None,
        }))),
        _ => Level::Named(named_values(column)),
    }
}

fn named_values(column: &[Guess]) -> Vec<String> {
    distinct(column.iter().map(|g| match g {
        Guess::Named(n) => n.clone(),
        Guess::Kind(w) => w.clone(),
        Guess::Year => "year".to_string(),
        Guess::Month => "month".to_string(),
        Guess::Ext => "ext".to_string(),
        Guess::Size => "size".to_string(),
    }))
}

fn distinct(values: impl Iterator<Item = String>) -> Vec<String> {
    values.collect::<BTreeSet<_>>().into_iter().collect()
}

/// A folder with no directories at all.
///
/// Still not a dead end: the filenames may say where their files belong even
/// when nothing else does, which is exactly the folder-of-downloads case.
fn nothing_found(folder_name: &str, all: &[&Attributes], of: usize, loose: usize) -> Learned {
    let found = conventions_among(all);
    let mut suggestions = vec![Suggestion {
        headline: "There is no shape here yet to keep.".to_string(),
        why: format!("All {of} file(s) sit at the top of the folder, in no directory."),
    }];
    for (convention, count, _, _) in &found {
        suggestions.push(Suggestion {
            headline: format!("File the {} files by their names.", convention.app),
            why: format!(
                "{count} unfiled file(s) are named the way {} names them (`{}`), which says \
                 where they belong without any directory to read.",
                convention.app, convention.pattern
            ),
        });
    }
    Learned {
        levels: Vec::new(),
        explains: 0,
        of,
        loose,
        as_is: write_policy(folder_name, &[], &[], &[]),
        improved: (!found.is_empty()).then(|| write_policy(folder_name, &[], &[], &found)),
        suggestions,
    }
}

/// How many files sit in each leaf directory.
fn leaf_sizes(files: &[&Attributes]) -> Vec<usize> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for f in files {
        *counts.entry(f.parent.as_str()).or_default() += 1;
    }
    counts.into_values().collect()
}

/// What would make this shape easier to live in, and by how much.
fn advise(
    levels: &[Level],
    files: &[&Attributes],
    loose: usize,
    of: usize,
    found: &[(Convention, usize, bool, bool)],
) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let leaves = leaf_sizes(files);
    let biggest = leaves.iter().copied().max().unwrap_or(0);

    // Crowded, and a date nobody is using yet.
    let dated = files.iter().filter(|f| year_of(f).is_some()).count();
    let has_year = levels.contains(&Level::Year);
    if biggest > CROWDED && !has_year && dated * 10 >= files.len() * 9 {
        let years = distinct(
            files
                .iter()
                .filter_map(|f| year_of(f).map(|y| y.to_string())),
        );
        out.push(Suggestion {
            headline: "Split by year.".to_string(),
            why: format!(
                "The fullest directory holds {biggest} files, and {dated} of {} have a date \
                 spanning {} year(s) — about {} per year.",
                files.len(),
                years.len(),
                files.len() / years.len().max(1)
            ),
        });
        if biggest > VERY_CROWDED {
            out.push(Suggestion {
                headline: "And then by month.".to_string(),
                why: format!(
                    "{biggest} files in one directory is still a long list after a year split."
                ),
            });
        }
    }

    // A level everything lands in the same side of.
    for (i, level) in levels.iter().enumerate() {
        let values: Vec<String> = match level {
            Level::Named(v) | Level::Kind(v) => v.clone(),
            _ => continue,
        };
        if values.len() == 1 && levels.len() > 1 {
            out.push(Suggestion {
                headline: format!("Drop the `{}` level.", values[0]),
                why: format!(
                    "Every one of {} files is under `{}`, so that level is a directory \
                     everything shares and nothing is sorted by.",
                    files.len(),
                    values[0]
                ),
            });
        } else if values.len() > 1 {
            let dominant = dominant_percent(files, i);
            if dominant >= LOPSIDED_PERCENT {
                out.push(Suggestion {
                    headline: format!("The {} level barely sorts anything.", level.word()),
                    why: format!(
                        "{}% of {} files land in one of its {} directories.",
                        dominant,
                        files.len(),
                        values.len()
                    ),
                });
            }
        }
    }

    // Too fine: lots of directories, almost nothing in each.
    if leaves.len() >= ENOUGH_LEAVES && levels.len() > 1 {
        let thin = leaves.iter().filter(|n| **n <= THIN).count();
        if thin * 4 >= leaves.len() * 3 {
            out.push(Suggestion {
                headline: format!(
                    "Drop the deepest level ({}).",
                    levels[levels.len() - 1].word()
                ),
                why: format!(
                    "{thin} of {} directories hold {THIN} file(s) or fewer — that level costs \
                     a click per file and groups nothing.",
                    leaves.len()
                ),
            });
        }
    }

    if loose > 0 {
        out.push(Suggestion {
            headline: "Give the loose files a home.".to_string(),
            why: format!(
                "{loose} of {of} file(s) sit at the top of the folder, outside the shape."
            ),
        });
    }
    // What the filenames themselves say. This is the only advice here that can
    // home a file with no directory to read.
    for (convention, count, _, _) in found {
        out.push(Suggestion {
            headline: format!("File the {} files by their names.", convention.app),
            why: format!(
                "{count} unfiled file(s) are named the way {} names them (`{}`), which says \
                 where they belong without any directory to read.",
                convention.app, convention.pattern
            ),
        });
    }
    out
}

/// The percentage of files landing in the commonest directory at one level.
fn dominant_percent(files: &[&Attributes], level: usize) -> usize {
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    for f in files {
        if let Some(segment) = f.parent.split('/').nth(level) {
            *tally.entry(segment).or_default() += 1;
        }
    }
    let top = tally.values().copied().max().unwrap_or(0);
    if files.is_empty() {
        0
    } else {
        top * 100 / files.len()
    }
}

/// One observed branch: the literal at each level, or `None` where the level
/// is derived from the file rather than written down.
type Combo = Vec<Option<String>>;

/// The branches that actually occur.
///
/// Not the cross product of each level's values: two name levels are a *tree*,
/// so `Work/Clients/Acme` and `Personal/Photos/Wedding` are two branches and
/// `Work/Photos/Acme` is not a third. Taking the product invented directories
/// nobody has, and wrote `Personal/Personal/Personal`.
fn combinations(levels: &[Level], files: &[&Attributes]) -> Vec<Combo> {
    let mut out: BTreeSet<Combo> = BTreeSet::new();
    for file in files {
        let segments: Vec<&str> = file.parent.split('/').collect();
        if segments.len() != levels.len() {
            continue;
        }
        out.insert(
            levels
                .iter()
                .zip(&segments)
                .map(|(level, segment)| match level {
                    Level::Named(_) | Level::Kind(_) => Some((*segment).to_string()),
                    _ => None,
                })
                .collect(),
        );
    }
    if out.is_empty() {
        out.insert(vec![None; levels.len()]);
    }
    out.into_iter().collect()
}

/// The shape with every suggestion applied, when any of them change it.
fn improve(
    levels: &[Level],
    combos: &[Combo],
    suggestions: &[Suggestion],
) -> Option<(Vec<Level>, Vec<Combo>)> {
    let mut out = levels.to_vec();
    let mut combos = combos.to_vec();
    let mut changed = false;
    // Levels and branches move together: dropping a level drops its column
    // from every branch, and adding one adds an empty column.
    let insert = |out: &mut Vec<Level>, combos: &mut Vec<Combo>, at: usize, level: Level| {
        out.insert(at, level);
        for c in combos.iter_mut() {
            c.insert(at, None);
        }
    };
    let drop = |out: &mut Vec<Level>, combos: &mut Vec<Combo>, at: usize| {
        out.remove(at);
        for c in combos.iter_mut() {
            c.remove(at);
        }
    };

    for s in suggestions {
        if s.headline == "Split by year." && !out.contains(&Level::Year) {
            insert(&mut out, &mut combos, 0, Level::Year);
            changed = true;
        } else if s.headline == "And then by month." && !out.contains(&Level::Month) {
            let at = out
                .iter()
                .position(|l| *l == Level::Year)
                .map_or(0, |i| i + 1);
            insert(&mut out, &mut combos, at, Level::Month);
            changed = true;
        } else if s.headline.starts_with("Drop the deepest level") && out.len() > 1 {
            let at = out.len() - 1;
            drop(&mut out, &mut combos, at);
            changed = true;
        } else if let Some(name) = s
            .headline
            .strip_prefix("Drop the `")
            .and_then(|r| r.strip_suffix("` level."))
            && let Some(i) = out.iter().position(
                |l| matches!(l, Level::Named(v) | Level::Kind(v) if v.len() == 1 && v[0] == name),
            )
            && out.len() > 1
        {
            drop(&mut out, &mut combos, i);
            changed = true;
        }
    }
    if changed {
        // Dropping a level can make two branches identical.
        combos.sort();
        combos.dedup();
    }
    changed.then_some((out, combos))
}

/// The size bands a generated policy uses.
///
/// Written as intervals, not labels: `Interval::label` turns them into
/// `under-100MB` and friends, which are legal directory names on Windows and
/// over SMB where `<100MB` is not (slice 5).
const BANDS: &[&str] = &["<100MB", "100MB-500MB", "500MB-1GB", ">1GB"];

/// A name safe to use as a rule's name.
fn slug(text: &str) -> String {
    let mut out: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

/// The media type a kind word implies.
fn class_for(word: &str) -> Option<&'static str> {
    let lower = word.to_ascii_lowercase();
    KINDS
        .iter()
        .find(|(w, _)| *w == lower)
        .map(|(_, class)| *class)
}

/// The globs that pin a branch to its names, at the depth the shape describes.
///
/// A name — an app, a client — cannot be recovered from a file's own
/// attributes, so it has to be matched on where the file *is*.
///
/// **`*` and not `**`.** `**` crosses directories, and a rule whose path is
/// `Work/Clients/Acme` matching `Work/Clients/Acme/**` claims
/// `Work/Clients/Acme/2023/invoice.pdf` and files it one level up — flattening
/// the `2023` directory, out of the policy whose whole promise is keeping the
/// shape you already have. A file deeper than the shape is outside the shape,
/// and outside the shape means left alone.
///
/// Two arms, because the rule has to match the file both where it sits now and
/// where the rule will put it; without the second it stops matching its own
/// output. They collapse to one when the names *are* the whole path.
fn globs_for(levels: &[Level], combo: &Combo) -> Option<Vec<String>> {
    let named: Vec<String> = levels
        .iter()
        .zip(combo)
        .filter_map(|(level, value)| match (level, value) {
            (Level::Named(_), Some(v)) => Some(v.clone()),
            _ => None,
        })
        .collect();
    if named.is_empty() {
        return None;
    }

    // Where this rule will put it: one segment per level — the literal where
    // the level is a name, `*` where it is read off the file — and one more
    // for the filename. Exactly as deep as the shape, so a stray directory
    // below it does not match.
    let mut after: Vec<String> = levels
        .iter()
        .zip(combo)
        .map(|(level, value)| match (level, value) {
            (Level::Named(_) | Level::Kind(_), Some(v)) => v.clone(),
            _ => "*".to_string(),
        })
        .collect();
    after.push("*".to_string());

    // Where a file may be arriving: loose inside the named directories.
    let before = format!("{}/*", named.join("/"));
    let after = after.join("/");

    let mut globs = vec![before];
    if !globs.contains(&after) {
        globs.push(after);
    }
    Some(globs)
}

/// Write the rules for a shape, as a policy somebody can read and edit.
///
/// One rule per branch that actually exists, per kind.
fn write_policy(
    folder_name: &str,
    levels: &[Level],
    combos: &[Combo],
    conventions: &[(Convention, usize, bool, bool)],
) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    out.push_str("# Written by `tungstate folder learn`, from the shape this folder\n");
    out.push_str("# already had. Read it, change it, keep it — it is a normal policy\n");
    out.push_str("# and nothing here will touch it again.\n\n");
    out.push_str("[folder]\n");
    let _ = writeln!(out, "name = {:?}", slug(folder_name));
    // Deliberately no `inbox`. An inbox sweeps every file no rule matched,
    // and these rules only describe the shape that was found — on a folder
    // where that shape explains a third of the files, an inbox would move the
    // other two thirds. "Keep what you have" has to mean it. The count of
    // unhomed files is reported as a suggestion instead, where the person can
    // decide.
    out.push_str("ignore = [\".DS_Store\", \"Thumbs.db\", \"*.part\", \"*.crdownload\"]\n\n");
    out.push_str("[defaults]\n");
    out.push_str("cooldown = \"30s\"\n");
    out.push_str("on_conflict = \"rename\"\n");

    if levels.is_empty() && conventions.is_empty() {
        out.push_str(
            "\n# No shape was found to keep: every file sits at the top of the\n\
             # folder. Add a rule below, or start from one of the layouts in\n\
             # `tungstate init --template`.\n",
        );
        return out;
    }

    // With nothing to keep but names that say where files belong, the names
    // supply the shape. Year and kind are still read off the file itself.
    let borrowed;
    let levels = if levels.is_empty() {
        borrowed = vec![
            Level::Year,
            Level::Named(Vec::new()),
            Level::Kind(Vec::new()),
            Level::Ext,
        ];
        &borrowed[..]
    } else {
        levels
    };

    let wants_date = levels
        .iter()
        .any(|l| matches!(l, Level::Year | Level::Month));
    let wants_band = levels.contains(&Level::Size);
    let mut used: BTreeSet<String> = BTreeSet::new();

    for combo in combos {
        let path: Vec<String> = levels
            .iter()
            .zip(combo)
            .map(|(level, value)| level.template(value.as_deref()))
            .collect();

        // The names pin the branch; the kind is matched on the file's type.
        let kind = levels
            .iter()
            .zip(combo)
            .find_map(|(level, value)| match (level, value) {
                (Level::Kind(_), Some(v)) => Some(v.clone()),
                _ => None,
            });

        let mut clauses: Vec<String> = Vec::new();
        if let Some(class) = kind.as_deref().and_then(class_for) {
            clauses.push(format!("mime = \"{class}/*\""));
        }
        if let Some(globs) = globs_for(levels, combo) {
            clauses.push(format!(
                "glob = [{}]",
                globs
                    .iter()
                    .map(|g| format!("{g:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if clauses.is_empty() {
            clauses.push("glob = [\"**\"]".to_string());
        }

        let stem = combo
            .iter()
            .flatten()
            .map(|v| slug(v))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let stem = if stem.is_empty() {
            "everything".to_string()
        } else {
            stem
        };
        let mut rule = stem.clone();
        let mut n = 2;
        while !used.insert(rule.clone()) {
            rule = format!("{stem}-{n}");
            n += 1;
        }

        out.push_str("\n[[rule]]\n");
        let _ = writeln!(out, "name = {rule:?}");
        let _ = writeln!(out, "path = {:?}", path.join("/"));
        let _ = writeln!(out, "match = {{ {} }}", clauses.join(", "));
        if wants_date {
            out.push_str("vars.date = { from = [\"exif.DateTimeOriginal\", \"mtime\"] }\n");
        }
        if wants_band {
            let bands = BANDS
                .iter()
                .map(|b| format!("{b:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "vars.band = {{ from = \"size\", bucket = [{bands}] }}");
        }
    }
    write_conventions(&mut out, levels, conventions, &mut used);
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use jiff::Timestamp;

    use super::*;
    use crate::attrs::Tier;
    use crate::policy::Policy;

    /// A file at a path, with the type and date a shape is read from.
    fn file(path: &str, mime: &str, year: i16, size: u64) -> Attributes {
        let mut a = Attributes::new(path, size, Timestamp::UNIX_EPOCH);
        a.mime = Some(mime.to_string());
        a.mtime = Some(
            format!("{year}-06-01T12:00:00Z")
                .parse::<Timestamp>()
                .expect("a timestamp"),
        );
        a
    }

    fn folder(entries: Vec<Attributes>) -> Snapshot {
        let mut dirs = BTreeSet::new();
        for e in &entries {
            for ancestor in crate::snapshot::ancestors(&e.relative_path()) {
                dirs.insert(ancestor);
            }
        }
        Snapshot::new(Timestamp::UNIX_EPOCH, entries, dirs, true)
    }

    /// The shape the user actually keeps: year / app / type / extension / size.
    fn yardstick() -> Snapshot {
        folder(vec![
            file(
                "2024/WhatsApp/Photo/jpg/under-100MB/a.jpg",
                "image/jpeg",
                2024,
                10,
            ),
            file(
                "2024/WhatsApp/Photo/jpg/under-100MB/b.jpg",
                "image/jpeg",
                2024,
                20,
            ),
            file(
                "2024/WhatsApp/Video/mp4/under-100MB/c.mp4",
                "video/mp4",
                2024,
                30,
            ),
            file(
                "2025/Telegram/Photo/jpg/under-100MB/d.jpg",
                "image/jpeg",
                2025,
                40,
            ),
            file(
                "2025/Telegram/Video/mp4/under-100MB/e.mp4",
                "video/mp4",
                2025,
                50,
            ),
        ])
    }

    #[test]
    fn the_probe_policy_parses_and_asks_for_what_learning_needs() {
        let loaded = Policy::parse(PROBE).expect("the probe policy parses");
        assert_eq!(
            loaded.policy.required_tier(),
            Tier::Meta,
            "learning needs the media type and the date, and nothing dearer"
        );
    }

    #[test]
    fn a_five_level_shape_is_read_back_level_by_level() {
        let learned = learn(&yardstick(), "media");
        assert_eq!(learned.levels.len(), 5, "{:?}", learned.levels);
        assert_eq!(learned.levels[0], Level::Year);
        assert_eq!(
            learned.levels[1],
            Level::Named(vec!["Telegram".into(), "WhatsApp".into()])
        );
        assert_eq!(
            learned.levels[2],
            Level::Kind(vec!["Photo".into(), "Video".into()])
        );
        assert_eq!(learned.levels[3], Level::Ext);
        assert_eq!(learned.levels[4], Level::Size);
    }

    #[test]
    fn the_rules_it_writes_are_a_policy_that_parses() {
        let learned = learn(&yardstick(), "media");
        Policy::parse(&learned.as_is).expect("the written policy parses");
    }

    /// The whole promise of "maintain it": read the shape, write the rules,
    /// and the folder is already where those rules would put it.
    #[test]
    fn a_learned_policy_leaves_the_folder_it_was_learned_from_alone() {
        let snap = yardstick();
        let learned = learn(&snap, "media");
        let policy = Policy::parse(&learned.as_is).expect("parses").policy;
        let plan = policy.plan(&snap);

        assert!(plan.settles, "a learned policy must converge");
        let moves: Vec<_> = plan
            .ops
            .iter()
            .filter(|op| matches!(op, crate::plan::Op::Move { .. }))
            .collect();
        assert!(
            moves.is_empty(),
            "learned rules moved files they had just been read from: {moves:#?}"
        );
    }

    /// The same rules have to hold for a *new* file arriving in an app folder,
    /// or "maintain the shape" only means "do nothing".
    #[test]
    fn a_new_arrival_is_filed_into_the_shape_that_was_learned() {
        let learned = learn(&yardstick(), "media");
        let policy = Policy::parse(&learned.as_is).expect("parses").policy;

        let arriving = folder(vec![file("WhatsApp/new.jpg", "image/jpeg", 2024, 10)]);
        let moved: Vec<String> = policy
            .plan(&arriving)
            .ops
            .iter()
            .filter_map(|op| match op {
                crate::plan::Op::Move { to, .. } => Some(to.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(moved, vec!["2024/WhatsApp/Photo/jpg/under-100MB/new.jpg"]);
    }

    /// Two name levels are a tree, not a cross product. Taking the product
    /// wrote `path = "Personal/Personal/Personal"` for a folder that has
    /// `Work/Clients/Acme` and `Personal/Photos/Wedding` in it.
    #[test]
    fn nested_names_follow_the_branches_that_exist_and_invent_none() {
        let snap = folder(vec![
            file("Work/Clients/Acme/a.pdf", "application/pdf", 2024, 10),
            file("Work/Clients/Globex/b.pdf", "application/pdf", 2024, 10),
            file("Personal/Photos/Wedding/c.jpg", "image/jpeg", 2024, 10),
        ]);
        let learned = learn(&snap, "f");
        Policy::parse(&learned.as_is).expect("parses");

        for branch in [
            "Work/Clients/Acme",
            "Work/Clients/Globex",
            "Personal/Photos/Wedding",
        ] {
            assert!(
                learned.as_is.contains(&format!("path = {branch:?}")),
                "missing the branch {branch}:\n{}",
                learned.as_is
            );
        }
        assert_eq!(
            learned.as_is.matches("[[rule]]").count(),
            3,
            "one rule per branch that exists, and no others:\n{}",
            learned.as_is
        );
        assert!(
            !learned.as_is.contains("Personal/Personal"),
            "the cross-product bug is back:\n{}",
            learned.as_is
        );
    }

    /// A file deeper than the learned shape is outside the shape, and outside
    /// the shape means left alone. The first version globbed `Acme/**`, which
    /// claimed `Acme/2023/invoice.pdf` and filed it one level up -- flattening
    /// the `2023` directory, out of the policy whose whole promise is keeping
    /// what you already have.
    #[test]
    fn a_file_deeper_than_the_shape_is_left_alone_and_not_flattened_into_it() {
        let snap = folder(vec![
            file("Work/Clients/Acme/notes.pdf", "application/pdf", 2024, 10),
            file("Work/Clients/Globex/deal.pdf", "application/pdf", 2024, 10),
            file("Personal/Photos/Wedding/a.jpg", "image/jpeg", 2024, 10),
            // One level deeper than the shape the other three describe.
            file(
                "Work/Clients/Acme/2023/invoice.pdf",
                "application/pdf",
                2023,
                10,
            ),
        ]);
        let learned = learn(&snap, "f");
        let policy = Policy::parse(&learned.as_is).expect("parses").policy;
        let plan = policy.plan(&snap);

        assert!(plan.settles);
        let moves: Vec<String> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                crate::plan::Op::Move { from, .. } => Some(from.clone()),
                _ => None,
            })
            .collect();
        assert!(
            moves.is_empty(),
            "a learned policy flattened a directory it was supposed to keep: {moves:#?}"
        );
    }

    #[test]
    fn a_directory_named_for_a_year_its_files_do_not_have_is_a_name() {
        let snap = folder(vec![
            file("2019/Photo/a.jpg", "image/jpeg", 2024, 10),
            file("2019/Photo/b.jpg", "image/jpeg", 2025, 10),
        ]);
        let learned = learn(&snap, "f");
        assert_eq!(learned.levels[0], Level::Named(vec!["2019".into()]));
    }

    #[test]
    fn a_crowded_undated_folder_is_told_to_split_by_year_with_the_numbers() {
        let mut entries = Vec::new();
        for i in 0..200_i16 {
            entries.push(file(
                &format!("Photo/f{i}.jpg"),
                "image/jpeg",
                2020 + i % 4,
                10,
            ));
        }
        let learned = learn(&folder(entries), "f");
        let split = learned
            .suggestions
            .iter()
            .find(|s| s.headline == "Split by year.")
            .expect("a crowded dated folder should be offered a year split");
        assert!(split.why.contains("200"), "{}", split.why);
        assert!(split.why.contains("4 year(s)"), "{}", split.why);

        let improved = learned.improved.expect("the improved rules exist");
        assert!(improved.contains("{date:%Y}"), "{improved}");
        Policy::parse(&improved).expect("the improved policy parses too");
    }

    #[test]
    fn a_level_everything_shares_is_offered_for_removal() {
        let snap = folder(vec![
            file("Archive/Photo/a.jpg", "image/jpeg", 2024, 10),
            file("Archive/Video/b.mp4", "video/mp4", 2024, 10),
        ]);
        let learned = learn(&snap, "f");
        assert!(
            learned
                .suggestions
                .iter()
                .any(|s| s.headline == "Drop the `Archive` level."),
            "{:?}",
            learned.suggestions
        );
    }

    #[test]
    fn loose_files_are_counted_rather_than_quietly_ignored() {
        let snap = folder(vec![
            file(
                "2024/WhatsApp/Photo/jpg/under-100MB/a.jpg",
                "image/jpeg",
                2024,
                10,
            ),
            file("stray.jpg", "image/jpeg", 2024, 10),
            file("another.pdf", "application/pdf", 2024, 10),
        ]);
        let learned = learn(&snap, "f");
        assert_eq!(learned.loose, 2);
        assert!(
            learned
                .suggestions
                .iter()
                .any(|s| s.headline == "Give the loose files a home." && s.why.contains("2 of 3")),
            "{:?}",
            learned.suggestions
        );
    }

    /// The name is the one handle that does not change when the file moves,
    /// and it works on a file with no directory to read at all.
    #[test]
    fn loose_files_are_recognised_by_the_names_their_apps_gave_them() {
        let snap = folder(vec![
            file("IMG-20240312-WA0001.jpg", "image/jpeg", 2024, 10),
            file("IMG-20240513-WA0002.jpg", "image/jpeg", 2024, 10),
            file("VID-20240722-WA0003.mp4", "video/mp4", 2024, 10),
            file("photo_2025-01-04_18-22-11.jpg", "image/jpeg", 2025, 10),
        ]);
        let learned = learn(&snap, "downloads");

        assert!(
            learned.suggestions.iter().any(|s| {
                s.headline == "File the WhatsApp files by their names."
                    && s.why.contains("3 unfiled")
            }),
            "{:?}",
            learned.suggestions
        );

        // `as_is` keeps what is there, which is nothing -- the offer is in the
        // improved rules, where somebody has to read it first.
        assert!(!learned.as_is.contains("WhatsApp"), "{}", learned.as_is);

        let improved = learned
            .improved
            .expect("names alone are a shape worth offering");
        let policy = Policy::parse(&improved).expect("parses").policy;
        let plan = policy.plan(&snap);
        assert!(plan.settles, "a rule keyed on a name must settle");

        let moved: BTreeSet<String> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                crate::plan::Op::Move { to, .. } => Some(to.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            moved,
            [
                "2024/WhatsApp/Photo/jpg/IMG-20240312-WA0001.jpg",
                "2024/WhatsApp/Photo/jpg/IMG-20240513-WA0002.jpg",
                "2024/WhatsApp/Video/mp4/VID-20240722-WA0003.mp4",
                "2025/Telegram/Photo/jpg/photo_2025-01-04_18-22-11.jpg",
            ]
            .into_iter()
            .map(String::from)
            .collect::<BTreeSet<_>>()
        );
    }

    /// A file already sitting in the shape is filed. Re-filing it by its name
    /// would be the tool second-guessing the person who put it there.
    #[test]
    fn a_file_already_in_the_shape_is_not_refiled_by_its_name() {
        let snap = folder(vec![
            file(
                "2024/WhatsApp/Photo/jpg/IMG-20240312-WA0001.jpg",
                "image/jpeg",
                2024,
                10,
            ),
            file(
                "2024/WhatsApp/Photo/jpg/IMG-20240513-WA0002.jpg",
                "image/jpeg",
                2024,
                10,
            ),
        ]);
        let learned = learn(&snap, "media");
        assert!(
            !learned
                .suggestions
                .iter()
                .any(|s| s.headline.starts_with("File the WhatsApp")),
            "{:?}",
            learned.suggestions
        );
    }

    #[test]
    fn a_folder_with_no_shape_says_so_instead_of_inventing_one() {
        let snap = folder(vec![
            file("a.jpg", "image/jpeg", 2024, 10),
            file("b.pdf", "application/pdf", 2024, 10),
        ]);
        let learned = learn(&snap, "f");
        assert!(!learned.found_a_shape());
        Policy::parse(&learned.as_is).expect("even the empty policy parses");
    }
}

#[cfg(test)]
mod keeping_tests {
    use std::collections::BTreeSet;

    use jiff::Timestamp;

    use super::*;
    use crate::policy::Policy;

    fn file(path: &str, mime: &str, year: i16) -> Attributes {
        let mut a = Attributes::new(path, 10, Timestamp::UNIX_EPOCH);
        a.mime = Some(mime.to_string());
        a.mtime = Some(
            format!("{year}-06-01T12:00:00Z")
                .parse::<Timestamp>()
                .expect("a timestamp"),
        );
        a
    }

    fn folder(entries: Vec<Attributes>) -> Snapshot {
        let mut dirs = BTreeSet::new();
        for e in &entries {
            for ancestor in crate::snapshot::ancestors(&e.relative_path()) {
                dirs.insert(ancestor);
            }
        }
        Snapshot::new(Timestamp::UNIX_EPOCH, entries, dirs, true)
    }

    /// "Keep the shape it already has" has to mean it. An `inbox` would sweep
    /// every file the learned shape does not explain — on a half-organised
    /// folder that is most of them.
    #[test]
    fn files_the_shape_does_not_explain_are_left_where_they_are() {
        let snap = folder(vec![
            file("Work/Clients/Acme/a.pdf", "application/pdf", 2024),
            file("Work/Clients/Globex/b.pdf", "application/pdf", 2024),
            file("loose.txt", "text/plain", 2024),
            file("Somewhere/else/entirely/deep.txt", "text/plain", 2024),
        ]);
        let learned = learn(&snap, "f");
        let policy = Policy::parse(&learned.as_is).expect("parses").policy;
        let plan = policy.plan(&snap);

        assert!(plan.settles);
        let moves: Vec<_> = plan
            .ops
            .iter()
            .filter(|op| matches!(op, crate::plan::Op::Move { .. }))
            .collect();
        assert!(
            moves.is_empty(),
            "a learned policy moved files it never claimed to understand: {moves:#?}"
        );
        assert!(
            !learned.as_is.contains("inbox"),
            "a learned policy must not sweep what it did not explain"
        );
    }
}

/// Rules that file by filename convention, into the shape that was learned.
///
/// These go in the *improved* policy only. `as_is` keeps what is there; moving
/// files that are merely lying about untidily is a change, and a change is
/// something to be offered rather than assumed.
fn write_conventions(
    out: &mut String,
    levels: &[Level],
    conventions: &[(Convention, usize, bool, bool)],
    used: &mut BTreeSet<String>,
) {
    use std::fmt::Write as _;

    let wants_date = levels
        .iter()
        .any(|l| matches!(l, Level::Year | Level::Month));
    let wants_band = levels.contains(&Level::Size);

    for (convention, _, images, videos) in conventions {
        // One rule per kind the convention actually produced, so a `Photo`
        // directory is never created for an app that only sent videos.
        let kinds: Vec<Option<&str>> = match (*images, *videos) {
            (true, true) => vec![Some("Photo"), Some("Video")],
            (true, false) => vec![Some("Photo")],
            (false, true) => vec![Some("Video")],
            (false, false) => vec![None],
        };
        for kind in kinds {
            let path: Vec<String> = levels
                .iter()
                .map(|level| match level {
                    Level::Named(_) => convention.app.to_string(),
                    Level::Kind(_) => kind.unwrap_or("Other").to_string(),
                    other => other.template(None),
                })
                .collect();

            let mut clauses = vec![format!("regex = {:?}", convention.pattern)];
            match kind {
                Some("Photo") => clauses.push("mime = \"image/*\"".to_string()),
                Some("Video") => clauses.push("mime = \"video/*\"".to_string()),
                _ => {}
            }

            let stem = slug(&format!(
                "{}-{}",
                convention.app,
                kind.unwrap_or("all").to_ascii_lowercase()
            ));
            let mut rule = stem.clone();
            let mut n = 2;
            while !used.insert(rule.clone()) {
                rule = format!("{stem}-{n}");
                n += 1;
            }

            let _ = writeln!(out, "\n[[rule]]");
            let _ = writeln!(out, "name = {rule:?}");
            let _ = writeln!(out, "path = {:?}", path.join("/"));
            let _ = writeln!(out, "match = {{ {} }}", clauses.join(", "));
            if wants_date {
                let _ = writeln!(
                    out,
                    "vars.date = {{ from = [\"exif.DateTimeOriginal\", \"mtime\"] }}"
                );
            }
            if wants_band {
                let bands = BANDS
                    .iter()
                    .map(|b| format!("{b:?}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "vars.band = {{ from = \"size\", bucket = [{bands}] }}");
            }
        }
    }
}
