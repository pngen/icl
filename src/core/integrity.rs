use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::core::error::*;
use crate::core::ledger::IntelligenceCapitalLedger;
use crate::core::types::*;

#[derive(Debug)]
pub struct IntegrityChecker<'a> {
    pub ledger: &'a IntelligenceCapitalLedger,
}

impl<'a> IntegrityChecker<'a> {
    pub fn new(ledger: &'a IntelligenceCapitalLedger) -> Self {
        Self { ledger }
    }

    pub fn validate_asset(&self, asset: &IntelligenceAsset) -> IclResult<()> {
        if asset.owner.is_empty() {
            return Err(IclError::IntegrityViolation(
                "Asset must have an owner".into(),
            ));
        }

        if !asset.initial_value.is_finite() || asset.initial_value <= 0.0 {
            return Err(IclError::IntegrityViolation(
                "Initial value must be positive".into(),
            ));
        }

        if asset.useful_life_months <= 0 {
            return Err(IclError::IntegrityViolation(
                "Useful life must be positive".into(),
            ));
        }

        if let Some(cv) = asset.current_value {
            if !cv.is_finite() || cv < 0.0 {
                return Err(IclError::IntegrityViolation(
                    "Current value cannot be negative".into(),
                ));
            }
            if cv > asset.initial_value {
                return Err(IclError::IntegrityViolation(
                    "Current value cannot exceed initial value".into(),
                ));
            }
        }

        Ok(())
    }

    pub fn validate_event(&self, event: &CapitalEvent) -> IclResult<()> {
        if !self.ledger.assets.contains_key(&event.asset_id) {
            return Err(IclError::AssetNotFound(event.asset_id));
        }

        if event.event_type.is_empty() {
            return Err(IclError::IntegrityViolation(
                "Event type is required".into(),
            ));
        }

        if let Some(value) = event.details.get("amount") {
            let amount = value.as_f64().ok_or_else(|| {
                IclError::IntegrityViolation("Event amount must be numeric".into())
            })?;
            if !amount.is_finite() {
                return Err(IclError::IntegrityViolation(
                    "Event amount must be finite".into(),
                ));
            }
        }

        Ok(())
    }

    pub fn validate_entry(&self, entry: &LedgerEntry) -> IclResult<()> {
        if !self.ledger.assets.contains_key(&entry.asset_id) {
            return Err(IclError::AssetNotFound(entry.asset_id));
        }

        if !entry.amount.is_finite() {
            return Err(IclError::IntegrityViolation(
                "Ledger entry amount must be finite".into(),
            ));
        }

        if !self.ledger.entries.is_empty() {
            let last_entry = &self.ledger.entries[self.ledger.entries.len() - 1];
            if entry.timestamp < last_entry.timestamp {
                return Err(IclError::IntegrityViolation(
                    "Ledger entries must be time-ordered".into(),
                ));
            }
        }

        Ok(())
    }

    pub fn check_all_integrity(&self) -> Vec<String> {
        let mut errors = Vec::new();

        for asset in self.ledger.assets.values() {
            if let Err(e) = self.validate_asset(asset) {
                errors.push(format!("Asset {}: {}", asset.asset_id, e));
            }
        }

        // Check events
        let mut seen_event_ids = std::collections::HashSet::new();
        let mut last_event_timestamp = None;
        for event in &self.ledger.events {
            if !seen_event_ids.insert(event.event_id) {
                errors.push(format!("Event {}: duplicate event id", event.event_id));
            }
            if let Some(last_timestamp) = last_event_timestamp {
                if event.timestamp < last_timestamp {
                    errors.push(format!(
                        "Event {}: events must be time-ordered",
                        event.event_id
                    ));
                }
            }
            last_event_timestamp = Some(event.timestamp);

            if let Err(e) = self.validate_event(event) {
                errors.push(format!("Event {}: {}", event.event_id, e));
            }
        }

        // Check entries
        let mut last_entry_timestamp = None;
        for entry in &self.ledger.entries {
            if !self.ledger.assets.contains_key(&entry.asset_id) {
                errors.push(format!(
                    "Entry {}: {}",
                    entry.entry_id,
                    IclError::AssetNotFound(entry.asset_id)
                ));
            }
            if !entry.amount.is_finite() {
                errors.push(format!(
                    "Entry {}: ledger entry amount must be finite",
                    entry.entry_id
                ));
            }
            if let Some(last_timestamp) = last_entry_timestamp {
                if entry.timestamp < last_timestamp {
                    errors.push(format!(
                        "Entry {}: ledger entries must be time-ordered",
                        entry.entry_id
                    ));
                }
            }
            last_entry_timestamp = Some(entry.timestamp);
        }

        // Verify proof chain integrity
        let proof_errors = self.verify_proof_chain();
        errors.extend(proof_errors);

        errors
    }

    pub fn ensure_no_retroactive_modification(&self, new_event: &CapitalEvent) -> IclResult<()> {
        if let Some(last_event) = self.ledger.events.last() {
            if new_event.timestamp < last_event.timestamp {
                return Err(IclError::IntegrityViolation(
                    "Cannot add event with timestamp before last recorded event".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn validate_depreciation_period(
        &self,
        asset_id: Uuid,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> IclResult<()> {
        if start >= end {
            return Err(IclError::InvalidDateRange {
                start: start.to_rfc3339(),
                end: end.to_rfc3339(),
            });
        }

        let existing_depreciations: Vec<&CapitalEvent> = self
            .ledger
            .get_events_for_asset(asset_id)
            .into_iter()
            .filter(|e| e.event_type == "depreciation")
            .collect();

        for dep_event in existing_depreciations {
            if let (Some(existing_start), Some(existing_end)) = (
                dep_event.details.get("start_date").and_then(|v| v.as_str()),
                dep_event.details.get("end_date").and_then(|v| v.as_str()),
            ) {
                if let (Ok(ex_start), Ok(ex_end)) = (
                    DateTime::parse_from_rfc3339(existing_start),
                    DateTime::parse_from_rfc3339(existing_end),
                ) {
                    let ex_start = ex_start.with_timezone(&Utc);
                    let ex_end = ex_end.with_timezone(&Utc);

                    // Check for overlap: periods overlap if start < ex_end AND end > ex_start
                    if start < ex_end && end > ex_start {
                        return Err(IclError::OverlappingDepreciation);
                    }
                }
            }
        }

        Ok(())
    }

    pub fn verify_proof_chain(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut proofs_by_asset: std::collections::HashMap<Uuid, Vec<&CapitalProof>> =
            std::collections::HashMap::new();

        for proof in &self.ledger.proofs {
            match &proof.proof_hash {
                Some(stored_hash) if stored_hash == &proof.compute_hash() => {}
                Some(_) => errors.push(format!(
                    "Proof {} has an invalid proof hash",
                    proof.proof_id
                )),
                None => errors.push(format!("Proof {} is missing a proof hash", proof.proof_id)),
            }

            if let Some(event_id) = proof.event_id {
                let event_belongs_to_asset =
                    self.ledger.events.iter().any(|event| {
                        event.event_id == event_id && event.asset_id == proof.asset_id
                    });
                if !event_belongs_to_asset {
                    errors.push(format!(
                        "Proof {} references an event that does not belong to asset {}",
                        proof.proof_id, proof.asset_id
                    ));
                }
            }

            proofs_by_asset
                .entry(proof.asset_id)
                .or_default()
                .push(proof);
        }

        for (asset_id, proofs) in proofs_by_asset {
            let mut sorted_proofs = proofs;
            sorted_proofs.sort_by_key(|p| p.timestamp);

            if let Some(first) = sorted_proofs.first() {
                if first.previous_proof_hash.is_some() {
                    errors.push(format!(
                        "Proof chain break for asset {}: first proof {} has a previous hash",
                        asset_id, first.proof_id
                    ));
                }
            }

            for i in 1..sorted_proofs.len() {
                let prev = sorted_proofs[i - 1];
                let curr = sorted_proofs[i];

                if prev.proof_hash.as_deref() != curr.previous_proof_hash.as_deref() {
                    errors.push(format!(
                        "Proof chain break for asset {}: proof {} references wrong previous hash",
                        asset_id, curr.proof_id
                    ));
                }
            }
        }

        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
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

    fn event_at(asset_id: Uuid, timestamp: DateTime<Utc>) -> CapitalEvent {
        let mut details = HashMap::new();
        details.insert("amount".to_string(), serde_json::json!(100.0));

        CapitalEvent {
            event_id: Uuid::new_v4(),
            asset_id,
            event_type: "utilization".to_string(),
            timestamp,
            details,
        }
    }

    #[test]
    fn whole_ledger_integrity_allows_valid_time_ordered_entries() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        ledger
            .record_event(event_at(
                asset_id,
                Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            ))
            .unwrap();
        ledger
            .record_event(event_at(
                asset_id,
                Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
            ))
            .unwrap();

        let checker = IntegrityChecker::new(&ledger);
        assert!(checker.check_all_integrity().is_empty());
    }

    #[test]
    fn whole_ledger_integrity_detects_duplicate_and_retroactive_events() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        let first = event_at(asset_id, Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap());
        let mut second = event_at(asset_id, Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap());
        second.event_id = first.event_id;
        ledger.events.push(first.clone());
        ledger.events.push(second);

        let checker = IntegrityChecker::new(&ledger);
        let errors = checker.check_all_integrity();
        assert!(errors
            .iter()
            .any(|error| error.contains("duplicate event id")));
        assert!(errors.iter().any(|error| error.contains("time-ordered")));
    }

    #[test]
    fn whole_ledger_integrity_detects_tampered_proof() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        ledger.generate_proof(asset_id, None).unwrap();
        ledger.proofs[0].origin = "tampered".to_string();

        let checker = IntegrityChecker::new(&ledger);
        let errors = checker.check_all_integrity();
        assert!(errors
            .iter()
            .any(|error| error.contains("invalid proof hash")));
    }
}
