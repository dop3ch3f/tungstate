//! `tungstate sync`: a set of folders kept in step.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Args, Subcommand};
use serde_json::json;
use tungstate_core::sync::{MemberBlast, SyncPlan, Why};
use tungstate_journal::{
    ConflictAction, FirstCheck, Journal, NewMember, NewSync, OnRemove, Sync, SyncDirection,
    VerifyLevel, ends,
};
use tungstate_secret::SecretStore;
use tungstate_sync::{Decided, Opened};

use crate::{fail, human_bytes};

#[derive(Subcommand)]
pub enum SyncAction {
    /// Keep two or more folders in step.
    ///
    /// Say which way things move: --push (the anchor sends to every other
    /// member), --pull (the anchor receives from every other member) or --all
    /// (every member ends up with everything). Nothing is ever removed until
    /// --exact arrives in slice 9c.
    Add(AddSync),
    /// List every sync and its members.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Add a member to a sync, or take one out.
    Members {
        /// The sync.
        name: String,
        #[command(subcommand)]
        action: MemberAction,
    },
    /// Show what a run would do, per member, changing nothing.
    Preview {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Run a sync.
    Run {
        name: String,
        /// Do not stop to confirm.
        #[arg(long)]
        yes: bool,
        /// Files at once within each leg.
        #[arg(long, value_name = "N")]
        parallel: Option<usize>,
        #[arg(long)]
        json: bool,
    },
    /// Remove a sync. Every member's files stay where they are.
    Remove { name: String },
}

#[derive(Subcommand)]
pub enum MemberAction {
    /// Add a folder. It is filled on the next run.
    Add {
        /// A path, or `connection:path`.
        end: String,
        /// What to call it. Defaults to the connection's name, or the
        /// folder's own name.
        #[arg(long = "as")]
        called: Option<String>,
    },
    /// Take a member out. Its files are not touched.
    Remove {
        /// The member's name.
        member: String,
    },
}

/// Which way things move. One of the three, said out loud rather than
/// defaulted: `--all` brings the NAS's whole archive to the laptop, which is
/// not something to do by omission.
#[derive(Args)]
#[group(multiple = false)]
pub struct Way {
    /// The anchor sends; every other member ends up with what it holds.
    #[arg(long)]
    push: bool,
    /// The anchor receives; it ends up with what every other member holds.
    #[arg(long)]
    pull: bool,
    /// Every member ends up with everything.
    #[arg(long)]
    all: bool,
}

#[derive(Args)]
pub struct AddSync {
    /// What this sync is called.
    name: String,
    /// Two or more folders: a path, or `connection:path`.
    #[arg(required = true, num_args = 2.., value_name = "FOLDER")]
    ends: Vec<String>,
    #[command(flatten)]
    way: Way,
    /// Which member is the anchor for --push or --pull. Defaults to the first.
    #[arg(long)]
    anchor: Option<String>,
    /// A name per member, in the order the folders were given.
    #[arg(long = "as", value_name = "NAME")]
    names: Vec<String>,
    /// Also carry deletions, so every member is exactly the same. Slice 9c.
    #[arg(long)]
    exact: bool,
    /// What an unattended run does when members changed a file differently.
    #[arg(long, default_value = "quarantine")]
    on_conflict: String,
    /// What removing a copy means: set-aside, or delete (slice 9c).
    #[arg(long, default_value = "set-aside")]
    on_remove: String,
    /// How thoroughly each copy is checked.
    #[arg(long, default_value = "hash")]
    verify: String,
    /// Seconds a file must have been untouched before it moves.
    #[arg(long, default_value_t = 30)]
    cooldown: u64,
    /// How hard to check two copies that meet for the first time, most
    /// thorough first:
    ///
    /// full: reads both copies. Exact, but a first run over a large archive
    /// on a network takes as long as reading it.
    ///
    /// sampled: reads the start, middle and end. Minutes rather than hours;
    /// two copies differing only between the samples are taken to match.
    ///
    /// size: reads nothing. Fastest; an edit that kept the size is missed.
    ///
    /// Only ever the first meeting: after that each member is compared with
    /// its own last reading, which costs no reading at all.
    #[arg(long, default_value = "full", verbatim_doc_comment)]
    first_check: String,
}

/// `tungstate sync …`.
pub fn run(action: SyncAction, journal: &Journal, secrets: &dyn SecretStore) -> ExitCode {
    match action {
        SyncAction::Add(asked) => add(&asked, journal),
        SyncAction::List { json } => list(journal, json),
        SyncAction::Members { name, action } => members(&name, action, journal),
        SyncAction::Preview { name, json } => preview(&name, json, journal, secrets),
        SyncAction::Run {
            name,
            yes,
            parallel,
            json,
        } => go(&name, yes, parallel, json, journal, secrets),
        SyncAction::Remove { name } => match journal
            .sync_by_name(&name)
            .and_then(|sync| journal.remove_sync(sync.id))
        {
            Ok(()) => {
                println!("removed `{name}`; every member's files are where they were");
                ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },
    }
}

fn refuse(message: &str) -> ExitCode {
    eprintln!("error: {message}");
    ExitCode::from(2)
}

fn add(asked: &AddSync, journal: &Journal) -> ExitCode {
    let direction = match (asked.way.push, asked.way.pull, asked.way.all) {
        (true, _, _) => SyncDirection::Push,
        (_, true, _) => SyncDirection::Pull,
        (_, _, true) => SyncDirection::All,
        _ => return refuse("say which way things move: --push, --pull or --all"),
    };
    if asked.exact {
        return refuse("--exact arrives in slice 9c; until then a sync only ever adds");
    }
    let Some(on_remove) = OnRemove::parse(&asked.on_remove) else {
        return refuse("--on-remove must be set-aside or delete");
    };
    if on_remove == OnRemove::Delete {
        return refuse("--on-remove delete arrives in slice 9c, with --exact");
    }
    let Some(on_conflict) = ConflictAction::parse(&asked.on_conflict) else {
        return refuse("--on-conflict must be quarantine, rename or skip");
    };
    if on_conflict == ConflictAction::Replace {
        // With three members changed, "replace" cannot say whose version
        // wins; any answer would be a guess, so it is not offered.
        return refuse("--on-conflict replace has no meaning when several members changed a file");
    }
    let Some(verify) = VerifyLevel::parse(&asked.verify) else {
        return refuse("--verify must be size, hash or readback");
    };
    let Some(first_check) = FirstCheck::parse(&asked.first_check) else {
        return refuse("--first-check must be full, sampled or size");
    };
    if direction == SyncDirection::All && asked.anchor.is_some() {
        return refuse("--all has no anchor: every member sends and receives");
    }
    if asked.names.len() > asked.ends.len() {
        return refuse("more --as names than folders");
    }

    let mut members = Vec::with_capacity(asked.ends.len());
    for (index, raw) in asked.ends.iter().enumerate() {
        match member(raw, asked.names.get(index).map(String::as_str), journal) {
            Ok(member) => members.push(member),
            Err(message) => return refuse(&message),
        }
    }
    if let Err(message) = distinct(&members) {
        return refuse(&message);
    }
    let anchor = match direction {
        SyncDirection::All => None,
        _ => Some(
            asked
                .anchor
                .clone()
                .unwrap_or_else(|| members[0].name.clone()),
        ),
    };

    let created = journal.create_sync(
        &NewSync {
            name: asked.name.clone(),
            direction,
            exact: false,
            anchor,
            on_conflict,
            on_remove,
            verify,
            cooldown: Duration::from_secs(asked.cooldown),
            first_check,
        },
        &members,
    );
    match created.and_then(|_| journal.sync_by_name(&asked.name)) {
        Ok(sync) => {
            println!("created `{}`: {}", sync.name, promise(&sync));
            for member in &sync.members {
                println!("  {:<10} {}", member.name, where_is(member, journal));
            }
            println!(
                "nothing is ever removed; `tungstate sync preview {}` shows the first run",
                sync.name
            );
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

/// One member from what was typed.
fn member(raw: &str, called: Option<&str>, journal: &Journal) -> Result<NewMember, String> {
    let end = ends::parse_end(raw, None, journal).map_err(|e| e.to_string())?;
    let path = if end.connection.is_some() {
        end.path.to_string_lossy().to_string()
    } else {
        let resolved = tungstate_journal::resolve_for_lookup(&end.path);
        if !resolved.is_dir() {
            return Err(format!("`{raw}` is not a folder on this machine"));
        }
        resolved.to_string_lossy().to_string()
    };
    let name = called.map_or_else(
        || match end.connection {
            // The connection's own name: `nas:capcut` is "nas".
            Some(_) => raw.split_once(':').map_or(raw, |(c, _)| c).to_string(),
            None => Path::new(&path)
                .file_name()
                .map_or_else(|| path.clone(), |n| n.to_string_lossy().to_string()),
        },
        str::to_string,
    );
    Ok(NewMember {
        name,
        connection: end.connection,
        path,
    })
}

/// Members need names a person can tell apart, and folders that do not sit
/// inside one another: two members over one directory would fight over it.
fn distinct(members: &[NewMember]) -> Result<(), String> {
    for (i, a) in members.iter().enumerate() {
        for b in &members[i + 1..] {
            if a.name == b.name {
                return Err(format!(
                    "two members would both be called `{}`; name them with --as",
                    a.name
                ));
            }
            let (pa, pb) = (Path::new(&a.path), Path::new(&b.path));
            if a.connection == b.connection && (pa.starts_with(pb) || pb.starts_with(pa)) {
                return Err(format!(
                    "`{}` and `{}` overlap; a member cannot sit inside another",
                    a.name, b.name
                ));
            }
        }
    }
    Ok(())
}

/// What a sync promises, in words.
fn promise(sync: &Sync) -> String {
    let anchor = sync
        .anchor
        .and_then(|id| sync.members.iter().find(|m| m.id == id))
        .map_or("the anchor", |m| m.name.as_str());
    match sync.direction {
        SyncDirection::Push => format!("every member ends up with everything {anchor} holds"),
        SyncDirection::Pull => format!("{anchor} ends up with everything every member holds"),
        SyncDirection::All => "every member ends up with everything".to_string(),
    }
}

fn where_is(member: &tungstate_journal::Member, journal: &Journal) -> String {
    let end = match member.connection {
        Some(c) => tungstate_journal::Endpoint::remote(c, std::path::PathBuf::from(&member.path)),
        None => tungstate_journal::Endpoint::local(&member.path),
    };
    ends::describe(&end, journal)
}

fn list(journal: &Journal, json: bool) -> ExitCode {
    let syncs = match journal.syncs() {
        Ok(syncs) => syncs,
        Err(error) => return fail(&error),
    };
    if json {
        let document: Vec<_> = syncs
            .iter()
            .map(|sync| {
                json!({
                    "name": sync.name,
                    "direction": sync.direction.as_str(),
                    "anchor": sync.anchor.and_then(|id| sync.members.iter().find(|m| m.id == id)).map(|m| &m.name),
                    "first_check": sync.first_check.as_str(),
                    "members": sync.members.iter().map(|m| json!({
                        "name": m.name,
                        "at": where_is(m, journal),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&document).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }
    if syncs.is_empty() {
        println!("no syncs yet; `tungstate sync add` makes one");
    }
    for sync in &syncs {
        println!("{}: {}", sync.name, promise(sync));
        for member in &sync.members {
            let marked = if sync.anchor == Some(member.id) {
                " (anchor)"
            } else {
                ""
            };
            println!(
                "  {:<10} {}{marked}",
                member.name,
                where_is(member, journal)
            );
        }
    }
    ExitCode::SUCCESS
}

fn members(name: &str, action: MemberAction, journal: &Journal) -> ExitCode {
    let sync = match journal.sync_by_name(name) {
        Ok(sync) => sync,
        Err(error) => return fail(&error),
    };
    match action {
        MemberAction::Add { end, called } => {
            let new = match member(&end, called.as_deref(), journal) {
                Ok(new) => new,
                Err(message) => return refuse(&message),
            };
            let existing: Vec<NewMember> = sync
                .members
                .iter()
                .map(|m| NewMember {
                    name: m.name.clone(),
                    connection: m.connection,
                    path: m.path.clone(),
                })
                .chain(std::iter::once(new.clone()))
                .collect();
            if let Err(message) = distinct(&existing) {
                return refuse(&message);
            }
            match journal.add_member(sync.id, &new) {
                Ok(_) => {
                    println!("added `{}` to `{name}`; the next run fills it", new.name);
                    ExitCode::SUCCESS
                }
                Err(error) => fail(&error),
            }
        }
        MemberAction::Remove { member } => match journal.remove_member(sync.id, &member) {
            Ok(()) => {
                println!("took `{member}` out of `{name}`; its files are where they were");
                ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },
    }
}

/// Open and decide, which is everything a preview is.
fn decided(
    name: &str,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> Result<(Opened, Decided), ExitCode> {
    let opened = tungstate_sync::open(journal, name, secrets).map_err(|e| fail(&e))?;
    let decided = tungstate_sync::decide(&opened, journal).map_err(|e| fail(&e))?;
    Ok((opened, decided))
}

fn preview(name: &str, json: bool, journal: &Journal, secrets: &dyn SecretStore) -> ExitCode {
    let (opened, decided) = match decided(name, journal, secrets) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&document(&opened, &decided)).unwrap_or_default()
        );
    } else {
        say(&opened, &decided);
    }
    ExitCode::SUCCESS
}

fn go(
    name: &str,
    yes: bool,
    parallel: Option<usize>,
    json: bool,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> ExitCode {
    let (opened, decided) = match decided(name, journal, secrets) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    if decided.plan.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&document(&opened, &decided)).unwrap_or_default()
            );
        } else {
            say(&opened, &decided);
        }
        return ExitCode::SUCCESS;
    }
    if !json {
        say(&opened, &decided);
    }
    // Asked only where somebody is there to answer; a piped or scripted run
    // goes ahead, as `link run` does, because a sync never removes anything.
    let interactive = !yes && !json && std::io::IsTerminal::is_terminal(&std::io::stdin());
    if interactive && !confirm() {
        println!("nothing moved");
        return ExitCode::SUCCESS;
    }
    let mut progress = crate::CliProgress::new();
    match tungstate_sync::run(&opened, &decided, journal, &mut progress, parallel) {
        Ok(ran) => {
            if json {
                let legs: Vec<_> = ran
                    .legs
                    .iter()
                    .map(|leg| {
                        json!({
                            "from": leg.from,
                            "to": leg.to,
                            "copied": leg.summary.transferred,
                            "bytes": leg.summary.bytes,
                            "failed": leg.summary.failures.len(),
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({ "plan": ran.plan.map(|p| p.0), "legs": legs })
                    )
                    .unwrap_or_default()
                );
            } else {
                for leg in &ran.legs {
                    println!(
                        "{} → {}: {} copied ({}){}",
                        leg.from,
                        leg.to,
                        files(leg.summary.transferred),
                        human_bytes(leg.summary.bytes),
                        if leg.summary.failures.is_empty() {
                            String::new()
                        } else {
                            format!(
                                ", {} failed and will be tried next run",
                                leg.summary.failures.len()
                            )
                        }
                    );
                    for failure in &leg.summary.failures {
                        println!("  {}: {}", failure.path.display(), failure.reason);
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

fn confirm() -> bool {
    use std::io::Write as _;
    print!("run it? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok() && answer.trim().eq_ignore_ascii_case("y")
}

fn files(n: u64) -> String {
    format!("{n} {}", if n == 1 { "file" } else { "files" })
}

/// A preview in words, per member.
fn say(opened: &Opened, decided: &Decided) {
    let plan = &decided.plan;
    println!("{}: {}", opened.sync.name, promise(&opened.sync));
    for place in &opened.places {
        let blast = plan
            .blast
            .get(&place.member.id.0)
            .cloned()
            .unwrap_or_default();
        println!("  {}: {}", place.member.name, member_line(&blast));
        println!("    at {}", place.described);
    }
    if plan.is_empty() {
        println!("nothing to do: every member already holds what it should");
    }
    for leg in &plan.legs {
        let through = !opened_local(opened, leg.from) && !opened_local(opened, leg.to);
        println!(
            "  {} → {}: {} ({}){}",
            opened.name(leg.from),
            opened.name(leg.to),
            files(leg.paths.len() as u64),
            human_bytes(leg.bytes),
            if through {
                ", read through this machine"
            } else {
                ""
            }
        );
    }
    if decided.read > 0 {
        let mut why = Vec::new();
        if decided.sampled > 0 {
            why.push(format!("{} sampled", decided.sampled));
        }
        if decided.read_for_no_times > 0 {
            why.push(format!(
                "{} because a member keeps no modification times",
                decided.read_for_no_times
            ));
        }
        println!(
            "read {} to decide{}",
            files(decided.read as u64),
            if why.is_empty() {
                String::new()
            } else {
                format!(" ({})", why.join(", "))
            }
        );
    }
    if !plan.left_alone.is_empty() {
        println!("left alone: {}", files(plan.left_alone.len() as u64));
        for left in &plan.left_alone {
            println!("  {}: {}", left.path, reason(opened, &left.why));
        }
    }
}

fn opened_local(opened: &Opened, id: i64) -> bool {
    opened
        .places
        .iter()
        .find(|p| p.member.id.0 == id)
        .is_some_and(|p| p.member.connection.is_none() && !p.networked)
}

fn member_line(blast: &MemberBlast) -> String {
    let mut parts = vec![if blast.arriving == 0 {
        "nothing arriving".to_string()
    } else {
        format!(
            "{} arriving ({})",
            files(blast.arriving as u64),
            human_bytes(blast.arriving_bytes)
        )
    }];
    if blast.leaving > 0 {
        parts.push(format!("{} leaving", files(blast.leaving as u64)));
    }
    if blast.replacing > 0 {
        parts.push(format!(
            "{} replaced, the old version set aside",
            blast.replacing
        ));
    }
    parts.join(", ")
}

fn reason(opened: &Opened, why: &Why) -> String {
    match why {
        Why::TooRecent { member } => format!("still being written on {}", opened.name(*member)),
        Why::OnlyTheAnchorSends { member } => {
            format!("{} has it, but only the anchor sends", opened.name(*member))
        }
        Why::OnlyTheAnchorReceives { member } => {
            format!(
                "{} has it, but as the anchor it only receives",
                opened.name(*member)
            )
        }
        Why::Conflict { members } => format!(
            "changed differently on {}; conflicts are asked about in slice 9c",
            members
                .iter()
                .map(|m| opened.name(*m))
                .collect::<Vec<_>>()
                .join(" and ")
        ),
        Why::CaseClash { member, existing } => {
            format!(
                "{} already holds `{existing}`, which differs only in case",
                opened.name(*member)
            )
        }
    }
}

/// A preview as data, for `--json`.
fn document(opened: &Opened, decided: &Decided) -> serde_json::Value {
    let plan: &SyncPlan = &decided.plan;
    json!({
        "sync": opened.sync.name,
        "direction": opened.sync.direction.as_str(),
        "members": opened.places.iter().map(|p| {
            let b = plan.blast.get(&p.member.id.0).cloned().unwrap_or_default();
            json!({
                "name": p.member.name,
                "at": p.described,
                "arriving": b.arriving,
                "arriving_bytes": b.arriving_bytes,
                "leaving": b.leaving,
                "replacing": b.replacing,
                "holds": b.of,
            })
        }).collect::<Vec<_>>(),
        "legs": plan.legs.iter().map(|l| json!({
            "from": opened.name(l.from),
            "to": opened.name(l.to),
            "files": l.paths.len(),
            "bytes": l.bytes,
            "read_through_here": !opened_local(opened, l.from) && !opened_local(opened, l.to),
        })).collect::<Vec<_>>(),
        "read": decided.read,
        "sampled": decided.sampled,
        "left_alone": plan.left_alone.iter().map(|l| json!({
            "path": l.path,
            "why": reason(opened, &l.why),
        })).collect::<Vec<_>>(),
    })
}
