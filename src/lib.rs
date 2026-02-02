#![deny(unsafe_code)]

// Re-export all modules and types
pub use crate::core::types::*;
pub use crate::core::ledger::*;
pub use crate::core::depreciation::*;
pub use crate::core::lifecycle::*;
pub use crate::core::integrity::*;
pub use crate::core::proofs::*;
pub use crate::core::error::*;
pub use crate::core::integration::*;

// Core modules
pub mod core {
    pub mod types;
    pub mod ledger;
    pub mod depreciation;
    pub mod lifecycle;
    pub mod integrity;
    pub mod error;
    pub mod proofs;
    pub mod integration;
}