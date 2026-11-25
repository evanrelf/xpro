//! Requests: what an evaluation is asked for, beyond which node's value.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// How to evaluate a node's value.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// How small an image to compute.
    pub scale: PreviewScale,
    /// Stops the evaluation early, when its result is no longer wanted.
    pub cancel: Cancel,
}

impl Request {
    /// A request for images at `scale`.
    #[must_use]
    pub fn at_scale(scale: PreviewScale) -> Self {
        Self {
            scale,
            ..Self::default()
        }
    }
}

/// How much smaller than full size to compute images, for a fast preview: the number of times
/// to halve their width and height (rounding up). A mosaic is always full size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PreviewScale(pub u8);

impl PreviewScale {
    /// Full size.
    pub const FULL: Self = Self(0);

    /// The number of times images are halved.
    #[must_use]
    pub fn halvings(self) -> u8 {
        self.0
    }
}

/// A flag that stops an evaluation. Clones share the flag, so one can be kept to cancel an
/// evaluation running elsewhere.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stops evaluations using this flag, or any clone of it, as soon as they check it.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Fails with [`Cancelled`] if cancelled.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

/// The error from an evaluation that was cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

impl Cancelled {
    /// Whether an error, or any error that caused it, is [`Cancelled`].
    #[must_use]
    pub fn caused(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| cause.is::<Self>())
    }
}

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cancelled")
    }
}

impl std::error::Error for Cancelled {}
