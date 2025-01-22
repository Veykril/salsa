use crate::{hash::FxLinkedHashSet, Id};

use crossbeam::atomic::AtomicCell;
use parking_lot::Mutex;

mod sealed {
    pub trait Sealed {}
}

pub trait LruChoice: sealed::Sealed + Default {
    /// Records the `index` into this LRU, returning the index to evict if any.
    fn record_use(&self, index: Id) -> Option<Id>;
    /// Fetches all elements that should be evicted.
    fn to_be_evicted(&self, cb: impl FnMut(Id));
    /// Sets the capacity of the LRU.
    fn set_capacity(&self, capacity: usize);
}

/// An LRU choice that does not evict anything.
#[derive(Default)]
pub struct NoLru;

impl sealed::Sealed for NoLru {}
impl LruChoice for NoLru {
    #[inline(always)]
    fn record_use(&self, _: Id) -> Option<Id> {
        None
    }
    fn to_be_evicted(&self, _: impl FnMut(Id)) {}
    fn set_capacity(&self, _: usize) {}
}

/// An LRU choice that tracks elements but does not evict on its own.
///
/// The user must manually trigger eviction.
#[derive(Default)]
pub struct ManualLru {
    capacity: AtomicCell<usize>,
    set: Mutex<FxLinkedHashSet<Id>>,
}

impl sealed::Sealed for ManualLru {}
impl LruChoice for ManualLru {
    fn record_use(&self, index: Id) -> Option<Id> {
        self.set.lock().insert(index);
        None
    }
    fn to_be_evicted(&self, mut cb: impl FnMut(Id)) {
        let mut set = self.set.lock();
        let cap = self.capacity.load();
        if set.len() <= cap || cap == 0 {
            return;
        }
        while let Some(id) = set.pop_front() {
            cb(id);
            if set.len() <= cap {
                break;
            }
        }
    }
    fn set_capacity(&self, capacity: usize) {
        self.capacity.store(capacity);
    }
}

/// An LRU choice that tracks elements and automatically evicts the least recently used if over capacity.
#[derive(Default)]
pub struct AutomaticLru {
    capacity: AtomicCell<usize>,
    set: Mutex<FxLinkedHashSet<Id>>,
}

impl sealed::Sealed for AutomaticLru {}
impl LruChoice for AutomaticLru {
    fn record_use(&self, index: Id) -> Option<Id> {
        let capacity = self.capacity.load();

        if capacity == 0 {
            // LRU is disabled
            return None;
        }

        let mut set = self.set.lock();
        set.insert(index);
        if set.len() > capacity {
            return set.pop_front();
        }

        None
    }
    fn to_be_evicted(&self, _: impl FnMut(Id)) {}
    fn set_capacity(&self, capacity: usize) {
        self.capacity.store(capacity);

        let mut set = self.set.lock();
        if capacity == 0 {
            *set = FxLinkedHashSet::default();
        } else {
            // + 1 as we may insert an extra element before evicting again
            set.reserve(capacity + 1);
        }
    }
}
