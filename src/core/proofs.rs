use chrono::{DateTime, Utc};
use std::collections::HashMap;
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

    fn authenticate(&self, proof: &mut CapitalProof) {
        proof.proof_hash = Some(proof.compute_hash());
        proof.content.insert(
            "_ledger_authentication".to_string(),
            serde_json::json!(self.ledger.sign_proof(proof)),
        );
    }

    pub fn generate_asset_proof(&self, asset_id: Uuid) -> IclResult<CapitalProof> {
        let integrity_errors =
            crate::core::integrity::IntegrityChecker::new(self.ledger).check_all_integrity();
        if let Some(error) = integrity_errors.first() {
            return Err(IclError::IntegrityViolation(format!(
                "Cannot generate proof for an invalid ledger: {}",
                error
            )));
        }
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;

        let previous_proof = self.ledger.proofs.iter().rfind(|p| p.asset_id == asset_id);
        let previous_hash = match previous_proof {
            Some(previous) => {
                if !self.ledger.proof_hash_is_valid(previous) {
                    return Err(IclError::IntegrityViolation(
                        "Cannot extend an invalid proof chain".into(),
                    ));
                }
                previous.proof_hash.clone()
            }
            None => None,
        };
        let mut proof_timestamp = Utc::now();
        if let Some(previous) = previous_proof {
            proof_timestamp = proof_timestamp.max(previous.timestamp);
        }
        if let Some(latest_event_timestamp) = self
            .ledger
            .events
            .iter()
            .filter(|event| event.asset_id == asset_id)
            .map(|event| event.timestamp)
            .max()
        {
            proof_timestamp = proof_timestamp.max(latest_event_timestamp);
        }

        let content = asset_proof_content(asset);

        let mut proof = CapitalProof {
            proof_id: Uuid::new_v4(),
            asset_id,
            event_id: None,
            timestamp: proof_timestamp,
            origin: "ICL".to_string(),
            previous_proof_hash: previous_hash,
            content,
            proof_hash: None,
        };

        self.authenticate(&mut proof);

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
        let event = self
            .ledger
            .events
            .iter()
            .find(|event| event.event_id == event_id)
            .ok_or_else(|| IclError::InvalidEvent("Proof event does not exist".into()))?;
        proof
            .content
            .insert("event".to_string(), serde_json::json!(event));
        self.authenticate(&mut proof);
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
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;
        let total_depreciation = events
            .iter()
            .filter(|event| event.event_type == "depreciation")
            .try_fold(0.0, |total, event| -> IclResult<f64> {
                Ok(total + depreciation_amount_for_period(asset, event, period_start, period_end)?)
            })?;
        proof.content.insert(
            "total_depreciation".to_string(),
            serde_json::json!(total_depreciation),
        );

        self.authenticate(&mut proof);
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
        if !self.ledger.storage_is_untampered()
            || !self.ledger.proof_hash_is_valid(proof)
            || proof.origin != "ICL"
        {
            return false;
        }

        let asset = match self.ledger.get_asset(proof.asset_id) {
            Some(asset) => asset,
            None => return false,
        };
        if proof.content.get("asset_id") != Some(&serde_json::json!(proof.asset_id.to_string())) {
            return false;
        }

        let stored_positions: Vec<usize> = self
            .ledger
            .proofs
            .iter()
            .enumerate()
            .filter_map(|(index, stored)| {
                if stored.proof_id == proof.proof_id {
                    Some(index)
                } else {
                    None
                }
            })
            .collect();
        if stored_positions.len() > 1 {
            return false;
        }
        let stored_position = stored_positions.first().copied();
        if let Some(index) = stored_position {
            if &self.ledger.proofs[index] != proof {
                return false;
            }
        }

        let prefix_end = stored_position.unwrap_or(self.ledger.proofs.len());
        let mut previous_hash: Option<&str> = None;
        let mut previous_timestamp = None;
        for candidate in self.ledger.proofs[..prefix_end]
            .iter()
            .filter(|candidate| candidate.asset_id == proof.asset_id)
        {
            if !self.ledger.proof_hash_is_valid(candidate)
                || candidate.previous_proof_hash.as_deref() != previous_hash
                || previous_timestamp.is_some_and(|timestamp| candidate.timestamp < timestamp)
            {
                return false;
            }
            previous_hash = candidate.proof_hash.as_deref();
            previous_timestamp = Some(candidate.timestamp);
        }
        if proof.previous_proof_hash.as_deref() != previous_hash
            || previous_timestamp.is_some_and(|timestamp| proof.timestamp < timestamp)
        {
            return false;
        }

        let is_latest = stored_position.is_none_or(|index| {
            !self.ledger.proofs[index + 1..]
                .iter()
                .any(|candidate| candidate.asset_id == proof.asset_id)
        });
        let last_event_timestamp = self
            .ledger
            .events
            .iter()
            .filter(|event| event.asset_id == proof.asset_id)
            .map(|event| event.timestamp)
            .next_back();
        let represents_current_state =
            is_latest && last_event_timestamp.is_none_or(|timestamp| proof.timestamp >= timestamp);
        if represents_current_state && !asset_content_matches(&proof.content, asset) {
            return false;
        }

        if let Some(event_id) = proof.event_id {
            let event = match self
                .ledger
                .events
                .iter()
                .find(|event| event.event_id == event_id && event.asset_id == proof.asset_id)
            {
                Some(event) => event,
                None => return false,
            };
            if proof.content.get("event") != Some(&serde_json::json!(event)) {
                return false;
            }
        } else if proof.content.contains_key("event") {
            return false;
        }

        match proof
            .content
            .get("proof_type")
            .and_then(|value| value.as_str())
        {
            None => proof.content.len() == 8 + usize::from(proof.event_id.is_some()),
            Some("execution") => proof.event_id.is_some() && proof.content.len() == 10,
            Some("financial_outcome") => {
                proof.event_id.is_none()
                    && proof.content.len() == 12
                    && financial_content_is_valid(self.ledger, proof)
            }
            Some(_) => false,
        }
    }
}

pub(crate) fn asset_proof_content(asset: &IntelligenceAsset) -> HashMap<String, serde_json::Value> {
    let mut content = HashMap::new();
    content.insert(
        "asset_id".to_string(),
        serde_json::json!(asset.asset_id.to_string()),
    );
    content.insert("owner".to_string(), serde_json::json!(&asset.owner));
    content.insert(
        "initial_value".to_string(),
        serde_json::json!(asset.initial_value),
    );
    content.insert(
        "depreciation_method".to_string(),
        serde_json::json!(asset.depreciation_method.to_string()),
    );
    content.insert(
        "useful_life_months".to_string(),
        serde_json::json!(asset.useful_life_months),
    );
    content.insert(
        "status".to_string(),
        serde_json::json!(asset.status.to_string()),
    );
    content.insert(
        "current_value".to_string(),
        serde_json::json!(asset.current_value.unwrap_or_default()),
    );
    content
}

fn asset_content_matches(
    content: &HashMap<String, serde_json::Value>,
    asset: &IntelligenceAsset,
) -> bool {
    let expected = asset_proof_content(asset);
    expected
        .iter()
        .all(|(key, value)| content.get(key) == Some(value))
}

fn depreciation_amount_for_period(
    asset: &IntelligenceAsset,
    event: &CapitalEvent,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> IclResult<f64> {
    let recorded_amount = IntelligenceCapitalLedger::event_amount(event)?;
    let event_start = event
        .details
        .get("start_date")
        .and_then(|value| value.as_str())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .ok_or_else(|| IclError::InvalidEvent("Invalid depreciation start_date".into()))?;
    let event_end = event
        .details
        .get("end_date")
        .and_then(|value| value.as_str())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .ok_or_else(|| IclError::InvalidEvent("Invalid depreciation end_date".into()))?;
    if event_start >= event_end {
        return Err(IclError::InvalidEvent(
            "Depreciation period must be positive".into(),
        ));
    }

    let overlap_start = std::cmp::max(event_start, period_start);
    let overlap_end = std::cmp::min(event_end, period_end);
    if overlap_start >= overlap_end {
        return Ok(0.0);
    }
    let previous_value = event
        .details
        .get("previous_value")
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite())
        .ok_or_else(|| IclError::InvalidEvent("Invalid depreciation previous_value".into()))?;
    let salvage_value = event
        .details
        .get("salvage_value")
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite())
        .ok_or_else(|| IclError::InvalidEvent("Invalid depreciation salvage_value".into()))?;
    let rate_multiplier = event
        .details
        .get("rate_multiplier")
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite())
        .ok_or_else(|| IclError::InvalidEvent("Invalid depreciation rate_multiplier".into()))?;
    let mut schedule_asset = asset.clone();
    schedule_asset.current_value = Some(previous_value);
    let full_amount = crate::core::depreciation::calculate_depreciation(
        &schedule_asset,
        event_start,
        event_end,
        salvage_value,
        rate_multiplier,
    )?
    .0;
    if (full_amount - recorded_amount).abs() > 1e-9 {
        return Err(IclError::IntegrityViolation(
            "Recorded depreciation does not match its schedule".into(),
        ));
    }
    let through_start = if overlap_start == event_start {
        0.0
    } else {
        crate::core::depreciation::calculate_depreciation(
            &schedule_asset,
            event_start,
            overlap_start,
            salvage_value,
            rate_multiplier,
        )?
        .0
    };
    let through_end = crate::core::depreciation::calculate_depreciation(
        &schedule_asset,
        event_start,
        overlap_end,
        salvage_value,
        rate_multiplier,
    )?
    .0;
    let amount = through_end - through_start;
    if amount.is_finite() && amount >= -1e-9 {
        Ok(amount.max(0.0))
    } else {
        Err(IclError::IntegrityViolation(
            "Depreciation schedule is not monotonic".into(),
        ))
    }
}

fn financial_content_is_valid(ledger: &IntelligenceCapitalLedger, proof: &CapitalProof) -> bool {
    let start = proof
        .content
        .get("period_start")
        .and_then(|value| value.as_str())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    let end = proof
        .content
        .get("period_end")
        .and_then(|value| value.as_str())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    let (start, end) = match (start, end) {
        (Some(start), Some(end)) if start < end => (start, end),
        _ => return false,
    };
    let expected = ledger
        .get_events_for_asset(proof.asset_id)
        .iter()
        .filter(|event| event.event_type == "depreciation")
        .try_fold(0.0, |total, event| {
            let asset = ledger
                .get_asset(proof.asset_id)
                .ok_or(IclError::AssetNotFound(proof.asset_id))?;
            depreciation_amount_for_period(asset, event, start, end).map(|amount| total + amount)
        });
    match expected {
        Ok(expected) => {
            proof
                .content
                .get("total_depreciation")
                .and_then(|value| value.as_f64())
                == Some(expected)
        }
        Err(_) => false,
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
            .create_asset_at(
                asset_id,
                "finance".to_string(),
                1000.0,
                DepreciationMethod::Linear,
                10,
                Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
            )
            .unwrap();
        asset_id
    }

    fn depreciation_event(
        asset_id: Uuid,
        timestamp: DateTime<Utc>,
        previous_value: f64,
        amount: f64,
        start_date: &str,
        end_date: &str,
    ) -> CapitalEvent {
        let mut details = HashMap::new();
        details.insert("amount".to_string(), serde_json::json!(amount));
        details.insert("start_date".to_string(), serde_json::json!(start_date));
        details.insert("end_date".to_string(), serde_json::json!(end_date));
        details.insert(
            "previous_value".to_string(),
            serde_json::json!(previous_value),
        );
        details.insert(
            "new_value".to_string(),
            serde_json::json!(previous_value - amount),
        );
        details.insert("salvage_value".to_string(), serde_json::json!(0.0));
        details.insert("rate_multiplier".to_string(), serde_json::json!(2.0));

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
            Utc::now(),
            1000.0,
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
                Utc::now(),
                1000.0,
                100.0,
                "2024-01-01T00:00:00Z",
                "2024-02-01T00:00:00Z",
            ))
            .unwrap();
        ledger
            .record_event(depreciation_event(
                asset_id,
                Utc::now(),
                900.0,
                100.0,
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

    #[test]
    fn proof_verification_rejects_fabrication_and_event_tampering() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        let event = depreciation_event(
            asset_id,
            Utc::now(),
            1000.0,
            100.0,
            "2024-01-01T00:00:00Z",
            "2024-02-01T00:00:00Z",
        );
        let event_id = event.event_id;
        ledger.record_event(event).unwrap();

        let proof = CapitalProofGenerator::new(&ledger)
            .generate_execution_proof(asset_id, event_id)
            .unwrap();
        assert_eq!(
            proof.proof_hash.as_deref(),
            Some(proof.compute_hash().as_str())
        );
        assert!(proof.timestamp >= ledger.events.last().unwrap().timestamp);
        assert!(CapitalProofGenerator::new(&ledger).verify_proof(&proof));

        ledger
            .events
            .iter_mut()
            .find(|event| event.event_id == event_id)
            .unwrap()
            .details
            .insert("amount".to_string(), serde_json::json!(999.0));
        assert!(!CapitalProofGenerator::new(&ledger).verify_proof(&proof));

        let mut fabricated = proof.clone();
        fabricated.proof_id = Uuid::new_v4();
        fabricated.asset_id = Uuid::new_v4();
        fabricated.content.insert(
            "asset_id".to_string(),
            serde_json::json!(fabricated.asset_id.to_string()),
        );
        fabricated.proof_hash = Some(fabricated.compute_hash());
        assert!(!CapitalProofGenerator::new(&ledger).verify_proof(&fabricated));
    }

    #[test]
    fn financial_outcome_prorates_partial_period_and_rejects_malformed_records() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 1, 0, 1, 40).unwrap();
        let recorded_amount = crate::core::depreciation::calculate_depreciation(
            ledger.get_asset(asset_id).unwrap(),
            start,
            end,
            0.0,
            2.0,
        )
        .unwrap()
        .0;
        ledger
            .record_event(depreciation_event(
                asset_id,
                Utc::now(),
                1000.0,
                recorded_amount,
                "2024-01-01T00:00:00Z",
                "2024-01-01T00:01:40Z",
            ))
            .unwrap();

        let proof = CapitalProofGenerator::new(&ledger)
            .generate_financial_outcome_proof(
                asset_id,
                "2024-01-01T00:00:25Z",
                "2024-01-01T00:01:15Z",
            )
            .unwrap();
        let partial_amount = proof
            .content
            .get("total_depreciation")
            .and_then(|value| value.as_f64())
            .unwrap();
        assert!((partial_amount - recorded_amount / 2.0).abs() < 1e-9);

        ledger
            .events
            .iter_mut()
            .find(|event| event.event_type == "depreciation")
            .unwrap()
            .details
            .insert("start_date".to_string(), serde_json::json!("invalid"));
        assert!(CapitalProofGenerator::new(&ledger)
            .generate_financial_outcome_proof(
                asset_id,
                "2024-01-01T00:00:00Z",
                "2024-01-02T00:00:00Z",
            )
            .is_err());
    }
}
