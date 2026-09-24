//! The policy model and the classifier: where a file belongs, as a pure
//! function of its attributes and the policy.
//!
//! **No I/O of any kind.** Attributes arrive as data ([`Attributes`]) and a
//! decision leaves as data ([`Explanation`]). DESIGN.md §7 calls this the
//! load-bearing decision for testability: the whole classifier can be
//! property-tested without a filesystem, and `tungstate-attrs` is where the
//! reading happens.
//!
//! Errors carry byte spans as plain `Range<usize>`. This crate does not know
//! how to draw them; the CLI does, with `miette`.

pub mod attrs;
pub mod classify;
pub mod compare;
pub mod dupes;
pub mod error;
pub mod grammar;
pub mod graph;
pub mod learn;
pub mod matcher;
pub mod plan;
pub mod policy;
pub mod similar;
pub mod snapshot;
pub mod sync;
pub mod template;
pub mod templates;
pub mod vars;

pub use attrs::{Attributes, Tier, Value};
pub use classify::{Explanation, Outcome, RuleResult, RuleTrace, VarTrace};
pub use error::{PolicyError, Warning, line_of};
pub use plan::{Because, Blast, Op, Parked, Plan, PlanError, Reason, Untouched};
pub use policy::{Loaded, Mode, Policy, Rule, Symlinks};
pub use snapshot::Snapshot;
pub use templates::{TEMPLATES, Template};

#[cfg(test)]
mod dupes_tests;
#[cfg(test)]
mod plan_tests;
#[cfg(test)]
mod similar_tests;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod tests;
