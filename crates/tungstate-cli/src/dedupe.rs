//! `tungstate dedupe`: which files are the same file, and what to do about it.
//!
//! The pass itself is pure and lives in `tungstate_core::dupes`. This is the
//! conversation around it: which folder, what to do with the extra copies,
//! and whether anything is allowed to happen at all.
//!
//! Nothing here decides for the person. Without `--extras`, and without an
//! answer remembered from last time, it asks.

use std::collections::BTreeSet;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use tungstate_backend::Backend as _;
use tungstate_backend::local::LocalBackend;
use tungstate_core::dupes::{self, Extras, Found, Kept, Wants};
use tungstate_execute::digest::Cached;
use tungstate_journal::Journal;

use crate::folder::{load, locate};

/// Where the answer to "set aside or trash?" is kept, so it is asked once.
const REMEMBERED: &str = "dedupe.extras";

/// Confirming a sampled group reads both copies. Past this much, over a
/// network, it asks first.
const ASK_PAST: u64 = 1024 * 1024 * 1024;

/// What was asked for on the command line.
///
/// One struct rather than seven arguments, and the flags stay flags: they are
/// what `clap` parsed, and inventing an enum per pair would only move the
/// booleans somewhere else.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
pub struct Asked {
    /// Deal with the extra copies rather than only reporting them.
    pub apply: bool,
    /// What happens to them, if the person has already said.
    pub extras: Option<Extras>,
    /// Copies to keep whatever the tie-break says.
    pub keep: Vec<String>,
    /// Only group files, never whole folders.
    pub files_only: bool,
    /// Emit what was found as JSON and stop.
    pub json: bool,
    /// Do not stop to ask anything.
    pub yes: bool,
}

/// `tungstate dedupe [PATH] [--apply] [--extras ...] [--keep PATH] [--json]`.
pub fn dedupe(target: Option<&str>, asked: &Asked) -> ExitCode {
    let target = target.unwrap_or(".");
    let located = match locate(Path::new(target), None) {
        Ok(located) => located,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let Ok((_, loaded)) = load(&located) else {
        return ExitCode::FAILURE;
    };
    let journal = match crate::open_journal() {
        Ok(journal) => journal,
        Err(error) => return crate::fail(&error),
    };

    let backend = LocalBackend::new(located.root.clone());
    let root = located.root.to_string_lossy().to_string();
    let snapshot = match tungstate_attrs::survey(&backend, &loaded.policy) {
        Ok(snapshot) => snapshot,
        Err(error) => return crate::fail(&error),
    };

    // A file already where the rules would put it wins the tie-break, so the
    // copy that stays is the one the folder's own shape agrees with.
    let plan = loaded.policy.plan(&snapshot);
    let moving: BTreeSet<&str> = plan
        .ops
        .iter()
        .filter_map(tungstate_core::Op::source)
        .collect();
    // On a share, reading a file in full means pulling it across the network,
    // so the pass samples instead and says which groups are unconfirmed.
    let networked = backend.capabilities().networked;
    let wants = Wants {
        sampled: networked,
        pinned: asked.keep.iter().cloned().collect(),
        settled: snapshot
            .entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(tungstate_core::attrs::Attributes::relative_path)
            .filter(|path| !moving.contains(path.as_str()))
            .collect(),
        files_only: asked.files_only,
    };

    let mut digest = Cached::new(&backend, &journal, &root);
    let found = match dupes::find(&snapshot, &mut digest, &wants) {
        Ok(found) => found,
        Err(trouble) => {
            eprintln!("error: {trouble}");
            return ExitCode::FAILURE;
        }
    };

    if asked.json {
        match serde_json::to_string_pretty(&found) {
            Ok(text) => println!("{text}"),
            Err(error) => return crate::fail(&error),
        }
        return ExitCode::SUCCESS;
    }

    print!(
        "{}",
        report(
            &found,
            &located.root.display().to_string(),
            digest.reads(),
            digest.hits(),
            digest.recorded()
        )
    );
    if found.is_empty() || !asked.apply {
        if !found.is_empty() {
            println!("Nothing has been changed. Add --apply to deal with the extra copies.");
        }
        return ExitCode::SUCCESS;
    }

    // Samples are enough to show a group and never enough to move a file, so
    // the ones about to be acted on are confirmed in full first.
    let found = match settle(&found, &snapshot, &backend, &journal, &root, asked) {
        Ok(found) => found,
        Err(code) => return code,
    };
    if found.is_empty() {
        println!("Nothing left to do: the samples matched and the files did not.");
        return ExitCode::SUCCESS;
    }

    let Some(extras) = decide(asked, &journal) else {
        eprintln!(
            "error: say what should happen to the extra copies: --extras set-aside or --extras trash"
        );
        return ExitCode::FAILURE;
    };
    carry_out(
        &snapshot,
        &found,
        extras,
        &loaded.policy,
        &backend,
        &journal,
        &root,
    )
}

/// Confirm every unconfirmed group, and drop the ones the samples got wrong.
///
/// The expensive half of the whole feature, and it happens once, here, for
/// the groups about to be acted on rather than for the folder.
fn settle(
    found: &Found,
    snapshot: &tungstate_core::Snapshot,
    backend: &LocalBackend,
    journal: &Journal,
    root: &str,
    asked: &Asked,
) -> Result<Found, ExitCode> {
    let unsure = found.unsure();
    if unsure == 0 {
        return Ok(found.clone());
    }

    let to_read: u64 = found
        .groups
        .iter()
        .filter(|group| !group.sure)
        .map(|group| group.size * (group.extras.len() as u64 + 1))
        .chain(
            found
                .folders
                .iter()
                .filter(|group| !group.sure)
                .map(|group| group.bytes * (group.extras.len() as u64 + 1)),
        )
        .sum();
    if to_read > ASK_PAST && !asked.yes && !agreed(to_read) {
        eprintln!("Stopped. Nothing has been changed.");
        return Err(ExitCode::SUCCESS);
    }

    println!();
    println!("Confirming {unsure} group(s) in full before anything moves…");
    let mut digest = Cached::new(backend, journal, root);
    let mut settled = found.clone();
    let mut dropped = 0;
    let mut groups = Vec::new();
    for group in &found.groups {
        match dupes::confirm(group, &mut digest) {
            Ok(Some(confirmed)) => groups.push(confirmed),
            Ok(None) => dropped += 1,
            Err(trouble) => {
                eprintln!("error: {trouble}");
                return Err(ExitCode::FAILURE);
            }
        }
    }
    let mut folders = Vec::new();
    for group in &found.folders {
        match dupes::confirm_folder(group, snapshot, &mut digest) {
            Ok(Some(confirmed)) => folders.push(confirmed),
            Ok(None) => dropped += 1,
            Err(trouble) => {
                eprintln!("error: {trouble}");
                return Err(ExitCode::FAILURE);
            }
        }
    }
    if dropped > 0 {
        println!("{dropped} group(s) were not the same after all, and are left alone.");
    }
    settled.groups = groups;
    settled.folders = folders;
    Ok(settled)
}

/// Ask before pulling a lot of data across a network.
fn agreed(to_read: u64) -> bool {
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "error: confirming these would read {} across the network. Re-run with --yes to allow it.",
            bytes(to_read)
        );
        return false;
    }
    println!();
    println!(
        "Confirming these reads {} across the network, because samples are not proof.",
        bytes(to_read)
    );
    print!("Go ahead? [y/N]: ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes")
}

/// What happens to the extra copies: what was asked for, what was answered
/// last time, or whatever the person says now.
fn decide(asked: &Asked, journal: &Journal) -> Option<Extras> {
    if let Some(chosen) = asked.extras.or_else(|| remembered(journal)) {
        return Some(chosen);
    }
    let chosen = ask(asked.yes)?;
    remember(journal, chosen);
    Some(chosen)
}

/// Build the plan and carry it out.
fn carry_out(
    snapshot: &tungstate_core::Snapshot,
    found: &Found,
    extras: Extras,
    policy: &tungstate_core::Policy,
    backend: &LocalBackend,
    journal: &Journal,
    root: &str,
) -> ExitCode {
    let plan = dupes::plan(
        snapshot,
        found,
        extras,
        &policy.folder.name,
        policy.folder.mode,
    );
    match tungstate_execute::apply(&plan, snapshot, backend, journal, root) {
        Ok(applied) => {
            println!();
            println!(
                "{} file(s) {}. {} reclaimed.",
                found.extra_files(),
                match extras {
                    Extras::SetAside => "set aside",
                    Extras::Trash => "sent to the trash",
                },
                bytes(found.reclaimable())
            );
            if extras.reversible() {
                println!(
                    "Put it all back with `tungstate undo --plan {}`.",
                    applied.plan.0
                );
            } else {
                println!("These are in your Trash. Only the Finder can put them back.");
            }
            for failure in &applied.failed {
                eprintln!("  {}: {}", failure.path, failure.why);
            }
            ExitCode::SUCCESS
        }
        Err(error) => crate::fail(&error),
    }
}

/// What was found, in the order somebody clearing space wants it.
fn report(found: &Found, root: &str, reads: usize, hits: usize, recorded: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "folder at {root}");
    if found.is_empty() {
        let _ = writeln!(
            out,
            "  no duplicates. Every file here is the only copy of it."
        );
        return out;
    }

    if !found.folders.is_empty() {
        let _ = writeln!(out, "\nfolders copied whole");
        for group in &found.folders {
            let _ = writeln!(
                out,
                "  {} ({} file(s), {}){}",
                group.keep,
                group.files,
                bytes(group.bytes),
                if group.sure {
                    ""
                } else {
                    "  [almost certainly the same; confirmed before anything moves]"
                }
            );
            for extra in &group.extras {
                let _ = writeln!(out, "    also at {extra}");
            }
        }
    }

    if !found.groups.is_empty() {
        let _ = writeln!(out, "\nthe same file, more than once");
        for group in &found.groups {
            let _ = writeln!(
                out,
                "  {} ({}, kept: {}){}",
                group.keep,
                bytes(group.size),
                why(group.why),
                if group.sure {
                    ""
                } else {
                    "  [almost certainly the same; confirmed before anything moves]"
                }
            );
            for copy in &group.extras {
                let _ = writeln!(out, "    also at {}", copy.path);
            }
        }
    }

    if !found.linked.is_empty() {
        let _ = writeln!(out, "\ntwo names for one file");
        for linked in &found.linked {
            let _ = writeln!(
                out,
                "  {} ({})",
                linked.names.join("  =  "),
                bytes(linked.size)
            );
        }
        let _ = writeln!(
            out,
            "  These are hard links, not copies: dealing with one frees nothing."
        );
    }

    let _ = writeln!(
        out,
        "\n{} would come back, from {} extra file(s).",
        bytes(found.reclaimable()),
        found.extra_files()
    );
    let _ = writeln!(
        out,
        "{reads} file(s) read, {hits} taken from what was remembered last time."
    );
    if recorded > 0 {
        let _ = writeln!(
            out,
            "{recorded} needed no reading at all: tungstate moved them here and recorded what they hold."
        );
    }
    out
}

/// The tie-break, in the words a person would use.
fn why(kept: Kept) -> &'static str {
    match kept {
        Kept::Pinned => "you said so",
        Kept::Settled => "it is where the rules put it",
        Kept::Oldest => "it is the oldest",
        Kept::Path => "it is the plainest path",
    }
}

/// Ask what should happen to the extra copies, once.
fn ask(yes: bool) -> Option<Extras> {
    if yes {
        // `--yes` means "do not stop to ask", and the answer that cannot lose
        // anything is the only one this may assume.
        return Some(Extras::SetAside);
    }
    if !std::io::stdin().is_terminal() {
        return None;
    }
    println!();
    println!("What should happen to the extra copies?");
    println!("  1  set them aside inside the folder  (undoable with `tungstate undo`)");
    println!("  2  send them to the Trash            (only the Finder can put those back)");
    print!("Choose 1 or 2: ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return None;
    }
    match answer.trim() {
        "1" => Some(Extras::SetAside),
        "2" => Some(Extras::Trash),
        _ => None,
    }
}

fn remembered(journal: &Journal) -> Option<Extras> {
    match journal.setting(REMEMBERED).ok().flatten()?.as_str() {
        "set-aside" => Some(Extras::SetAside),
        "trash" => Some(Extras::Trash),
        _ => None,
    }
}

fn remember(journal: &Journal, extras: Extras) {
    let value = match extras {
        Extras::SetAside => "set-aside",
        Extras::Trash => "trash",
    };
    let _ = journal.remember_setting(REMEMBERED, value);
}

/// Bytes in the units a person reads.
#[allow(clippy::cast_precision_loss)]
fn bytes(count: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    // Precision past 52 bits would be four petabytes of duplicate video, and
    // the answer is printed to one decimal place either way.
    let mut size = count as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{count} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}
