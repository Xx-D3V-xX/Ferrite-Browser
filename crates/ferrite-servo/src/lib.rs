// Servo embedding crate — implementation follows in subsequent blocks
pub mod bundle;
#[cfg(feature = "servo")]
mod gpu_context;
pub mod diag;
pub mod permissions;
pub mod session;
pub mod shell;
