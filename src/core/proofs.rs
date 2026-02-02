use uuid::Uuid;
use chrono::Utc;

use crate::core::types::*;
use crate::core::ledger::IntelligenceCapitalLedger;
use crate::core::error::*;

#[derive(Debug)]
pub struct CapitalProofGenerator<'a> {
    pub ledger: &'a IntelligenceCapitalLedger,
}

impl<'a> CapitalProofGenerator<'a> {
    pub fn new(ledger: &'a IntelligenceCapitalLedger) -> Self {
        Self { ledger }
    }

    pub fn generate_asset_proof(&self, asset_id: Uuid) -> IclResult<CapitalProof> {
        let asset = self.ledger.get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;
        
        let previous_hash = self.ledger.proofs.iter()
            .filter(|p| p.asset_id == asset_id)
            .last()
            .and_then(|p| p.proof_hash.clone());
        
        let mut content: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
        content.insert("asset_id".to_string(), serde_json::Value::String(asset.asset_id.to_string()));
        content.insert("owner".to_string(), serde_json::Value::String(asset.owner.clone()));
        content.insert("initial_value".to_string(), serde_json::json!(asset.initial_value));
        content.insert("depreciation_method".to_string(), serde_json::Value::String(asset.depreciation_method.to_string()));
        content.insert("useful_life_months".to_string(), serde_json::Value::Number(serde_json::Number::from(asset.useful_life_months)));
        content.insert("status".to_string(), serde_json::Value::String(asset.status.to_string()));
        content.insert("current_value".to_string(), serde_json::json!(asset.current_value.unwrap_or_default()));

        let mut proof = CapitalProof {
            proof_id: Uuid::new_v4(),
            asset_id,
            event_id: None,
            timestamp: Utc::now(),
            origin: "ICL".to_string(),
            previous_proof_hash: previous_hash,
            content,
            proof_hash: None,
        };
        
        proof.proof_hash = Some(proof.compute_hash());
        
        Ok(proof)
    }

    pub fn generate_execution_proof(
        &self,
        asset_id: Uuid,
        event_id: Uuid
    ) -> IclResult<CapitalProof> {
        let mut proof = self.generate_asset_proof(asset_id)?;
        proof.event_id = Some(event_id);
        proof.content.insert("proof_type".to_string(), serde_json::json!("execution"));
        proof.proof_hash = Some(proof.compute_hash());
        Ok(proof)
    }

    pub fn generate_financial_outcome_proof(
        &self,
        asset_id: Uuid,
        start_date: &str,
        end_date: &str
    ) -> IclResult<CapitalProof> {
        let mut proof = self.generate_asset_proof(asset_id)?;
        proof.content.insert("proof_type".to_string(), serde_json::json!("financial_outcome"));
        proof.content.insert("period_start".to_string(), serde_json::json!(start_date));
        proof.content.insert("period_end".to_string(), serde_json::json!(end_date));
        
        let events = self.ledger.get_events_for_asset(asset_id);
        let total_depreciation: f64 = events.iter()
            .filter(|e| e.event_type == "depreciation")
            .filter_map(|e| e.details.get("amount").and_then(|v| v.as_f64()))
            .sum();
        proof.content.insert("total_depreciation".to_string(), serde_json::json!(total_depreciation));
        
        proof.proof_hash = Some(proof.compute_hash());
        Ok(proof)
    }

    pub fn reconstruct_proof(&self, proof_id: Uuid) -> Option<&CapitalProof> {
        self.ledger.proofs.iter().find(|p| p.proof_id == proof_id)
    }

    pub fn get_asset_history(&self, asset_id: Uuid) -> Vec<serde_json::Value> {
        let events = self.ledger.get_events_for_asset(asset_id);
        events.iter().map(|e| {
            serde_json::json!({
                "event_id": e.event_id.to_string(),
                "event_type": &e.event_type,
                "timestamp": e.timestamp.to_rfc3339(),
                "details": &e.details,
            })
        }).collect()
    }

    pub fn verify_proof(&self, proof: &CapitalProof) -> bool {
        if let Some(stored_hash) = &proof.proof_hash {
            let computed = proof.compute_hash();
            return stored_hash == &computed;
        }
        false
    }
}