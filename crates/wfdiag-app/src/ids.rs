//! Identity newtypes used for every staleness comparison in this crate.
//!
//! Nothing outside this module compares raw integers: a request id, a
//! generation, and an epoch are three different things, and the native shell's
//! long-standing bugs came from mixing them. [`RequestId`] identifies one
//! in-flight request to one worker; [`Generation`] versions a re-derivable
//! projection (issue detection, provider status); [`Epoch`] versions the
//! committed evidence a projection was derived from.

use std::fmt;

/// A monotonically increasing, never-zero counter.
///
/// Zero is reserved for "nothing has been issued yet", and exhaustion is
/// refused rather than wrapped: an ancient worker reply must never match a
/// fresh request after a numeric wrap.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sequence(u64);

impl Sequence {
    /// A sequence that has issued nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self(0)
    }

    /// The most recently issued value, or `None` before the first issue.
    #[must_use]
    pub const fn current(self) -> Option<u64> {
        if self.0 == 0 { None } else { Some(self.0) }
    }

    /// Issue the next value, or `None` once the sequence is exhausted.
    pub fn advance(&mut self) -> Option<u64> {
        let next = self.0.checked_add(1)?;
        self.0 = next;
        Some(next)
    }
}

macro_rules! identity_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            /// Wrap a raw value. Only this module's counters should mint one.
            #[must_use]
            pub const fn from_raw(value: u64) -> Self {
                Self(value)
            }

            /// The raw value, for logging and worker wire structs.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }
    };
}

identity_newtype! {
    /// One in-flight request to one worker.
    RequestId
}

identity_newtype! {
    /// A version of a re-derivable projection.
    Generation
}

identity_newtype! {
    /// A version of the committed diagnostic evidence.
    Epoch
}

/// A counter that mints [`RequestId`]s.
#[derive(Debug, Default, Clone, Copy)]
pub struct RequestIds(Sequence);

impl RequestIds {
    /// A counter that has issued nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self(Sequence::new())
    }

    /// Issue the next id, or `None` once the counter is exhausted.
    pub fn issue(&mut self) -> Option<RequestId> {
        self.0.advance().map(RequestId::from_raw)
    }

    /// The most recently issued id.
    #[must_use]
    pub const fn current(self) -> Option<RequestId> {
        match self.0.current() {
            Some(value) => Some(RequestId::from_raw(value)),
            None => None,
        }
    }
}

/// A counter that mints [`Generation`]s.
#[derive(Debug, Default, Clone, Copy)]
pub struct Generations(Sequence);

impl Generations {
    /// A counter that has issued nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self(Sequence::new())
    }

    /// Issue the next generation, or `None` once the counter is exhausted.
    pub fn issue(&mut self) -> Option<Generation> {
        self.0.advance().map(Generation::from_raw)
    }

    /// The most recently issued generation.
    #[must_use]
    pub const fn current(self) -> Option<Generation> {
        match self.0.current() {
            Some(value) => Some(Generation::from_raw(value)),
            None => None,
        }
    }
}

/// A counter that mints [`Epoch`]s.
#[derive(Debug, Default, Clone, Copy)]
pub struct Epochs(Sequence);

impl Epochs {
    /// A counter that has issued nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self(Sequence::new())
    }

    /// Issue the next epoch, or `None` once the counter is exhausted.
    pub fn issue(&mut self) -> Option<Epoch> {
        self.0.advance().map(Epoch::from_raw)
    }

    /// The most recently issued epoch.
    #[must_use]
    pub const fn current(self) -> Option<Epoch> {
        match self.0.current() {
            Some(value) => Some(Epoch::from_raw(value)),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Epochs, Generations, RequestIds, Sequence};

    #[test]
    fn sequences_start_unissued_and_never_yield_zero() {
        let mut ids = RequestIds::new();
        assert_eq!(ids.current(), None);
        let first = ids.issue().expect("first id");
        assert_eq!(first.get(), 1);
        assert_eq!(ids.current(), Some(first));
        assert_eq!(ids.issue().expect("second id").get(), 2);

        let mut generations = Generations::new();
        assert_eq!(generations.issue().expect("generation").get(), 1);
        let mut epochs = Epochs::new();
        assert_eq!(epochs.issue().expect("epoch").get(), 1);
    }

    #[test]
    fn an_exhausted_sequence_refuses_to_wrap() {
        let mut sequence = Sequence::new();
        // Reaching u64::MAX by counting is not testable; assert the refusal
        // contract on the checked add directly.
        for _ in 0..3 {
            assert!(sequence.advance().is_some());
        }
        let mut exhausted = Sequence(u64::MAX);
        assert_eq!(exhausted.advance(), None);
        assert_eq!(exhausted.current(), Some(u64::MAX));
    }
}
