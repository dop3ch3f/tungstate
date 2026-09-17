//! The window's half of the second half: folders, rules, and tidying.
//!
//! Everything here is a thin layer over `tungstate-core`, `tungstate-attrs`
//! and `tungstate-execute`. The one thing it adds is vocabulary: on screen a
//! policy is **rules**, a plan is a **preview**, applying is **tidying up**
//! and undoing is **putting it back**. The engine keeps the precise words; the
//! window speaks to whoever is looking at it.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tungstate_backend::local::LocalBackend;
use tungstate_core::plan::{Op, Plan, Reason};
use tungstate_core::{Policy, Snapshot};
use tungstate_journal::Journal;

/// Where a policy lives, relative to the folder it governs.
pub const POLICY_RELATIVE: &str = ".tungstate/policy.toml";

/// A folder as the list shows it.
#[derive(Debug, Clone, Serialize)]
pub struct FolderView {
    /// What to call it.
    pub name: String,
    /// Where it is.
    pub root: String,
    /// Whether it has rules yet. Without them nothing happens to it at all,
    /// which is not guessable from anywhere else on the screen.
    pub has_rules: bool,
    /// Why its rules will not load, if they will not.
    pub broken: Option<String>,
}

/// A starting layout, for the picker.
#[derive(Debug, Clone, Serialize)]
pub struct LayoutView {
    /// The token that chooses it.
    pub name: String,
    /// One line.
    pub summary: String,
    /// One more line.
    pub detail: String,
}

/// One entry in the before or after tree.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TreeEntry {
    /// Root-relative path, forward slashes.
    pub path: String,
    /// True for a directory.
    pub is_dir: bool,
    /// Size in bytes. Zero for a directory.
    pub size: u64,
    /// Whether this file is one the preview moves. Marked in both trees, so
    /// the eye can follow one file from the left to the right.
    pub moves: bool,
}

/// One move, for the detail view behind the trees.
#[derive(Debug, Clone, Serialize)]
pub struct MoveView {
    /// Where it is.
    pub from: String,
    /// Where it would be.
    pub to: String,
    /// The rule that decided, in words.
    pub why: String,
}

/// A file the preview leaves alone, and the reason in plain words.
#[derive(Debug, Clone, Serialize)]
pub struct LeftAloneView {
    /// The file.
    pub path: String,
    /// Why.
    pub why: String,
}

/// Everything the Folders tab needs to draw one folder.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewView {
    /// The folder's name from its rules.
    pub folder: String,
    /// The folder as it is.
    pub before: Vec<TreeEntry>,
    /// The folder as it would be.
    pub after: Vec<TreeEntry>,
    /// The moves, for the detail view.
    pub moves: Vec<MoveView>,
    /// What stays put, and why.
    pub left_alone: Vec<LeftAloneView>,
    /// How many files would move.
    pub files: usize,
    /// How many files there are.
    pub of: usize,
    /// How many bytes would move.
    pub bytes: u64,
    /// Whether this is a big change, worth a warning before the button.
    pub large: bool,
    /// Whether the rules settle. False is a refusal, not a warning.
    pub settles: bool,
    /// Files still too recent to touch.
    pub waiting: usize,
    /// How long the longest of those has to wait.
    pub longest_wait: u64,
    /// Whether there is anything at all to do.
    pub tidy: bool,
    /// The reorganisation that can be put back, if there is one.
    pub undoable: Option<i64>,
}

/// Load a folder's rules, or say why not.
///
/// # Errors
/// The diagnostic as a string, already drawn against the line it is about.
pub fn rules_at(root: &Path) -> Result<Policy, String> {
    let path = root.join(POLICY_RELATIVE);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Policy::parse(&text)
        .map(|loaded| loaded.policy)
        .map_err(|error| describe_policy_error(&text, &error))
}

/// A policy error with the line it is on, since the window has no `miette`.
fn describe_policy_error(text: &str, error: &tungstate_core::PolicyError) -> String {
    error.span().map_or_else(
        || error.to_string(),
        |span| {
            let (line, column) = tungstate_core::line_of(text, span.start);
            format!("line {line}, column {column}: {error}")
        },
    )
}

/// Survey a folder and work out what its rules would do.
///
/// # Errors
/// A sentence, ready to put on screen.
pub fn preview(root: &Path, journal: &Journal) -> Result<PreviewView, String> {
    let policy = rules_at(root)?;
    let backend = LocalBackend::new(root.to_path_buf());
    let before = tungstate_attrs::survey(&backend, &policy).map_err(|e| e.to_string())?;
    let plan = policy.plan(&before);

    // The "after" tree is the snapshot the planner already computed on paper
    // to check that the rules settle, so this costs nothing new.
    let after = plan
        .apply_to(&before)
        .map_err(|e| format!("these rules cannot be carried out: {e}"))?;

    let moving_from: std::collections::BTreeSet<&str> =
        plan.ops.iter().filter_map(Op::source).collect();
    let moving_to: std::collections::BTreeSet<&str> =
        plan.ops.iter().filter_map(Op::target).collect();

    let undoable = journal
        .recent_plans(32)
        .ok()
        .and_then(|plans| {
            plans
                .into_iter()
                .find(|p| p.folder == root.to_string_lossy() && p.is_undoable())
        })
        .map(|p| p.id.0);

    Ok(PreviewView {
        folder: policy.folder.name.clone(),
        before: tree(&before, &moving_from),
        after: tree(&after, &moving_to),
        moves: plan.ops.iter().filter_map(describe_move).collect(),
        left_alone: plan
            .untouched
            .iter()
            .map(|u| LeftAloneView {
                path: u.file.clone(),
                why: plainly(&u.reason),
            })
            .collect(),
        files: plan.blast.files,
        of: plan.blast.of,
        bytes: plan.blast.bytes,
        large: plan.blast.over_limit,
        settles: plan.settles,
        waiting: plan.waiting(),
        longest_wait: plan.longest_wait(),
        tidy: plan.ops.is_empty(),
        undoable,
    })
}

/// A snapshot as a sorted list of entries, with the movers marked.
fn tree(snapshot: &Snapshot, marked: &std::collections::BTreeSet<&str>) -> Vec<TreeEntry> {
    let mut entries: Vec<TreeEntry> = snapshot
        .entries
        .iter()
        .filter(|e| !e.is_dir)
        .map(|entry| {
            let path = entry.relative_path();
            TreeEntry {
                moves: marked.contains(path.as_str()),
                path,
                is_dir: false,
                size: entry.size,
            }
        })
        .chain(snapshot.directories.iter().map(|path| TreeEntry {
            path: path.clone(),
            is_dir: true,
            size: 0,
            moves: false,
        }))
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

fn describe_move(op: &Op) -> Option<MoveView> {
    match op {
        Op::Move { from, to, because } => Some(MoveView {
            from: from.clone(),
            to: to.clone(),
            why: match because {
                tungstate_core::plan::Because::Rule { name } => format!("rule “{name}”"),
                tungstate_core::plan::Because::Inbox => {
                    "no rule matched, so it goes to the inbox".to_string()
                }
                tungstate_core::plan::Because::Numbered { wanted } => {
                    format!("numbered — something else wanted {wanted}")
                }
                tungstate_core::plan::Because::MakeWay => {
                    "moved aside so two files can trade places".to_string()
                }
            },
        }),
        Op::Quarantine { from, to, because } => Some(MoveView {
            from: from.clone(),
            to: to.clone(),
            why: match because {
                tungstate_core::plan::Parked::Conflict { holder } => {
                    format!("set aside — “{holder}” already has that name")
                }
                tungstate_core::plan::Parked::Replaced { by } => {
                    format!("set aside, not deleted, so “{by}” can have the name")
                }
            },
        }),
        Op::MkDir { .. } | Op::RmDir { .. } => None,
    }
}

/// A reason in the words somebody looking at a screen would use.
fn plainly(reason: &Reason) -> String {
    match reason {
        Reason::AlreadyThere => "already in the right place".to_string(),
        Reason::Ignored { because } => format!("ignored ({because})"),
        Reason::Opaque { pattern } => {
            format!("left whole — “{pattern}” is treated as one thing")
        }
        Reason::Unresolvable { rule, var, reason } => {
            format!("rule “{rule}” needs {{{var}}}, and {reason}")
        }
        Reason::Unmatched => "no rule covers it, and there is no inbox".to_string(),
        Reason::Cooling { seconds_left } => {
            format!("written too recently — {seconds_left}s to wait")
        }
        Reason::Conflicted { holder } => format!("“{holder}” already has the name it wants"),
        Reason::Blocked { by, detail } => format!("{detail} (“{by}”)"),
    }
}

/// The rules as written, for the read-only view.
///
/// # Errors
/// A sentence saying why the file cannot be read.
pub fn rules_text(root: &Path) -> Result<String, String> {
    let path = root.join(POLICY_RELATIVE);
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Whether this folder has rules yet.
#[must_use]
pub fn has_rules(root: &Path) -> bool {
    root.join(POLICY_RELATIVE).is_file()
}

/// Write a starting layout into a folder.
///
/// # Errors
/// A sentence. Never overwrites rules that already exist.
pub fn write_layout(root: &Path, layout: &str) -> Result<PathBuf, String> {
    let Some(chosen) = tungstate_core::templates::template(layout) else {
        return Err(format!("there is no starting layout called “{layout}”"));
    };
    let path = root.join(POLICY_RELATIVE);
    if path.exists() {
        return Err("this folder already has rules".to_string());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, chosen.body)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// Every starting layout, for the picker.
#[must_use]
pub fn layouts() -> Vec<LayoutView> {
    tungstate_core::TEMPLATES
        .iter()
        .map(|t| LayoutView {
            name: t.name.to_string(),
            summary: t.summary.to_string(),
            detail: t.detail.to_string(),
        })
        .collect()
}

/// How many files a finished tidy actually moved.
///
/// Deliberately not `Applied::done`: that counts *operations*, and a plan's
/// operations include the directories it creates and the ones its own moves
/// empty. Reporting it as files told somebody who had just read "5 of 6 files
/// would move" that 9 files had moved. `blast.files` is the number the preview
/// showed, so it is the number the result has to agree with.
#[must_use]
pub fn files_moved(plan: &Plan, skipped: usize, failed: usize) -> usize {
    plan.blast
        .files
        .saturating_sub(skipped)
        .saturating_sub(failed)
}

/// A folder's name for the list: its own, or the directory's.
#[must_use]
pub fn label_for(root: &Path) -> String {
    root.file_name().map_or_else(
        || root.to_string_lossy().to_string(),
        |n| n.to_string_lossy().to_string(),
    )
}

/// The plan for a folder, for the commands that act on one.
///
/// # Errors
/// A sentence.
pub fn plan_for(root: &Path) -> Result<(Policy, Snapshot, Plan), String> {
    let policy = rules_at(root)?;
    let backend = LocalBackend::new(root.to_path_buf());
    let snapshot = tungstate_attrs::survey(&backend, &policy).map_err(|e| e.to_string())?;
    let plan = policy.plan(&snapshot);
    Ok((policy, snapshot, plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder with rules and something to tidy.
    fn messy() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        std::fs::create_dir_all(root.join(".tungstate")).unwrap();
        std::fs::write(
            root.join(".tungstate/policy.toml"),
            "[folder]\nname = \"demo\"\nignore = [\".DS_Store\"]\n\
             [defaults]\ncooldown = \"0s\"\n\n\
             [[rule]]\nname = \"text\"\npath = \"Text\"\nmatch = { ext = \"txt\" }\n",
        )
        .unwrap();
        std::fs::write(root.join("a.txt"), b"one").unwrap();
        std::fs::write(root.join("keep.bin"), b"two").unwrap();
        std::fs::write(root.join(".DS_Store"), b"junk").unwrap();
        dir
    }

    fn journal() -> Journal {
        Journal::open_in_memory().expect("journal")
    }

    #[test]
    fn a_preview_carries_both_trees_and_marks_what_moves_in_each() {
        // The whole point of the screen: the eye follows one file from the
        // left to the right, so it has to be marked on both sides.
        let dir = messy();
        let view = preview(dir.path(), &journal()).expect("preview");

        let before_moving: Vec<&str> = view
            .before
            .iter()
            .filter(|e| e.moves)
            .map(|e| e.path.as_str())
            .collect();
        let after_moving: Vec<&str> = view
            .after
            .iter()
            .filter(|e| e.moves)
            .map(|e| e.path.as_str())
            .collect();
        assert_eq!(before_moving, ["a.txt"]);
        assert_eq!(after_moving, ["Text/a.txt"]);
    }

    #[test]
    fn the_after_tree_is_what_actually_happens() {
        // It is the planner's own paper model rather than a second guess, so
        // the screen cannot promise something the executor will not do.
        let dir = messy();
        let view = preview(dir.path(), &journal()).expect("preview");
        let after: Vec<&str> = view.after.iter().map(|e| e.path.as_str()).collect();
        assert!(after.contains(&"Text/a.txt"), "{after:?}");
        assert!(after.contains(&"keep.bin"));
        assert!(!after.contains(&"a.txt"), "it moved: {after:?}");
    }

    #[test]
    fn a_preview_counts_what_would_move_and_what_there_is() {
        let dir = messy();
        let view = preview(dir.path(), &journal()).expect("preview");
        assert_eq!(view.files, 1);
        assert_eq!(view.of, 3);
        assert!(!view.tidy);
        assert!(view.settles);
        assert_eq!(view.undoable, None, "nothing has been done yet");
    }

    #[test]
    fn a_tidy_folder_says_so_rather_than_showing_an_empty_list() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join(".tungstate")).unwrap();
        std::fs::write(
            dir.path().join(".tungstate/policy.toml"),
            "[folder]\nname = \"empty\"\n[defaults]\ncooldown = \"0s\"\n\n\
             [[rule]]\nname = \"text\"\npath = \"\"\nmatch = { ext = \"txt\" }\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        let view = preview(dir.path(), &journal()).expect("preview");
        assert!(view.tidy);
        assert_eq!(view.waiting, 0);
    }

    #[test]
    fn a_folder_still_settling_is_not_the_same_as_a_tidy_one() {
        // Both are an empty list of moves and very different sentences.
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join(".tungstate")).unwrap();
        std::fs::write(
            dir.path().join(".tungstate/policy.toml"),
            "[folder]\nname = \"waiting\"\n[defaults]\ncooldown = \"1h\"\n\n\
             [[rule]]\nname = \"text\"\npath = \"Text\"\nmatch = { ext = \"txt\" }\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        let view = preview(dir.path(), &journal()).expect("preview");
        assert!(view.tidy, "nothing would move");
        assert_eq!(view.waiting, 1, "but only because it is too recent");
        assert!(view.longest_wait > 0);
    }

    #[test]
    fn a_reason_is_given_in_words_somebody_would_use() {
        let dir = messy();
        let view = preview(dir.path(), &journal()).expect("preview");
        let junk = view
            .left_alone
            .iter()
            .find(|l| l.path == ".DS_Store")
            .expect("the ignored file is named rather than dropped");
        assert!(junk.why.starts_with("ignored"), "{}", junk.why);
        assert!(
            !junk.why.contains("Outcome"),
            "no engine words: {}",
            junk.why
        );
    }

    #[test]
    fn broken_rules_say_which_line() {
        // The window has no `miette`, so the line number is put into the
        // sentence rather than drawn under the source.
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join(".tungstate")).unwrap();
        std::fs::write(
            dir.path().join(".tungstate/policy.toml"),
            "[folder]\nname = \"x\"\nmode = \"aggressive\"\n",
        )
        .unwrap();
        let error = rules_at(dir.path()).expect_err("it should not load");
        assert!(error.contains("line 3"), "{error}");
    }

    #[test]
    fn every_starting_layout_reaches_the_picker_with_its_description() {
        let offered = layouts();
        assert!(offered.len() >= 4);
        for layout in &offered {
            assert!(!layout.summary.is_empty(), "{}", layout.name);
            assert!(!layout.detail.is_empty(), "{}", layout.name);
        }
    }

    #[test]
    fn a_layout_is_written_once_and_never_over() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_layout(dir.path(), "downloads").expect("written");
        assert!(has_rules(dir.path()));
        let before = std::fs::read_to_string(dir.path().join(POLICY_RELATIVE)).unwrap();

        assert!(write_layout(dir.path(), "photos").is_err(), "never over");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(POLICY_RELATIVE)).unwrap(),
            before
        );
    }

    #[test]
    fn an_unknown_layout_is_refused_in_words() {
        let dir = tempfile::tempdir().expect("temp dir");
        let error = write_layout(dir.path(), "nonsense").expect_err("refused");
        assert!(error.contains("nonsense"), "{error}");
    }

    #[test]
    fn a_folder_with_no_rules_has_none_rather_than_erroring() {
        // The list has to be able to say "none yet", which is a state rather
        // than a failure -- it is the one this whole screen exists to fix.
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(!has_rules(dir.path()));
        assert!(rules_at(dir.path()).is_err());
    }

    #[test]
    fn the_count_a_tidy_reports_is_files_and_not_operations() {
        // The window promised "N of M files would move" and then reported the
        // operation count, which also counts every directory the plan makes
        // and every one its moves empty. The two numbers must not be swapped.
        let dir = messy();
        let (_, _, plan) = plan_for(dir.path()).expect("plan");

        assert_eq!(plan.blast.files, 1, "only a.txt moves");
        assert!(
            plan.ops.len() > plan.blast.files,
            "this plan also has to create Text/, which is what made the two \
             numbers differ: {} ops vs {} files",
            plan.ops.len(),
            plan.blast.files
        );
        assert_eq!(files_moved(&plan, 0, 0), plan.blast.files);
        assert_ne!(
            files_moved(&plan, 0, 0),
            plan.ops.len(),
            "reporting operations as files is the bug"
        );
    }

    #[test]
    fn a_file_left_behind_is_not_counted_as_moved() {
        let dir = messy();
        let (_, _, plan) = plan_for(dir.path()).expect("plan");
        assert_eq!(files_moved(&plan, 1, 0), 0, "the one mover was skipped");
        assert_eq!(files_moved(&plan, 0, 1), 0, "or it failed");
        assert_eq!(files_moved(&plan, 9, 9), 0, "and it never goes negative");
    }

    #[test]
    fn a_folder_takes_its_name_from_its_directory() {
        assert_eq!(label_for(Path::new("/Users/me/Downloads")), "Downloads");
    }
}
