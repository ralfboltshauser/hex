//! Concrete safety-copy cleanup for explicit History deletion. Retention never
//! calls this: unresolved recovery work has its own owner and expiry policy.
use super::*;

impl History {
    pub fn attach_recovery(&self, client: crate::recovery::Client) {
        *self
            .recovery
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(client);
    }
    pub(super) fn delete_safety_copies(&self, ids: Vec<String>) -> io::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let client = self
            .recovery
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| {
                io::Error::other(
                    "recording recovery is not available; start Hex before deleting these entries",
                )
            })?;
        for id in ids {
            let id = crate::recovery::SessionId::try_from(id).map_err(io::Error::other)?;
            match client.delete(id) {
                Ok(()) | Err(crate::recovery::Failure::Busy) => {}
                Err(_) => {
                    return Err(io::Error::other(
                        "safety-copy deletion could not be confirmed; History was kept",
                    ));
                }
            }
        }
        Ok(())
    }
}
