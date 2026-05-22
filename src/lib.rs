#![deny(unsafe_code)]

// Re-export all modules and types
pub use crate::core::depreciation::*;
pub use crate::core::error::*;
pub use crate::core::integration::*;
pub use crate::core::integrity::*;
pub use crate::core::ledger::*;
pub use crate::core::lifecycle::*;
pub use crate::core::proofs::*;
pub use crate::core::types::*;

// Core modules
pub mod core {
    pub mod depreciation;
    pub mod error;
    pub mod integration;
    pub mod integrity;
    pub mod ledger;
    pub mod lifecycle;
    pub mod proofs;
    pub mod types;
}
