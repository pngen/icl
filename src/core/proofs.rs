use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::core::error::*;
use crate::core::ledger::IntelligenceCapitalLedger;
use crate::core::types::*;

#[derive(Debug)]
pub struct CapitalProofGenerator<'a> {
    pub ledger: &'a IntelligenceCapitalLedger,
}

impl<'a> CapitalProofGenerator<'a> {
    pub fn new(ledger: &'a IntelligenceCapitalLedger) -> Self {
        Self { ledger }
    }

    pub fn generate_asset_proof(&self, asset_id: Uuid) -> IclResult<CapitalProof> {
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;

        let previous_hash = self
            .ledger
            .proofs
            .iter()
            .filter(|p| p.asset_id == asset_id)
            .last()
            .and_then(|p| p.proof_hash.clone());

        let mut content: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        content.insert(
            "asset_id".to_string(),
            serde_json::Value::String(asset.asset_id.to_string()),
        );
        content.insert(
            "owner".to_string(),
            serde_json::Value::String(asset.owner.clone()),
        );
        content.insert(
            "initial_value".to_string(),
            serde_json::json!(asset.initial_value),
        );
        content.insert(
            "depreciation_method".to_string(),
            serde_json::Value::String(asset.depreciation_method.to_string()),
        );
        content.insert(
            "useful_life_months".to_string(),
            serde_json::Value::Number(serde_json::Number::from(asset.useful_life_months)),
        );
        content.insert(
            "status".to_string(),
            serde_json::Value::String(asset.status.to_string()),
        );
        content.insert(
            "current_value".to_string(),
            serde_json::json!(asset.current_value.unwrap_or_default()),
        );

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
        event_id: Uuid,
    ) -> IclResult<CapitalProof> {
        let event_belongs_to_asset = self
            .ledger
            .events
            .iter()
            .any(|event| event.event_id == event_id && event.asset_id == asset_id);
        if !event_belongs_to_asset {
            return Err(IclError::InvalidEvent(
                "Proof event must belong to the requested asset".into(),
            ));
        }

        let mut proof = self.generate_asset_proof(asset_id)?;
        proof.event_id = Some(event_id);
        proof
            .content
            .insert("proof_type".to_string(), serde_json::json!("execution"));
        proof.proof_hash = Some(proof.compute_hash());
        Ok(proof)
    }

    pub fn generate_financial_outcome_proof(
        &self,
        asset_id: Uuid,
        start_date: &str,
        end_date: &str,
    ) -> IclResult<CapitalProof> {
        let period_start = DateTime::parse_from_rfc3339(start_date)
            .map_err(|_| IclError::InvalidDateRange {
                start: start_date.to_string(),
                end: end_date.to_string(),
            })?
            .with_timezone(&Utc);
        let period_end = DateTime::parse_from_rfc3339(end_date)
            .map_err(|_| IclError::InvalidDateRange {
                start: start_date.to_string(),
                end: end_date.to_string(),
            })?
            .with_timezone(&Utc);
        if period_start >= period_end {
            return Err(IclError::InvalidDateRange {
                start: start_date.to_string(),
                end: end_date.to_string(),
            });
        }

        let mut proof = self.generate_asset_proof(asset_id)?;
        proof.content.insert(
            "proof_type".to_string(),
            serde_json::json!("financial_outcome"),
        );
        proof
            .content
            .insert("period_start".to_string(), serde_json::json!(start_date));
        proof
            .content
            .insert("period_end".to_string(), serde_json::json!(end_date));

        let events = self.ledger.get_events_for_asset(asset_id);
        let total_depreciation: f64 = events
            .iter()
            .filter(|e| e.event_type == "depreciation")
            .filter(|e| depreciation_event_overlaps_period(e, period_start, period_end))
            .filter_map(|e| e.details.get("amount").and_then(|v| v.as_f64()))
            .sum();
        proof.content.insert(
            "total_depreciation".to_string(),
            serde_json::json!(total_depreciation),
        );

        proof.proof_hash = Some(proof.compute_hash());
        Ok(proof)
    }

    pub fn reconstruct_proof(&self, proof_id: Uuid) -> Option<&CapitalProof> {
        self.ledger.proofs.iter().find(|p| p.proof_id == proof_id)
    }

    pub fn get_asset_history(&self, asset_id: Uuid) -> Vec<serde_json::Value> {
        let events = self.ledger.get_events_for_asset(asset_id);
        events
            .iter()
            .map(|e| {
                serde_json::json!({
                    "event_id": e.event_id.to_string(),
                    "event_type": &e.event_type,
                    "timestamp": e.timestamp.to_rfc3339(),
                    "details": &e.details,
                })
            })
            .collect()
    }

    pub fn verify_proof(&self, proof: &CapitalProof) -> bool {
        if let Some(stored_hash) = &proof.proof_hash {
            let computed = proof.compute_hash();
            return stored_hash == &computed;
        }
        false
    }
}

fn depreciation_event_overlaps_period(
    event: &CapitalEvent,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> bool {
    let event_start = event
        .details
        .get("start_date")
        .and_then(|value| value.as_str())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    let event_end = event
        .details
        .get("end_date")
        .and_then(|value| value.as_str())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));

    match (event_start, event_end) {
        (Some(start), Some(end)) => start < period_end && end > period_start,
        _ => event.timestamp >= period_start && event.timestamp < period_end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ledger::IntelligenceCapitalLedger;
    use chrono::TimeZone;
    use std::collections::HashMap;

    fn create_asset(ledger: &mut IntelligenceCapitalLedger) -> Uuid {
        let asset_id = Uuid::new_v4();
        ledger
            .create_asset(
                asset_id,
                "finance".to_string(),
                1000.0,
                DepreciationMethod::Linear,
                12,
            )
            .unwrap();
        asset_id
    }

    fn depreciation_event(
        asset_id: Uuid,
        timestamp: DateTime<Utc>,
        amount: f64,
        start_date: &str,
        end_date: &str,
    ) -> CapitalEvent {
        let mut details = HashMap::new();
        details.insert("amount".to_string(), serde_json::json!(amount));
        details.insert("start_date".to_string(), serde_json::json!(start_date));
        details.insert("end_date".to_string(), serde_json::json!(end_date));

        CapitalEvent {
            event_id: Uuid::new_v4(),
            asset_id,
            event_type: "depreciation".to_string(),
            timestamp,
            details,
        }
    }

    #[test]
    fn execution_proof_rejects_event_from_another_asset() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let first_asset = create_asset(&mut ledger);
        let second_asset = create_asset(&mut ledger);
        let event = depreciation_event(
            second_asset,
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
            100.0,
            "2024-01-01T00:00:00Z",
            "2024-02-01T00:00:00Z",
        );
        let event_id = event.event_id;
        ledger.record_event(event).unwrap();

        let generator = CapitalProofGenerator::new(&ledger);
        assert!(generator
            .generate_execution_proof(first_asset, event_id)
            .is_err());
    }

    #[test]
    fn financial_outcome_proof_sums_only_requested_period() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        ledger
            .record_event(depreciation_event(
                asset_id,
                Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
                100.0,
                "2024-01-01T00:00:00Z",
                "2024-02-01T00:00:00Z",
            ))
            .unwrap();
        ledger
            .record_event(depreciation_event(
                asset_id,
                Utc.with_ymd_and_hms(2024, 3, 2, 0, 0, 0).unwrap(),
                300.0,
                "2024-03-01T00:00:00Z",
                "2024-04-01T00:00:00Z",
            ))
            .unwrap();

        let generator = CapitalProofGenerator::new(&ledger);
        let proof = generator
            .generate_financial_outcome_proof(
                asset_id,
                "2024-01-01T00:00:00Z",
                "2024-02-01T00:00:00Z",
            )
            .unwrap();

        assert_eq!(
            proof
                .content
                .get("total_depreciation")
                .and_then(|value| value.as_f64()),
            Some(100.0)
        );
    }
}
