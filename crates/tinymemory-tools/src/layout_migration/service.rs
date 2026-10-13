//! The migration as a service: what a host's RPCs and background job call.
//!
//! - [`scan`]: whether there is anything to move (a UI shows nothing when
//!   there is not).
//! - [`status`]: its progress, with a run the app quit in the middle of
//!   reported as interrupted.
//! - [`retry`]: puts the items that could not be moved back in line.
//! - [`is_finished`]: whether a background tick has nothing left to start.
//! - [`record_failure`]: saves why a run stopped on an error.
//!
//! Running in the background (one run per workspace, a spawned task) needs an
//! executor, which this crate does not link: the host owns that, and passes
//! `running` in.

use serde::Serialize;

use super::copy::legacy_present;
use super::error::{Error, Result};
use super::job::LayoutHost;
use super::state::{MigrationState, Phase};

/// What [`scan`] found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanView {
    /// There is legacy memory still to move.
    pub needed: bool,
    /// The legacy tree may be shared with other accounts, so moving it needs
    /// the user's consent.
    pub shared: bool,
}

/// [`status`]'s answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MigrationStatus {
    /// The saved progress.
    pub state: MigrationState,
    /// A run is going now.
    pub running: bool,
    /// The app stopped in the middle of a run; it resumes on its own, or the
    /// user starts it again.
    pub interrupted: bool,
}

/// Whether the host's account has legacy memory to move.
///
/// # Errors
///
/// When the state cannot be read, the engines cannot be bound, or the legacy
/// tree cannot be read.
pub async fn scan(host: &dyn LayoutHost) -> Result<ScanView> {
    let state = host.store().load()?;
    let claim = match host.legacy_claim() {
        // Memory off or signed out: nothing to show.
        Err(Error::Off(_)) => None,
        claim => claim?,
    };
    let shared = claim.is_some();
    let needed = match state.phase {
        Phase::Cleaned => false,
        // Another account on this machine took the shared tree.
        Phase::Idle
            if claim
                .as_ref()
                .map(|claim| claim.held_by_other())
                .transpose()?
                == Some(true) =>
        {
            false
        }
        Phase::Idle if !state.switched => match host.engines() {
            // Memory off or signed out: nothing to show.
            Err(Error::Off(_)) => false,
            Err(error) => return Err(error),
            Ok(engines) => legacy_present(&*engines.legacy).await?,
        },
        _ => true,
    };
    Ok(ScanView { needed, shared })
}

/// The migration's progress; `running` is whether the host has a run going.
///
/// # Errors
///
/// When the state cannot be read.
pub fn status(host: &dyn LayoutHost, running: bool) -> Result<MigrationStatus> {
    let state = host.store().load()?;
    let interrupted = !running && matches!(state.phase, Phase::Copying | Phase::Cleaning);
    Ok(MigrationStatus {
        state,
        running,
        interrupted,
    })
}

/// Puts the items that could not be moved back in line: the next run copies
/// again from the start (what was copied replays) and cleans up again.
///
/// # Errors
///
/// When the state cannot be read or saved, or a run is going now.
pub fn retry(host: &dyn LayoutHost, running: bool) -> Result<MigrationState> {
    if running {
        return Err(Error::invalid("the migration is running"));
    }
    let store = host.store();
    let mut state = store.load()?;
    state.failures.clear();
    state.incomplete.clear();
    state.cursor = None;
    state.error = None;
    if state.switched {
        state.caught_up = false;
        state.cleaning = false;
    }
    if state.phase != Phase::Idle {
        state.phase = Phase::Paused;
    }
    store.save(&state)?;
    Ok(state)
}

/// Whether the migration is done for good: cleaned, with no import left to
/// re-check. A background tick starts nothing then.
///
/// # Errors
///
/// When the state cannot be read.
pub fn is_finished(host: &dyn LayoutHost) -> Result<bool> {
    let state = host.store().load()?;
    Ok(state.phase == Phase::Cleaned && (state.rechecked || !host.listed_unconfirmed()))
}

/// Saves why a run stopped on `error`, so the status shows it.
pub fn record_failure(host: &dyn LayoutHost, error: &Error) {
    let store = host.store();
    if let Ok(mut state) = store.load() {
        state.phase = Phase::Paused;
        state.error = Some(error.to_string());
        if let Err(save) = store.save(&state) {
            log::warn!(
                "[memory:layout_migration] state not saved code={}",
                save.code()
            );
        }
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
