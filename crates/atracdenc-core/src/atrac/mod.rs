//! Shared psychoacoustics used by the ATRAC encoders.
//!
//! [`psy`] is the psychoacoustic model (masking/transient analysis driving bit
//! allocation) and [`scale`] handles scale-factor selection. Both are shared
//! across the codec-specific encoders in [`crate::at1`]/[`crate::at3`].

pub mod psy;
pub mod scale;
