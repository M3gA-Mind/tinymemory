//! Constrain memory reads and writes to a host-selected namespace reach.

use tinymemory_api::{Error, MemoryMeta, MetaFilter, Namespace, Reach, Result, StoreItem};

/// What to do when a requested namespace falls outside the allowed reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Move out-of-scope reads and writes to the allowed namespace subtree.
    Clamp,
    /// Reject an out-of-scope read or write as an invalid request.
    Refuse,
}

/// A host-selected reach and the policy applied to requests outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confinement {
    /// The namespace reach the host permits.
    pub allowed: Reach,
    /// Whether out-of-scope requests are narrowed or rejected.
    pub policy: Policy,
}

impl Confinement {
    /// Confines a requested reach, or uses the allowed reach when none was set.
    ///
    /// `Clamp` keeps a request rooted within the allowed node while removing
    /// inherited ancestors; a request rooted elsewhere becomes the allowed
    /// reach. `Refuse` keeps a request only when [`Reach::within`] confirms it
    /// cannot read beyond the allowed reach.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRequest`] when `policy` is [`Policy::Refuse`]
    /// and the requested reach is wider than `allowed`.
    pub fn reach(&self, requested: Option<Reach>) -> Result<Reach> {
        match self.policy {
            Policy::Clamp => Ok(match requested {
                Some(mut reach)
                    if (self.allowed.descendants || reach.at == self.allowed.at)
                        && within(&reach.at, &self.allowed.at) =>
                {
                    reach.inherit = false;
                    if !self.allowed.descendants {
                        reach.descendants = false;
                    }
                    reach
                }
                _ => self.allowed.clone(),
            }),
            Policy::Refuse => match requested {
                None => Ok(self.allowed.clone()),
                Some(reach) if reach.within(&self.allowed) => Ok(reach),
                Some(reach) => Err(out_of_reach(&reach, &self.allowed)),
            },
        }
    }

    /// Confines a filter's reach while preserving its other metadata clauses.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRequest`] when `policy` is [`Policy::Refuse`]
    /// and the filter's requested reach is wider than `allowed`.
    pub fn filter(&self, filter: Option<MetaFilter>) -> Result<MetaFilter> {
        let mut filter = filter.unwrap_or_default();
        filter.reach = Some(self.reach(filter.reach.take())?);
        Ok(filter)
    }

    /// Confines an item's namespace in place.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRequest`] when `policy` is [`Policy::Refuse`]
    /// and the item's namespace is outside `allowed`.
    pub fn item(&self, item: &mut StoreItem) -> Result<()> {
        self.meta(item.meta_mut())
    }

    /// Confines metadata for a learning, using default metadata when absent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRequest`] when `policy` is [`Policy::Refuse`]
    /// and the metadata namespace is outside `allowed`.
    pub fn learn_meta(&self, meta: Option<MemoryMeta>) -> Result<MemoryMeta> {
        let mut meta = meta.unwrap_or_default();
        self.meta(&mut meta)?;
        Ok(meta)
    }

    fn meta(&self, meta: &mut MemoryMeta) -> Result<()> {
        match self.policy {
            Policy::Clamp => {
                if !(self.allowed.descendants || meta.namespace == self.allowed.at)
                    || !within(&meta.namespace, &self.allowed.at)
                {
                    meta.namespace = self.allowed.at.clone();
                }
                Ok(())
            }
            Policy::Refuse if self.allowed.admits(&meta.namespace) => Ok(()),
            Policy::Refuse => Err(out_of_namespace(&meta.namespace, &self.allowed)),
        }
    }
}

fn within(namespace: &Namespace, root: &Namespace) -> bool {
    namespace.segments().starts_with(root.segments())
}

fn out_of_reach(requested: &Reach, allowed: &Reach) -> Error {
    Error::InvalidRequest(format!(
        "the reach `{}` is outside this caller's memory (`{}`)",
        requested.at, allowed.at
    ))
}

fn out_of_namespace(requested: &Namespace, allowed: &Reach) -> Error {
    Error::InvalidRequest(format!(
        "the namespace `{requested}` is outside this caller's memory (`{}`)",
        allowed.at
    ))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
