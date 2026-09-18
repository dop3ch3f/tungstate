//! What each starting layout would do to one folder.
//!
//! The question a layout picker should answer is *what will this do to my
//! files*, and the only honest way to answer it is to plan every layout
//! against the folder in front of you.
//!
//! Pure, like the rest of core: somebody else gathers the snapshot. That is
//! what lets the command line and the window ask the same question and get the
//! same answer, rather than each computing its own and drifting.

use serde::Serialize;

use crate::plan::Op;
use crate::policy::Policy;
use crate::snapshot::Snapshot;
use crate::templates::TEMPLATES;

/// One move, for showing an example rather than only a count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Move {
    /// Where it is now, relative to the folder.
    pub from: String,
    /// Where this layout would put it.
    pub to: String,
}

/// What one layout would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    /// The layout's name, or a label for rules that are not a layout.
    pub name: String,
    /// The one-line description, empty for rules that are not a layout.
    pub summary: String,
    /// False when these rules will not load at all.
    pub loads: bool,
    /// False when the rules would keep moving the same files for ever.
    pub settles: bool,
    /// Files that would move.
    pub files: usize,
    /// Files seen.
    pub of: usize,
    /// Bytes those files add up to.
    pub bytes: u64,
    /// Directories this would create.
    pub created: usize,
    /// Directories this would remove.
    ///
    /// The number that says a layout is *replacing* a shape rather than adding
    /// one, which a count of moved files alone never shows.
    pub removed: usize,
    /// One real move, because a count does not tell you whether you would like
    /// the answer.
    pub example: Option<Move>,
}

/// Plan every starting layout against one folder.
///
/// `also` is extra rules to include ahead of the built-in ones — the folder's
/// own, so that every other row reads as a change from where it actually is
/// rather than from nothing.
///
/// One snapshot serves them all. That is safe because [`crate::classify`]
/// applies each policy's own `ignore` when it plans and not only when the
/// folder is walked, so a shared walk cannot leak one layout's ignore list
/// into another's answer.
#[must_use]
pub fn against(snapshot: &Snapshot, also: &[(String, String)]) -> Vec<Outcome> {
    let of = snapshot.entries.iter().filter(|e| !e.is_dir).count();
    let mine = also
        .iter()
        .map(|(name, body)| (name.clone(), String::new(), body.clone()));
    let built_in = TEMPLATES.iter().map(|t| {
        (
            t.name.to_string(),
            t.summary.to_string(),
            t.body.to_string(),
        )
    });

    mine.chain(built_in)
        .map(|(name, summary, body)| one(&name, &summary, &body, snapshot, of))
        .collect()
}

fn one(name: &str, summary: &str, body: &str, snapshot: &Snapshot, of: usize) -> Outcome {
    let blank = Outcome {
        name: name.to_string(),
        summary: summary.to_string(),
        loads: false,
        settles: false,
        files: 0,
        of,
        bytes: 0,
        created: 0,
        removed: 0,
        example: None,
    };
    let Ok(loaded) = Policy::parse(body) else {
        return blank;
    };
    let plan = loaded.policy.plan(snapshot);
    let example = plan.ops.iter().find_map(|op| match op {
        Op::Move { from, to, .. } | Op::Quarantine { from, to, .. } => Some(Move {
            from: from.clone(),
            to: to.clone(),
        }),
        _ => None,
    });
    Outcome {
        loads: true,
        settles: plan.settles,
        files: plan.blast.files,
        bytes: plan.blast.bytes,
        created: plan.blast.created,
        removed: plan.blast.removed,
        example,
        ..blank
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use jiff::Timestamp;

    use super::*;
    use crate::attrs::Attributes;

    fn folder(paths: &[(&str, &str)]) -> Snapshot {
        let entries: Vec<Attributes> = paths
            .iter()
            .map(|(path, mime)| {
                let mut a = Attributes::new(path, 10, Timestamp::UNIX_EPOCH);
                a.mime = Some((*mime).to_string());
                a.mtime = Some(
                    "2024-06-01T12:00:00Z"
                        .parse::<Timestamp>()
                        .expect("a timestamp"),
                );
                a
            })
            .collect();
        let mut dirs = BTreeSet::new();
        for entry in &entries {
            for ancestor in crate::snapshot::ancestors(&entry.relative_path()) {
                dirs.insert(ancestor);
            }
        }
        Snapshot::new(Timestamp::UNIX_EPOCH, entries, dirs, true)
    }

    #[test]
    fn every_built_in_layout_gets_a_row_and_they_keep_their_order() {
        let outcomes = against(&folder(&[("a.jpg", "image/jpeg")]), &[]);
        let names: Vec<&str> = outcomes.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, crate::templates::names());
        assert!(outcomes.iter().all(|o| o.loads));
    }

    #[test]
    fn the_folders_own_rules_come_first_so_the_rest_read_as_a_change_from_them() {
        let mine = (
            "(the rules you have)".to_string(),
            "[folder]\nname = \"m\"\n[defaults]\ncooldown = \"0s\"\n\n\
             [[rule]]\nname = \"pics\"\npath = \"Pics\"\nmatch = { mime = \"image/*\" }\n"
                .to_string(),
        );
        let outcomes = against(&folder(&[("a.jpg", "image/jpeg")]), &[mine]);
        assert_eq!(outcomes[0].name, "(the rules you have)");
        assert_eq!(outcomes[0].files, 1);
        assert_eq!(
            outcomes[0].example,
            Some(Move {
                from: "a.jpg".into(),
                to: "Pics/a.jpg".into()
            })
        );
    }

    /// A layout that empties directories is replacing a shape rather than
    /// adding one, and only `removed` says so.
    #[test]
    fn a_layout_that_undoes_an_existing_shape_reports_the_directories_it_removes() {
        let snap = folder(&[("Holidays/Spain/a.jpg", "image/jpeg")]);
        let outcomes = against(&snap, &[]);
        let by_type = outcomes
            .iter()
            .find(|o| o.name == "by-type")
            .expect("by-type");
        assert_eq!(by_type.files, 1);
        assert!(
            by_type.removed >= 2,
            "flattening two directories should report removing them: {by_type:?}"
        );
    }

    #[test]
    fn rules_that_will_not_load_are_a_row_rather_than_a_missing_one() {
        let broken = (
            "(the rules you have)".to_string(),
            "not toml at all".to_string(),
        );
        let outcomes = against(&folder(&[("a.jpg", "image/jpeg")]), &[broken]);
        assert!(!outcomes[0].loads);
        assert_eq!(outcomes[0].files, 0);
        // And the built-in layouts are still all there behind it.
        assert_eq!(outcomes.len(), 1 + TEMPLATES.len());
    }
}
