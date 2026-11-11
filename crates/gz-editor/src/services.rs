use std::any::{Any, TypeId};
use std::sync::Arc;

use parking_lot::RwLock;
use rustc_hash::FxHashMap;

/// Values by type: how an area hangs its state on the editor or on the open project without
/// this crate knowing the area.
///
/// The lock guards the table, never a service: a lookup clones an `Arc` and lets go.
#[derive(Default)]
pub struct Services {
    map: RwLock<FxHashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl Services {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let found = self.map.read().get(&TypeId::of::<T>()).cloned();
        found.and_then(|any| any.downcast::<T>().ok())
    }

    /// The service of this type, made by `init` on first use. `init` may run twice when two
    /// threads race for the first use; one result wins and both callers get it.
    pub fn get_or_init<T: Any + Send + Sync>(&self, init: impl FnOnce() -> T) -> Arc<T> {
        if let Some(found) = self.get::<T>() {
            return found;
        }
        let made = Arc::new(init());
        let mut map = self.map.write();
        let existing = map.get(&TypeId::of::<T>()).cloned().and_then(|any| any.downcast::<T>().ok());
        if let Some(existing) = existing {
            return existing;
        }
        map.insert(TypeId::of::<T>(), made.clone());
        made
    }

    /// Like [`Self::get_or_init`] for an `init` that can fail. Nothing is stored on failure.
    pub fn get_or_try_init<T: Any + Send + Sync, E>(
        &self,
        init: impl FnOnce() -> Result<T, E>,
    ) -> Result<Arc<T>, E> {
        if let Some(found) = self.get::<T>() {
            return Ok(found);
        }
        let made = init()?;
        Ok(self.get_or_init(|| made))
    }

    pub fn insert<T: Any + Send + Sync>(&self, value: T) -> Arc<T> {
        let value = Arc::new(value);
        self.map.write().insert(TypeId::of::<T>(), value.clone());
        value
    }

    pub fn remove<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let old = self.map.write().remove(&TypeId::of::<T>());
        old.and_then(|any| any.downcast::<T>().ok())
    }
}
