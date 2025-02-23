//! Public API facades for the implementation details of [`Zalsa`] and [`ZalsaLocal`].
use std::{marker::PhantomData, panic::RefUnwindSafe, sync::Arc};

use parking_lot::{Condvar, Mutex};

use crate::{
    zalsa::{Zalsa, ZalsaDatabase},
    zalsa_local::{self, ZalsaLocal},
    Database, Event, EventKind,
};

/// A handle to non-local database state.
pub struct StorageHandle<Db> {
    // Note: Drop order is important, zalsa_impl needs to drop before coordinate
    /// Reference to the database.
    zalsa_impl: Arc<Zalsa>,

    // Note: Drop order is important, coordinate needs to drop after zalsa_impl
    /// Coordination data for cancellation of other handles when `zalsa_mut` is called.
    /// This could be stored in Zalsa but it makes things marginally cleaner to keep it separate.
    coordinate: CoordinateDrop,

    /// We store references to `Db`
    phantom: PhantomData<fn() -> Db>,
}

impl<Db> Clone for StorageHandle<Db> {
    fn clone(&self) -> Self {
        *self.coordinate.clones.lock() += 1;

        Self {
            zalsa_impl: self.zalsa_impl.clone(),
            coordinate: CoordinateDrop(Arc::clone(&self.coordinate)),
            phantom: PhantomData,
        }
    }
}

impl<Db: Database> Default for StorageHandle<Db> {
    fn default() -> Self {
        Self {
            zalsa_impl: Arc::new(Zalsa::new::<Db>()),
            coordinate: CoordinateDrop(Arc::new(Coordinate {
                clones: Mutex::new(1),
                cvar: Default::default(),
            })),
            phantom: PhantomData,
        }
    }
}

impl<Db> StorageHandle<Db> {
    pub fn into_storage(self) -> Storage<Db> {
        Storage {
            handle: self,
            zalsa_local: ZalsaLocal::new(),
        }
    }
}

/// Access the "storage" of a Salsa database: this is an internal plumbing trait
/// automatically implemented by `#[salsa::db]` applied to a struct.
///
/// # Safety
///
/// The `storage` and `storage_mut` fields must both return a reference to the same
/// storage field which must be owned by `self`.
pub unsafe trait HasStorage: Database + Clone + Sized {
    fn storage(&self) -> &Storage<Self>;
    fn storage_mut(&mut self) -> &mut Storage<Self>;
}

/// Concrete implementation of the [`Database`] trait with local state that can be used to drive computations.
pub struct Storage<Db> {
    handle: StorageHandle<Db>,

    /// Per-thread state
    zalsa_local: zalsa_local::ZalsaLocal,
}

impl<Db> Drop for Storage<Db> {
    fn drop(&mut self) {
        self.zalsa_local
            .record_unfilled_pages(self.handle.zalsa_impl.table());
    }
}

struct Coordinate {
    /// Counter of the number of clones of actor. Begins at 1.
    /// Incremented when cloned, decremented when dropped.
    clones: Mutex<usize>,
    cvar: Condvar,
}

// We cannot panic while holding a lock to `clones: Mutex<usize>` and therefore we cannot enter an
// inconsistent state.
impl RefUnwindSafe for Coordinate {}

impl<Db: Database> Default for Storage<Db> {
    fn default() -> Self {
        Self {
            handle: StorageHandle::default(),
            zalsa_local: ZalsaLocal::new(),
        }
    }
}

impl<Db: Database> Storage<Db> {
    /// Convert this instance of [`Storage`] into a [`StorageHandle`].
    ///
    /// This will discard the local state of this [`Storage`], thereby returning a value that
    /// is both [`Sync`] and [`std::panic::UnwindSafe`].
    pub fn into_zalsa_handle(mut self) -> StorageHandle<Db> {
        self.zalsa_local
            .record_unfilled_pages(self.handle.zalsa_impl.table());
        let Self {
            handle,
            zalsa_local: _,
        } = &self;
        // Avoid rust's annoying destructure prevention rules for `Drop` types
        // SAFETY: We forget `Self` afterwards to discard the original copy, and the destructure
        // above makes sure we won't forget to take into account newly added fields.
        let handle = unsafe { std::ptr::read(handle) };
        std::mem::forget::<Self>(self);
        handle
    }

    // ANCHOR: cancel_other_workers
    /// Sets cancellation flag and blocks until all other workers with access
    /// to this storage have completed.
    ///
    /// This could deadlock if there is a single worker with two handles to the
    /// same database!
    ///
    /// Needs to be paired with a call to `reset_cancellation_flag`.
    fn cancel_others(&self, db: &Db) {
        self.handle.zalsa_impl.runtime().set_cancellation_flag();

        db.salsa_event(&|| Event::new(EventKind::DidSetCancellationFlag));

        let mut clones = self.handle.coordinate.clones.lock();
        while *clones != 1 {
            self.handle.coordinate.cvar.wait(&mut clones);
        }
    }
    // ANCHOR_END: cancel_other_workers
}

unsafe impl<T: HasStorage> ZalsaDatabase for T {
    fn zalsa(&self) -> &Zalsa {
        &self.storage().handle.zalsa_impl
    }

    fn zalsa_mut(&mut self) -> &mut Zalsa {
        self.storage().cancel_others(self);

        let storage = self.storage_mut();
        // The ref count on the `Arc` should now be 1
        let zalsa = Arc::get_mut(&mut storage.handle.zalsa_impl).unwrap();
        // cancellation is done, so reset the flag
        zalsa.runtime_mut().reset_cancellation_flag();
        zalsa
    }

    fn zalsa_local(&self) -> &ZalsaLocal {
        &self.storage().zalsa_local
    }

    fn fork_db(&self) -> Box<dyn Database> {
        Box::new(self.clone())
    }
}

impl<Db: Database> Clone for Storage<Db> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            zalsa_local: ZalsaLocal::new(),
        }
    }
}

struct CoordinateDrop(Arc<Coordinate>);

impl std::ops::Deref for CoordinateDrop {
    type Target = Arc<Coordinate>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for CoordinateDrop {
    fn drop(&mut self) {
        *self.0.clones.lock() -= 1;
        self.0.cvar.notify_all();
    }
}
