use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
 
/// Status of an intelligence asset in its lifecycle
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AssetStatus {
    Active,
    Depreciated,
    Retired,
}

impl std::fmt::Display for AssetStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetStatus::Active => write!(f, "Active"),
            AssetStatus::Depreciated => write!(f, "Depreciated"),
            AssetStatus::Retired => write!(f, "Retired"),
        }
    }
}

/// Method used to calculate depreciation over asset lifetime
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DepreciationMethod {
    Linear,
    DecliningBalance,
}

impl std::fmt::Display for DepreciationMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DepreciationMethod::Linear => write!(f, "Linear"),
            DepreciationMethod::DecliningBalance => write!(f, "DecliningBalance"),
        }
    }
}

/// Account types for double-entry journal entries
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AccountType {
    Asset,
    AccumulatedDepreciation,
    DepreciationExpense,
}

impl std::fmt::Display for AccountType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountType::Asset => write!(f, "Asset"),
            AccountType::AccumulatedDepreciation => write!(f, "AccumulatedDepreciation"),
            AccountType::DepreciationExpense => write!(f, "DepreciationExpense"),
        }
    }
}

/// A capitalized intelligence asset with ownership and depreciation rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceAsset {
    pub asset_id: uuid::Uuid,
    pub owner: String,
    pub initial_value: f64,
    pub depreciation_method: DepreciationMethod,
    pub useful_life_months: i32,
    pub created_at: DateTime<Utc>,
    pub status: AssetStatus,
    pub current_value: Option<f64>,
}

/// A discrete economic event affecting intelligence capital
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapitalEvent {
    pub event_id: uuid::Uuid,
    pub asset_id: uuid::Uuid,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub details: HashMap<String, serde_json::Value>,
}

/// Immutable ledger entry derived from capital events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub entry_id: uuid::Uuid,
    pub event_id: uuid::Uuid,
    pub asset_id: uuid::Uuid,
    pub timestamp: DateTime<Utc>,
    pub amount: f64,
    pub description: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Double-entry accounting journal entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub entry_id: uuid::Uuid,
    pub event_id: uuid::Uuid,
    pub timestamp: DateTime<Utc>,
    pub debit_account: AccountType,
    pub credit_account: AccountType,
    pub amount: f64,
    pub description: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Machine-verifiable proof of capital state for audit purposes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapitalProof {
    pub proof_id: uuid::Uuid,
    pub asset_id: uuid::Uuid,
    pub event_id: Option<uuid::Uuid>,
    pub timestamp: DateTime<Utc>,
    pub origin: String,
    pub content: HashMap<String, serde_json::Value>,
    pub previous_proof_hash: Option<String>,
    pub proof_hash: Option<String>,
}

impl CapitalProof {
    pub fn compute_hash(&self) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        let content_str = serde_json::to_string(&self.content).unwrap_or_default();
        let hash_input = format!(
            "{}{}{}{}",
            self.proof_id,
            self.timestamp.timestamp(),
            content_str,
            self.previous_proof_hash.as_ref().unwrap_or(&String::new())
        );
        hasher.update(hash_input.as_bytes());
        let result = hasher.finalize();
        format!("{:x}", result)
    }
}