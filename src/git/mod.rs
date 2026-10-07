//! Git backend. Like IntelliJ's git4idea, every operation shells out to the
//! `git` executable and parses its machine-readable output.

mod command;
pub mod blame;
pub mod blob;
pub mod changelists;
pub mod diff;
pub mod gpg;
pub mod graph;
pub mod log;
pub mod merge;
pub mod ops;
pub mod patch;
pub mod rebase;
pub mod refs;
pub mod remotes;
pub mod status;

pub use command::{GitConsole, Repository, run_in};
pub use graph::GraphLayout;
pub use log::{Commit, CommitDetails, FileChangeKind, LogFilter};
pub use refs::{RefKind, RefName, RepositoryRefs};
pub use status::{StatusKind, WorkingTreeStatus};

/// A long-running operation the repository is in the middle of, shown in the
/// branch widget the way IntelliJ shows "Rebasing master".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryState {
    Normal,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
}
