//! The migration's error: an engine failure, or something the host reported.

use tinymemory_api::Error as EngineError;

/// The TinyHumans backend's code for an exhausted credit balance (HTTP 402),
/// which hosted failures put on their message as a `[CODE] ` prefix.
const INSUFFICIENT_CREDITS_CODE: &str = "USER_INSUFFICIENT_CREDITS";

/// Why a migration step failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The engine refused or failed.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// Memory is off: no usable engine, or nobody signed in to move for.
    #[error("memory is off: {0}")]
    Off(String),
    /// The request was malformed or refused (for example a run is going).
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// The saved state could not be read or written.
    #[error("engine error: {0}")]
    State(String),
}

/// Result alias for migration steps.
pub type Result<T> = std::result::Result<T, Error>;

/// Whether `error` is the hosted backend's "not enough credits" refusal.
fn is_insufficient_credits(error: &EngineError) -> bool {
    let EngineError::Engine(message) = error else {
        return false;
    };
    message
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
        .is_some_and(|(code, _)| code == INSUFFICIENT_CREDITS_CODE)
}

impl Error {
    /// Shorthand for [`Self::InvalidRequest`].
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    /// The stable code of this failure (never content), as the host's wire
    /// taxonomy names it.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Off(_) => "MEMORY_OFF",
            Self::InvalidRequest(_) => "INVALID_REQUEST",
            Self::State(_) => "ENGINE",
            Self::Engine(error) => match error {
                EngineError::Unsupported(_) => "UNSUPPORTED",
                EngineError::InvalidRequest(_)
                | EngineError::NotFound(_)
                | EngineError::Config(_) => "INVALID_REQUEST",
                EngineError::Unauthorized(_) => "UNAUTHORIZED",
                EngineError::Unavailable(_) => "UNAVAILABLE",
                EngineError::Engine(_) if is_insufficient_credits(error) => "INSUFFICIENT_CREDITS",
                _ => "ENGINE",
            },
        }
    }

    /// Whether this refuses every request on the account, not just this one:
    /// memory off, a rejected credential, no credits, or no engine to reach.
    /// A loop over items stops on these instead of skipping every item.
    #[must_use]
    pub fn is_account_wide(&self) -> bool {
        match self {
            Self::Off(_) => true,
            Self::Engine(error) => {
                matches!(
                    error,
                    EngineError::Unauthorized(_) | EngineError::Unavailable(_)
                ) || is_insufficient_credits(error)
            }
            Self::InvalidRequest(_) | Self::State(_) => false,
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
