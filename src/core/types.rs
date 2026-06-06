use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
        use sha2::{Digest, Sha256};
        use std::collections::BTreeMap;

        fn canonicalize_value(value: &serde_json::Value) -> serde_json::Value {
            match value {
                serde_json::Value::Array(values) => {
                    serde_json::Value::Array(values.iter().map(canonicalize_value).collect())
                }
                serde_json::Value::Object(values) => {
                    let mut sorted = serde_json::Map::new();
                    let mut keys: Vec<&String> = values.keys().collect();
                    keys.sort();
                    for key in keys {
                        if let Some(value) = values.get(key) {
                            sorted.insert(key.clone(), canonicalize_value(value));
                        }
                    }
                    serde_json::Value::Object(sorted)
                }
                _ => value.clone(),
            }
        }

        let mut hasher = Sha256::new();
        let mut sorted_content: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (key, value) in &self.content {
            sorted_content.insert(key.clone(), canonicalize_value(value));
        }
        let content_str = serde_json::to_string(&sorted_content).unwrap_or_default();
        let event_id = self.event_id.map(|id| id.to_string()).unwrap_or_default();
        let hash_input = format!(
            "proof_id={}\nasset_id={}\nevent_id={}\ntimestamp={}\norigin={}\ncontent={}\nprevious={}",
            self.proof_id,
            self.asset_id,
            event_id,
            self.timestamp.to_rfc3339(),
            self.origin,
            content_str,
            self.previous_proof_hash.as_deref().unwrap_or("")
        );
        hasher.update(hash_input.as_bytes());
        let result = hasher.finalize();
        format!("{:x}", result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn proof_with_content(content: HashMap<String, serde_json::Value>) -> CapitalProof {
        CapitalProof {
            proof_id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            asset_id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            event_id: Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap()),
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            origin: "ICL".to_string(),
            content,
            previous_proof_hash: Some("previous".to_string()),
            proof_hash: None,
        }
    }

    #[test]
    fn proof_hash_is_deterministic_for_content_order() {
        let mut first_content = HashMap::new();
        first_content.insert("owner".to_string(), serde_json::json!("finance"));
        first_content.insert("value".to_string(), serde_json::json!(100.0));

        let mut second_content = HashMap::new();
        second_content.insert("value".to_string(), serde_json::json!(100.0));
        second_content.insert("owner".to_string(), serde_json::json!("finance"));

        assert_eq!(
            proof_with_content(first_content).compute_hash(),
            proof_with_content(second_content).compute_hash()
        );
    }

    #[test]
    fn proof_hash_binds_audit_envelope_fields() {
        let mut content = HashMap::new();
        content.insert("owner".to_string(), serde_json::json!("finance"));

        let proof = proof_with_content(content);
        let original_hash = proof.compute_hash();

        let mut changed_asset = proof.clone();
        changed_asset.asset_id =
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
        assert_ne!(original_hash, changed_asset.compute_hash());

        let mut changed_event = proof.clone();
        changed_event.event_id = None;
        assert_ne!(original_hash, changed_event.compute_hash());

        let mut changed_origin = proof.clone();
        changed_origin.origin = "other".to_string();
        assert_ne!(original_hash, changed_origin.compute_hash());
    }
}
