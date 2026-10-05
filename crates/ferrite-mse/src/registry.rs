//! How the DOM's `MediaSource` and the media player find the same [`Shared`]: the DOM
//! registers it and puts the number in the object URL; the player is created with the
//! number.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use crate::Shared;

static NEXT: AtomicU64 = AtomicU64::new(1);
static REGISTRY: LazyLock<Mutex<HashMap<u64, Weak<Shared>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Registers a `MediaSource`'s state; the number identifies it until [`unregister`] (or
/// until the last owner drops it).
pub fn register(shared: &Arc<Shared>) -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut map = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, weak| weak.strong_count() > 0);
    map.insert(id, Arc::downgrade(shared));
    id
}

pub fn lookup(id: u64) -> Option<Arc<Shared>> {
    REGISTRY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .and_then(Weak::upgrade)
}

pub fn unregister(id: u64) {
    REGISTRY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_follows_the_owner() {
        let shared = Shared::new();
        let id = register(&shared);
        assert!(lookup(id).is_some());
        unregister(id);
        assert!(lookup(id).is_none());
        let id = register(&shared);
        drop(shared);
        assert!(lookup(id).is_none());
    }
}
