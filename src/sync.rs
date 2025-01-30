#[cfg(all(feature = "shuttle"))]
pub use shuttle::{sync::*, thread};

#[cfg(not(feature = "shuttle"))]
pub use parking_lot::*;
#[cfg(not(feature = "shuttle"))]
pub use std::{sync::*, thread};
