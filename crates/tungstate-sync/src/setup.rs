//! Making and changing a sync, checked the same way whoever asks.
//!
//! The command line and the window both make syncs, and SEAM rule 2 says they
//! must reach the same answer. So the checks live here and return
//! [`Refused`], a reason as data: the command line words it with its flags,
//! the window in plain words, and neither can drift from what is checked.

use std::path::Path;
use std::time::Duration;

use tungstate_journal::{
    ConflictAction, FirstCheck, Journal, JournalError, NewMember, NewSync, OnRemove, Sync,
    SyncDirection, SyncSettings, VerifyLevel, ends,
};

/// Why a sync could not be made or changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Deleting outright only makes sense when deletions are carried: without
    /// exact, all it could ever touch is an old version being replaced.
    DeleteNeedsExact,
    /// With three members changed, "replace" cannot say whose version wins.
    ReplaceHasNoMeaning,
    /// A setting spelled in a way the engine does not know.
    Unknown {
        setting: &'static str,
        value: String,
    },
    /// `All` has no anchor: every member sends and receives.
    AllHasNoAnchor,
    /// The anchor named is not one of the members.
    NoSuchAnchor(String),
    /// More names than folders.
    MoreNamesThanFolders,
    /// Fewer than two folders.
    TooFew,
    /// A folder on this machine that is not there.
    NotAFolder(String),
    /// A `connection:path` that names no connection, or cannot be read.
    BadEnd { raw: String, why: String },
    /// Two members would share a name.
    SameName(String),
    /// One member would sit inside another.
    Overlap(String, String),
    /// A sync by this name already exists.
    Taken(String),
    /// Anything the journal refused for its own reasons, worded by it.
    Journal(String),
}

impl From<JournalError> for Refused {
    fn from(error: JournalError) -> Self {
        match error {
            JournalError::SyncExists(name) => Self::Taken(name),
            other => Self::Journal(other.to_string()),
        }
    }
}

/// The settings as typed, checked.
///
/// # Errors
/// [`Refused`] naming the setting that cannot be.
pub fn settings(
    exact: bool,
    on_conflict: &str,
    on_remove: &str,
    verify: &str,
    cooldown: u64,
    first_check: &str,
) -> Result<SyncSettings, Refused> {
    let unknown = |setting: &'static str, value: &str| Refused::Unknown {
        setting,
        value: value.to_string(),
    };
    let on_remove = OnRemove::parse(on_remove).ok_or_else(|| unknown("on_remove", on_remove))?;
    if on_remove == OnRemove::Delete && !exact {
        return Err(Refused::DeleteNeedsExact);
    }
    let on_conflict =
        ConflictAction::parse(on_conflict).ok_or_else(|| unknown("on_conflict", on_conflict))?;
    if on_conflict == ConflictAction::Replace {
        return Err(Refused::ReplaceHasNoMeaning);
    }
    Ok(SyncSettings {
        exact,
        on_conflict,
        on_remove,
        verify: VerifyLevel::parse(verify).ok_or_else(|| unknown("verify", verify))?,
        cooldown: Duration::from_secs(cooldown),
        first_check: FirstCheck::parse(first_check)
            .ok_or_else(|| unknown("first_check", first_check))?,
    })
}

/// One member from what was typed: a folder here, or `connection:path`.
/// Named after its connection or its folder unless `called` says otherwise.
///
/// # Errors
/// [`Refused::BadEnd`] or [`Refused::NotAFolder`].
pub fn member(raw: &str, called: Option<&str>, journal: &Journal) -> Result<NewMember, Refused> {
    let end = ends::parse_end(raw, None, journal).map_err(|e| Refused::BadEnd {
        raw: raw.to_string(),
        why: e.to_string(),
    })?;
    let path = if end.connection.is_some() {
        end.path.to_string_lossy().to_string()
    } else {
        let resolved = tungstate_journal::resolve_for_lookup(&end.path);
        if !resolved.is_dir() {
            return Err(Refused::NotAFolder(raw.to_string()));
        }
        resolved.to_string_lossy().to_string()
    };
    let name = called.filter(|c| !c.trim().is_empty()).map_or_else(
        || match end.connection {
            // The connection's own name: `nas:capcut` is "nas".
            Some(_) => raw.split_once(':').map_or(raw, |(c, _)| c).to_string(),
            None => Path::new(&path)
                .file_name()
                .map_or_else(|| path.clone(), |n| n.to_string_lossy().to_string()),
        },
        |c| c.trim().to_string(),
    );
    Ok(NewMember {
        name,
        connection: end.connection,
        path,
    })
}

/// Members need names a person can tell apart, and folders that do not sit
/// inside one another: two members over one directory would fight over it.
///
/// # Errors
/// [`Refused::SameName`] or [`Refused::Overlap`].
pub fn distinct(members: &[NewMember]) -> Result<(), Refused> {
    for (i, a) in members.iter().enumerate() {
        for b in &members[i + 1..] {
            if a.name == b.name {
                return Err(Refused::SameName(a.name.clone()));
            }
            let (pa, pb) = (Path::new(&a.path), Path::new(&b.path));
            if a.connection == b.connection && (pa.starts_with(pb) || pb.starts_with(pa)) {
                return Err(Refused::Overlap(a.name.clone(), b.name.clone()));
            }
        }
    }
    Ok(())
}

/// A sync as asked for.
#[derive(Debug, Clone)]
pub struct Setup {
    pub name: String,
    /// Two or more folders: a path, or `connection:path`.
    pub ends: Vec<String>,
    /// A name per member, in order; fewer is fine, and the rest are chosen.
    pub names: Vec<String>,
    pub direction: SyncDirection,
    /// For `Push` and `Pull`; the first member when `None`.
    pub anchor: Option<String>,
    pub settings: SyncSettings,
}

/// Make a sync.
///
/// # Errors
/// [`Refused`] naming what cannot be.
pub fn create(journal: &Journal, asked: &Setup) -> Result<Sync, Refused> {
    if asked.direction == SyncDirection::All && asked.anchor.is_some() {
        return Err(Refused::AllHasNoAnchor);
    }
    if asked.names.len() > asked.ends.len() {
        return Err(Refused::MoreNamesThanFolders);
    }
    if asked.ends.len() < 2 {
        return Err(Refused::TooFew);
    }
    let members = asked
        .ends
        .iter()
        .enumerate()
        .map(|(index, raw)| member(raw, asked.names.get(index).map(String::as_str), journal))
        .collect::<Result<Vec<_>, _>>()?;
    distinct(&members)?;
    let anchor = if asked.direction == SyncDirection::All {
        None
    } else {
        let anchor = asked
            .anchor
            .clone()
            .unwrap_or_else(|| members[0].name.clone());
        if !members.iter().any(|m| m.name == anchor) {
            return Err(Refused::NoSuchAnchor(anchor));
        }
        Some(anchor)
    };
    let settings = &asked.settings;
    journal.create_sync(
        &NewSync {
            name: asked.name.clone(),
            direction: asked.direction,
            exact: settings.exact,
            anchor,
            on_conflict: settings.on_conflict,
            on_remove: settings.on_remove,
            verify: settings.verify,
            cooldown: settings.cooldown,
            first_check: settings.first_check,
        },
        &members,
    )?;
    Ok(journal.sync_by_name(&asked.name)?)
}

/// Add a member to a sync, checked against the ones it has.
///
/// # Errors
/// As [`member`] and [`distinct`].
pub fn add_member(
    journal: &Journal,
    sync: &Sync,
    raw: &str,
    called: Option<&str>,
) -> Result<NewMember, Refused> {
    let new = member(raw, called, journal)?;
    let all: Vec<NewMember> = sync
        .members
        .iter()
        .map(|m| NewMember {
            name: m.name.clone(),
            connection: m.connection,
            path: m.path.clone(),
        })
        .chain(std::iter::once(new.clone()))
        .collect();
    distinct(&all)?;
    journal.add_member(sync.id, &new)?;
    Ok(new)
}
