//! Deciding what to do when something already holds the destination name.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use tungstate_journal::ConflictAction;

/// A file that cannot be placed because something else is already there.
#[derive(Debug, Clone)]
pub struct Conflict {
    /// Path relative to the source root.
    pub path: PathBuf,
    /// Size of the file being transferred.
    pub incoming_size: u64,
    /// Size of the file already at the destination.
    pub existing_size: u64,
}

/// What a resolver decided, and whether it applies to everything that follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Just this file.
    Once(ConflictAction),
    /// This file and every later conflict in the run.
    All(ConflictAction),
}

/// Chooses what happens to a conflicting file.
pub trait ConflictResolver {
    /// Decide. Called once per conflict, unless a previous answer applied to all.
    fn resolve(&mut self, conflict: &Conflict) -> ConflictAction;
}

/// Always answers the same way. Used for unattended runs, `--on-conflict`, and tests.
#[derive(Debug, Clone, Copy)]
pub struct FixedResolver(pub ConflictAction);

impl ConflictResolver for FixedResolver {
    fn resolve(&mut self, _conflict: &Conflict) -> ConflictAction {
        self.0
    }
}

/// Asks at the terminal, and remembers an "apply to all" answer.
pub struct InteractiveResolver<R, W> {
    input: R,
    output: W,
    /// Set once the user answers "all", after which nothing more is asked.
    sticky: Option<ConflictAction>,
    /// Used when input runs out mid-run, for instance a closed terminal.
    fallback: ConflictAction,
}

impl<R: BufRead, W: Write> InteractiveResolver<R, W> {
    /// Build a resolver that prompts on `output` and reads from `input`.
    pub fn new(input: R, output: W, fallback: ConflictAction) -> Self {
        Self {
            input,
            output,
            sticky: None,
            fallback,
        }
    }

    fn prompt(&mut self, conflict: &Conflict) -> Option<Decision> {
        let _ = writeln!(
            self.output,
            "\nconflict: `{}` already exists at the destination with different contents\n  \
             incoming {} bytes, existing {} bytes",
            conflict.path.display(),
            conflict.incoming_size,
            conflict.existing_size
        );
        let _ = writeln!(
            self.output,
            "  [r]ename  [s]kip  [q]uarantine  [R]eplace\n  \
             capitalise or add ! to apply to every later conflict"
        );
        let _ = write!(self.output, "action: ");
        let _ = self.output.flush();

        let mut line = String::new();
        match self.input.read_line(&mut line) {
            // End of input: nobody is there to answer after all.
            Ok(0) | Err(_) => None,
            Ok(_) => Some(parse_answer(line.trim(), self.fallback)),
        }
    }
}

/// Read an answer. A trailing `!` or an uppercase letter means "and all the rest".
fn parse_answer(raw: &str, fallback: ConflictAction) -> Decision {
    let sticky = raw.ends_with('!') || raw.chars().next().is_some_and(char::is_uppercase);
    let letter = raw.trim_end_matches('!').chars().next().unwrap_or(' ');

    let action = match letter {
        'r' => ConflictAction::Rename,
        's' => ConflictAction::Skip,
        'q' => ConflictAction::Quarantine,
        // Capital R is Replace; lowercase r is Rename. They differ enough in
        // consequence that sharing a letter would be a trap, so Replace is the
        // one that must be typed deliberately.
        'R' => ConflictAction::Replace,
        _ => fallback,
    };

    if sticky && letter != 'R' {
        Decision::All(action)
    } else if sticky {
        Decision::All(ConflictAction::Replace)
    } else {
        Decision::Once(action)
    }
}

impl<R: BufRead, W: Write> ConflictResolver for InteractiveResolver<R, W> {
    fn resolve(&mut self, conflict: &Conflict) -> ConflictAction {
        if let Some(action) = self.sticky {
            return action;
        }
        match self.prompt(conflict) {
            Some(Decision::Once(action)) => action,
            Some(Decision::All(action)) => {
                self.sticky = Some(action);
                action
            }
            None => self.fallback,
        }
    }
}
