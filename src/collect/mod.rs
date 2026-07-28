//! Readers for `/proc` and `/sys`.
//!
//! Everything here is a plain blocking file read that returns a best-effort
//! value. Nothing in this module panics or propagates errors: a missing sensor
//! should blank one field, never stop the render loop.

pub mod battery;
pub mod cpu;
