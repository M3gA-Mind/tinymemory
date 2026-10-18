//! The local filesystem implementation of the shared-tree claim port.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest;

use super::error::{Error, Result};
use super::job::LegacyClaim;

/// A hard-link claim marker for a legacy tree shared by local accounts.
///
/// The marker is stored at
/// `<app_dir>/memory/legacy_claims/<sha256(normalized-endpoint)>.json`.
/// Hard-link creation makes the first account the winner without replacing a
/// claim another account already made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsLegacyClaim {
    marker: PathBuf,
    owner: String,
    endpoint: String,
}

impl FsLegacyClaim {
    /// A claim for `owner` on the tree served by `endpoint`.
    #[must_use]
    pub fn new(app_dir: &Path, endpoint: &str, owner: &str) -> Self {
        let endpoint = normalize(endpoint);
        let digest: String = sha2::Sha256::digest(endpoint.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            marker: app_dir
                .join("memory")
                .join("legacy_claims")
                .join(format!("{digest}.json")),
            owner: owner.to_string(),
            endpoint,
        }
    }

    /// The marker path shared by spellings of the same normalized endpoint.
    #[must_use]
    pub fn marker_path(&self) -> &Path {
        &self.marker
    }

    /// The claiming account's actor id.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The normalized endpoint used for this marker.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn recorded_owner(&self) -> Result<Option<String>> {
        match std::fs::read(&self.marker) {
            Ok(bytes) => serde_json::from_slice::<Marker>(&bytes)
                .map(|marker| Some(marker.owner))
                .map_err(|error| unreadable(&error)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(unreadable(&error)),
        }
    }
}

impl LegacyClaim for FsLegacyClaim {
    fn held_by_other(&self) -> Result<bool> {
        Ok(self
            .recorded_owner()?
            .is_some_and(|owner| owner != self.owner))
    }

    fn take(&self) -> Result<bool> {
        if let Some(owner) = self.recorded_owner()? {
            return Ok(owner == self.owner);
        }
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.marker.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let temp = tempfile::NamedTempFile::new_in(
                self.marker.parent().unwrap_or(Path::new(".")),
            )?;
            let marker = Marker {
                endpoint: self.endpoint.clone(),
                owner: self.owner.clone(),
            };
            serde_json::to_writer(temp.as_file(), &marker).map_err(std::io::Error::other)?;
            temp.as_file().sync_all()?;
            // Fails when the marker exists: never replaces another account's
            // claim.
            std::fs::hard_link(temp.path(), &self.marker)
        };
        match write() {
            Ok(()) => {
                log::info!("[memory:layout_migration] legacy tree claimed");
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(self
                .recorded_owner()?
                .is_some_and(|owner| owner == self.owner)),
            Err(error) => Err(Error::State(format!(
                "legacy tree claim not saved: {error}"
            ))),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Marker {
    endpoint: String,
    owner: String,
}

/// One spelling per endpoint: trimmed, scheme and host lowercased, default
/// ports and trailing slashes dropped.
fn normalize(endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    match url::Url::parse(endpoint) {
        Ok(url) => url.as_str().trim_end_matches('/').to_string(),
        Err(_) => endpoint.trim_end_matches('/').to_ascii_lowercase(),
    }
}

fn unreadable(error: &dyn std::fmt::Display) -> Error {
    Error::State(format!("legacy tree claim unreadable: {error}"))
}
