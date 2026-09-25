//! `tungstate sync`: a set of folders kept in step.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Args, Subcommand};
use serde_json::json;
use tungstate_core::sync::{Asked, MemberBlast, Resolve, SyncOp, SyncPlan, Why};
use tungstate_journal::{
    ConflictAction, FirstCheck, Journal, NewMember, NewSync, OnRemove, PlanId, Purpose, Sync,
    SyncDirection, SyncSettings, VerifyLevel, ends,
};
use tungstate_secret::SecretStore;
use tungstate_sync::{Decided, Opened, Ran, Refusal};

use crate::{fail, human_bytes};

#[derive(Subcommand)]
pub enum SyncAction {
    /// Keep two or more folders in step.
    ///
    /// Say which way things move: --push (the anchor sends to every other
    /// member), --pull (the anchor receives from every other member) or --all
    /// (every member ends up with everything). Without --exact nothing is
    /// removed, and a file deleted by hand comes back; with it, deletions are
    /// carried too.
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
    /// Settle a conflict: one member's version everywhere, or every version
    /// everywhere, each named after its member.
    Resolve {
        /// The sync.
        name: String,
        /// The path, as the preview lists it.
        path: String,
        #[command(flatten)]
        choice: Choice,
        /// Do not stop to confirm.
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    /// Take a file or folder off every member, for good.
    ///
    /// The one deletion that sticks: without --exact a file deleted by hand
    /// is copied back, and this is how to say it was meant.
    Forget {
        /// The sync.
        name: String,
        /// A file or folder, as the preview lists it.
        path: String,
        /// Do not stop to confirm.
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    /// Put back what a run did, on every member.
    Undo {
        /// The sync.
        name: String,
        /// Put back the last N runs, newest first. Defaults to 1.
        #[arg(long, value_name = "N", conflicts_with = "plan")]
        last: Option<usize>,
        /// Put back one run by id, as `sync run` reported it.
        #[arg(long = "plan", value_name = "ID")]
        plan: Option<i64>,
        #[arg(long)]
        json: bool,
    },
    /// Change how a sync behaves from its next run on.
    Set(SetSync),
}

/// Whose version a resolved conflict keeps.
#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct Choice {
    /// This member's version, on every member.
    #[arg(long, value_name = "MEMBER")]
    keep: Option<String>,
    /// Every version, on every member, each named after its member.
    #[arg(long)]
    keep_both: bool,
}

#[derive(Args)]
pub struct SetSync {
    /// The sync.
    name: String,
    /// Carry deletions too, so every member is exactly the same.
    #[arg(long, conflicts_with = "not_exact")]
    exact: bool,
    /// Only ever add; a file deleted by hand comes back.
    #[arg(long)]
    not_exact: bool,
    /// quarantine, rename or skip.
    #[arg(long)]
    on_conflict: Option<String>,
    /// set-aside or delete. Delete needs --exact.
    #[arg(long)]
    on_remove: Option<String>,
    /// size, hash or readback.
    #[arg(long)]
    verify: Option<String>,
    /// Seconds a file must have been untouched before it moves.
    #[arg(long)]
    cooldown: Option<u64>,
    /// full, sampled or size.
    #[arg(long)]
    first_check: Option<String>,
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
    /// Also carry deletions, so every member is exactly the same.
    #[arg(long)]
    exact: bool,
    /// When members changed a file differently: quarantine keeps each
    /// member's own and parks the others' beside it, rename keeps every
    /// version under a name saying whose, skip leaves it for
    /// `tungstate sync resolve`.
    #[arg(long, default_value = "quarantine")]
    on_conflict: String,
    /// What removing a copy means: set-aside, which `sync undo` can put
    /// back, or delete, which nothing can. Delete needs --exact.
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
        SyncAction::Resolve {
            name,
            path,
            choice,
            yes,
            json,
        } => resolve(&name, &path, &choice, yes, json, journal, secrets),
        SyncAction::Forget {
            name,
            path,
            yes,
            json,
        } => forget(&name, &path, yes, json, journal, secrets),
        SyncAction::Undo {
            name,
            last,
            plan,
            json,
        } => undo(&name, last, plan, json, journal, secrets),
        SyncAction::Set(asked) => set(&asked, journal),
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
    let settings = match settings(
        asked.exact,
        &asked.on_conflict,
        &asked.on_remove,
        &asked.verify,
        asked.cooldown,
        &asked.first_check,
    ) {
        Ok(settings) => settings,
        Err(message) => return refuse(&message),
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
            exact: settings.exact,
            anchor,
            on_conflict: settings.on_conflict,
            on_remove: settings.on_remove,
            verify: settings.verify,
            cooldown: settings.cooldown,
            first_check: settings.first_check,
        },
        &members,
    );
    match created.and_then(|_| journal.sync_by_name(&asked.name)) {
        Ok(sync) => {
            println!("created `{}`: {}", sync.name, promise(&sync));
            for member in &sync.members {
                println!("  {:<10} {}", member.name, where_is(member, journal));
            }
            println!("{}", removal_line(&sync));
            println!("`tungstate sync preview {}` shows the first run", sync.name);
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

/// The settings a person typed, checked. Shared by `add` and `set`, so the
/// two cannot come to different conclusions about what is allowed.
fn settings(
    exact: bool,
    on_conflict: &str,
    on_remove: &str,
    verify: &str,
    cooldown: u64,
    first_check: &str,
) -> Result<SyncSettings, String> {
    let on_remove = OnRemove::parse(on_remove).ok_or("--on-remove must be set-aside or delete")?;
    if on_remove == OnRemove::Delete && !exact {
        // Without --exact nothing is removed, so the only thing `delete` could
        // ever touch is an old version being replaced, and losing that for
        // good is not what anybody asking for a superset meant.
        return Err("--on-remove delete needs --exact".into());
    }
    let on_conflict = ConflictAction::parse(on_conflict)
        .ok_or("--on-conflict must be quarantine, rename or skip")?;
    if on_conflict == ConflictAction::Replace {
        // With three members changed, "replace" cannot say whose version
        // wins; any answer would be a guess, so it is not offered.
        return Err(
            "--on-conflict replace has no meaning when several members changed a file".into(),
        );
    }
    Ok(SyncSettings {
        exact,
        on_conflict,
        on_remove,
        verify: VerifyLevel::parse(verify).ok_or("--verify must be size, hash or readback")?,
        cooldown: Duration::from_secs(cooldown),
        first_check: FirstCheck::parse(first_check)
            .ok_or("--first-check must be full, sampled or size")?,
    })
}

/// What a sync does about removing anything, in one sentence.
fn removal_line(sync: &Sync) -> String {
    match (sync.exact, sync.on_remove) {
        (false, _) => "nothing is removed: a file deleted on one member is copied back, \
                       and a version replaced by a newer one is set aside"
            .to_string(),
        (true, OnRemove::SetAside) => "exact: a deletion on one member reaches every member, \
                                       and what is removed is set aside where \
                                       `tungstate sync undo` can put it back"
            .to_string(),
        (true, OnRemove::Delete) => "exact: a deletion on one member reaches every member, \
                                     and what is removed is deleted outright; \
                                     a run that deletes cannot be undone"
            .to_string(),
    }
}

fn set(asked: &SetSync, journal: &Journal) -> ExitCode {
    let sync = match journal.sync_by_name(&asked.name) {
        Ok(sync) => sync,
        Err(error) => return fail(&error),
    };
    let exact = if asked.exact {
        true
    } else if asked.not_exact {
        false
    } else {
        sync.exact
    };
    let cooldown = asked.cooldown.unwrap_or(sync.cooldown.as_secs());
    let on_remove = asked
        .on_remove
        .clone()
        .unwrap_or_else(|| sync.on_remove.as_str().to_string());
    let checked = settings(
        exact,
        asked
            .on_conflict
            .as_deref()
            .unwrap_or(sync.on_conflict.as_str()),
        &on_remove,
        asked.verify.as_deref().unwrap_or(sync.verify.as_str()),
        cooldown,
        asked
            .first_check
            .as_deref()
            .unwrap_or(sync.first_check.as_str()),
    );
    let settings = match checked {
        Ok(settings) => settings,
        Err(message) => return refuse(&message),
    };
    match journal
        .update_sync(sync.id, &settings)
        .and_then(|()| journal.sync_by_name(&asked.name))
    {
        Ok(sync) => {
            println!("`{}`: {}", sync.name, promise(&sync));
            println!("{}", removal_line(&sync));
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
    match (sync.direction, sync.exact) {
        (SyncDirection::Push, false) => {
            format!("every member ends up with everything {anchor} holds")
        }
        (SyncDirection::Push, true) => format!("every member ends up exactly as {anchor} is"),
        (SyncDirection::Pull, false) => {
            format!("{anchor} ends up with everything every member holds")
        }
        (SyncDirection::Pull, true) => {
            format!("{anchor} ends up holding exactly what the other members hold")
        }
        (SyncDirection::All, false) => "every member ends up with everything".to_string(),
        (SyncDirection::All, true) => {
            "every member ends up the same, deletions included".to_string()
        }
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
                    "exact": sync.exact,
                    "on_conflict": sync.on_conflict.as_str(),
                    "on_remove": sync.on_remove.as_str(),
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
    asked: &Asked,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> Result<(Opened, Decided), ExitCode> {
    let opened = tungstate_sync::open(journal, name, secrets).map_err(|e| fail(&e))?;
    let decided = tungstate_sync::decide_with(&opened, journal, asked).map_err(|e| fail(&e))?;
    Ok((opened, decided))
}

fn preview(name: &str, json: bool, journal: &Journal, secrets: &dyn SecretStore) -> ExitCode {
    let (opened, decided) = match decided(name, &Asked::default(), journal, secrets) {
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
    let (opened, decided) = match decided(name, &Asked::default(), journal, secrets) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    carry_out(&opened, &decided, yes, parallel, json, journal)
}

/// Show, check, confirm and run a decided plan: what `run`, `resolve` and
/// `forget` all end in.
fn carry_out(
    opened: &Opened,
    decided: &Decided,
    yes: bool,
    parallel: Option<usize>,
    json: bool,
    journal: &Journal,
) -> ExitCode {
    if !json {
        say(opened, decided);
    }
    if decided.plan.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&document(opened, decided)).unwrap_or_default()
            );
        }
        return ExitCode::SUCCESS;
    }
    let refused = tungstate_sync::refusals(opened, decided);
    if !yes && let Some(first) = refused.first() {
        eprint!("{}", refusal(opened, first));
        return ExitCode::FAILURE;
    }
    let removes = decided.plan.blast.values().any(|b| b.removing > 0);
    let interactive = !yes && !json && std::io::IsTerminal::is_terminal(&std::io::stdin());
    if removes && !yes && !interactive {
        // A run that only adds goes ahead unattended, as `link run` does.
        // One that takes files off a member waits for somebody to say so.
        eprintln!(
            "refusing: this run takes files off a member, and nobody is here to confirm it.\n\
             Run `tungstate sync preview {}` and read it, then pass --yes to run it unattended.",
            opened.sync.name
        );
        return ExitCode::FAILURE;
    }
    if interactive && !confirm() {
        println!("nothing moved");
        return ExitCode::SUCCESS;
    }
    let mut progress = crate::CliProgress::new();
    match tungstate_sync::run(opened, decided, journal, &mut progress, parallel) {
        Ok(ran) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&ran_document(&ran)).unwrap_or_default()
                );
            } else {
                report(opened, decided, &ran);
            }
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

/// A refusal in words, as the reorganisation breaker words its own.
fn refusal(opened: &Opened, refused: &Refusal) -> String {
    match refused {
        Refusal::Hollow { member, held } => format!(
            "refusing: `{member}` lists no files, but after the last run it held {}.\n\
             An unmounted volume or a wrong folder looks exactly like everything being deleted.\n\
             Check `{member}`, then pass --yes if it really is empty.\n",
            files(*held as u64)
        ),
        Refusal::Blast {
            member,
            taking_off,
            of,
        } => format!(
            "refusing: this would take {taking_off} of {} off `{member}`, past the \
             {}-file / {}% limit.\n\
             Run `tungstate sync preview {}` and read it, then pass --yes if that is what you meant.\n",
            files(*of as u64),
            tungstate_core::plan::Blast::LIMIT_FILES,
            100 / tungstate_core::plan::Blast::LIMIT_SHARE,
            opened.sync.name
        ),
    }
}

fn ran_document(ran: &Ran) -> serde_json::Value {
    json!({
        "plan": ran.plan.map(|p| p.0),
        "legs": ran.legs.iter().map(|leg| json!({
            "from": leg.from,
            "to": leg.to,
            "copied": leg.summary.transferred,
            "bytes": leg.summary.bytes,
            "failed": leg.summary.failures.len(),
        })).collect::<Vec<_>>(),
        "taken_off": ran.taken_off,
        "renamed": ran.renamed,
        "missed": ran.missed.iter().map(|m| json!({
            "member": m.member,
            "path": m.path,
            "why": m.why,
        })).collect::<Vec<_>>(),
    })
}

/// What a run did, in words.
fn report(opened: &Opened, decided: &Decided, ran: &Ran) {
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
    if ran.taken_off > 0 {
        let how = if decided.plan.reversible() {
            "set aside"
        } else {
            "set aside or deleted"
        };
        println!("{} {how}", files(ran.taken_off as u64));
    }
    if ran.renamed > 0 {
        println!("{} renamed", files(ran.renamed as u64));
    }
    if !ran.missed.is_empty() {
        println!("not done, and tried again next run:");
        for missed in &ran.missed {
            println!("  {} on {}: {}", missed.path, missed.member, missed.why);
        }
    }
    let conflicts = decided
        .plan
        .left_alone
        .iter()
        .filter(|l| matches!(l.why, Why::Conflict { .. }))
        .count();
    if conflicts > 0 {
        println!(
            "{} changed differently on more than one member; settle each with \
             `tungstate sync resolve {} <path> --keep <member>` or `--keep-both`",
            files(conflicts as u64),
            opened.sync.name
        );
    }
    if let Some(plan) = ran.plan {
        if decided.plan.reversible() {
            println!(
                "run {}: `tungstate sync undo {}` puts it back",
                plan.0, opened.sync.name
            );
        } else {
            println!(
                "run {}: it deleted files outright, so it cannot be put back",
                plan.0
            );
        }
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

/// A path as typed, as the sync spells paths: forward slashes, no `./` in
/// front and no `/` behind.
fn spelled(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.trim_start_matches("./").trim_end_matches('/');
    path.to_string()
}

fn resolve(
    name: &str,
    path: &str,
    choice: &Choice,
    yes: bool,
    json: bool,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> ExitCode {
    let sync = match journal.sync_by_name(name) {
        Ok(sync) => sync,
        Err(error) => return fail(&error),
    };
    let path = spelled(path);
    let resolution = match &choice.keep {
        Some(keep) => match sync.members.iter().find(|m| &m.name == keep) {
            Some(member) => Resolve::Keep(member.id.0),
            None => return refuse(&format!("`{name}` has no member called `{keep}`")),
        },
        None => Resolve::KeepBoth,
    };
    let asked = Asked {
        resolve: [(path.clone(), resolution)].into(),
        only: true,
        ..Asked::default()
    };
    let (opened, decided) = match decided(name, &asked, journal, secrets) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    if let Resolve::Keep(id) = resolution
        && !decided
            .members
            .iter()
            .any(|m| m.id == id && m.files.contains_key(&path))
    {
        return refuse(&format!(
            "`{}` does not hold `{path}`, so its version cannot be kept",
            opened.name(id)
        ));
    }
    if decided.plan.is_empty() && !json {
        println!("nothing to settle at `{path}`: every member already agrees");
        return ExitCode::SUCCESS;
    }
    carry_out(&opened, &decided, yes, None, json, journal)
}

fn forget(
    name: &str,
    path: &str,
    yes: bool,
    json: bool,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> ExitCode {
    let path = spelled(path);
    if path.is_empty() {
        return refuse(
            "say which file or folder to forget; forgetting everything is `sync remove`",
        );
    }
    let asked = Asked {
        forget: [path.clone()].into(),
        only: true,
        ..Asked::default()
    };
    let (opened, decided) = match decided(name, &asked, journal, secrets) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    if let Some(stuck) = decided
        .plan
        .left_alone
        .iter()
        .find(|l| matches!(l.why, Why::NeedsRename { .. }))
    {
        // All or nothing: forgotten on some members and not others, it would
        // be copied back to them by the next run.
        return refuse(&format!(
            "cannot forget `{path}`: {}; nothing was changed",
            reason(&opened, &stuck.why)
        ));
    }
    let taken: Vec<&str> = decided
        .plan
        .ops
        .iter()
        .filter_map(|op| match op {
            SyncOp::Remove { member, .. } => Some(opened.name(*member)),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if decided.plan.is_empty() {
        return refuse(&format!("no member of `{name}` holds `{path}`"));
    }
    let code = carry_out(&opened, &decided, yes, None, json, journal);
    if code == ExitCode::SUCCESS && !json && !taken.is_empty() {
        let (how, back) = match opened.sync.on_remove {
            OnRemove::SetAside => (
                "set aside",
                format!("`tungstate sync undo {name}` puts it back"),
            ),
            OnRemove::Delete => ("deleted", "it cannot be put back".to_string()),
        };
        println!(
            "forgot `{path}`: {how} on {}; it will not come back, and {back}",
            taken.join(" and ")
        );
    }
    code
}

fn undo(
    name: &str,
    last: Option<usize>,
    plan: Option<i64>,
    json: bool,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> ExitCode {
    let opened = match tungstate_sync::open(journal, name, secrets) {
        Ok(opened) => opened,
        Err(error) => return fail(&error),
    };
    let chosen: Vec<PlanId> = match plan {
        Some(id) => vec![PlanId(id)],
        None => match tungstate_sync::standing(journal, &opened.sync, last.unwrap_or(1)) {
            Ok(runs) => runs.into_iter().map(|run| run.id).collect(),
            Err(error) => return fail(&error),
        },
    };
    if chosen.is_empty() {
        println!("nothing to put back: `{name}` has no runs standing");
        return ExitCode::SUCCESS;
    }
    let mut documents = Vec::new();
    for id in chosen {
        match tungstate_sync::undo(&opened, journal, id) {
            Ok(undone) => {
                if json {
                    documents.push(json!({
                        "plan": id.0,
                        "put_back": undone.put_back,
                        "taken_off": undone.taken_off,
                        "parked_left": undone.parked_left,
                        "revived": undone.revived.iter().map(|(member, path)| json!({
                            "member": member,
                            "path": path,
                        })).collect::<Vec<_>>(),
                    }));
                } else {
                    say_undone(name, id, &undone);
                }
            }
            Err(error) => return fail(&error),
        }
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&documents).unwrap_or_default()
        );
    }
    ExitCode::SUCCESS
}

fn say_undone(name: &str, id: PlanId, undone: &tungstate_sync::Undone) {
    println!(
        "put back run {} of `{name}`: {} moved back, {} delivered taken off again",
        id.0,
        files(undone.put_back as u64),
        files(undone.taken_off as u64)
    );
    for (member, path) in &undone.revived {
        println!("  the next run brings `{path}` back to {member}");
    }
    if undone.parked_left > 0 {
        println!(
            "  {} parked by that run left in the set-aside area",
            files(undone.parked_left as u64)
        );
    }
}

/// `tungstate undo --plan ID` where the plan is a sync run: put it back on
/// every member. `None` when it is not one, for the folder undo to handle.
pub fn undo_run(plan: i64) -> Option<ExitCode> {
    let journal = crate::open_journal().ok()?;
    let recorded = journal.plan_by_id(PlanId(plan)).ok()?;
    if recorded.purpose != Some(Purpose::Sync) {
        return None;
    }
    let name = recorded.folder.strip_prefix("sync:")?;
    println!("plan {plan} is a run of sync `{name}`; putting it back on every member");
    Some(undo(
        name,
        None,
        Some(plan),
        false,
        &journal,
        crate::secret_store().as_ref(),
    ))
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
        let parked = if leg.parked.is_empty() {
            String::new()
        } else {
            format!(", {} of them parked", leg.parked.len())
        };
        println!(
            "  {} → {}: {} ({}){parked}{}",
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
        if decided.read_for_moves > 0 {
            why.push(format!(
                "{} to tell a moved file from a new one",
                decided.read_for_moves
            ));
        }
        if decided.read_for_parked > 0 {
            why.push(format!(
                "{} to see whether a conflicting version is already parked",
                decided.read_for_parked
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
    if !plan.is_empty() {
        let deleting: usize = plan.blast.values().map(|b| b.deleting).sum();
        if deleting > 0 {
            println!(
                "this run cannot be undone: it deletes {} outright",
                files(deleting as u64)
            );
        }
        for refused in tungstate_sync::refusals(opened, decided) {
            print!(
                "would be refused without --yes: {}",
                refusal(opened, &refused).trim_start_matches("refusing: ")
            );
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
    // Copied and removed are separate numbers, never added together. A sync
    // removes one way only, so one word covers both kinds.
    let how = if blast.deleting > 0 {
        "deleted"
    } else {
        "set aside"
    };
    if blast.replacing > 0 {
        parts.push(format!(
            "{} replaced, the old version {how}",
            blast.replacing
        ));
    }
    if blast.removing > 0 {
        parts.push(format!("{} removed ({how})", files(blast.removing as u64)));
    }
    if blast.renaming > 0 {
        parts.push(format!("{} renamed", files(blast.renaming as u64)));
    }
    if blast.parking > 0 {
        parts.push(format!(
            "{} parked",
            if blast.parking == 1 {
                "1 conflicting version".to_string()
            } else {
                format!("{} conflicting versions", blast.parking)
            }
        ));
    }
    parts.join(", ")
}

fn names(opened: &Opened, members: &[i64]) -> String {
    members
        .iter()
        .map(|m| opened.name(*m))
        .collect::<Vec<_>>()
        .join(" and ")
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
        Why::Conflict { members } => match opened.sync.on_conflict {
            ConflictAction::Skip => format!(
                "changed differently on {}; left until `tungstate sync resolve`",
                names(opened, members)
            ),
            _ => format!(
                "changed differently on {}; each keeps its own, and the others' versions \
                 are parked beside it in .tungstate-quarantine",
                names(opened, members)
            ),
        },
        Why::CaseClash { member, existing } => {
            format!(
                "{} already holds `{existing}`, which differs only in case",
                opened.name(*member)
            )
        }
        Why::NeedsRename { member } => format!(
            "{} cannot rename, so its copy cannot be set aside",
            opened.name(*member)
        ),
    }
}

/// A preview as data, for `--json`.
fn document(opened: &Opened, decided: &Decided) -> serde_json::Value {
    let plan: &SyncPlan = &decided.plan;
    json!({
        "sync": opened.sync.name,
        "direction": opened.sync.direction.as_str(),
        "exact": opened.sync.exact,
        "reversible": plan.reversible(),
        "members": opened.places.iter().map(|p| {
            let b = plan.blast.get(&p.member.id.0).cloned().unwrap_or_default();
            json!({
                "name": p.member.name,
                "at": p.described,
                "arriving": b.arriving,
                "arriving_bytes": b.arriving_bytes,
                "leaving": b.leaving,
                "replacing": b.replacing,
                "removing": b.removing,
                "deleting": b.deleting,
                "renaming": b.renaming,
                "parking": b.parking,
                "holds": b.of,
                "over_limit": b.over_limit,
                "hollow": plan.hollow.contains(&p.member.id.0),
            })
        }).collect::<Vec<_>>(),
        "legs": plan.legs.iter().map(|l| json!({
            "from": opened.name(l.from),
            "to": opened.name(l.to),
            "files": l.paths.len(),
            "parked": l.parked.len(),
            "bytes": l.bytes,
            "read_through_here": !opened_local(opened, l.from) && !opened_local(opened, l.to),
        })).collect::<Vec<_>>(),
        "read": decided.read,
        "sampled": decided.sampled,
        "read_for_no_times": decided.read_for_no_times,
        "read_for_moves": decided.read_for_moves,
        "read_for_parked": decided.read_for_parked,
        "left_alone": plan.left_alone.iter().map(|l| json!({
            "path": l.path,
            "why": reason(opened, &l.why),
        })).collect::<Vec<_>>(),
    })
}
