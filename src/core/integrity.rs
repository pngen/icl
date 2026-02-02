use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::core::types::*;
use crate::core::ledger::IntelligenceCapitalLedger;
use crate::core::error::*;

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
            return Err(IclError::IntegrityViolation("Asset must have an owner".into()));
        }
        
        if asset.initial_value <= 0.0 {
            return Err(IclError::IntegrityViolation("Initial value must be positive".into()));
        }
        
        if asset.useful_life_months <= 0 {
            return Err(IclError::IntegrityViolation("Useful life must be positive".into()));
        }

        if let Some(cv) = asset.current_value {
            if cv < 0.0 {
                return Err(IclError::IntegrityViolation("Current value cannot be negative".into()));
            }
            if cv > asset.initial_value {
                return Err(IclError::IntegrityViolation("Current value cannot exceed initial value".into()));
            }
        }
        
        Ok(())
    }

    pub fn validate_event(&self, event: &CapitalEvent) -> IclResult<()> {
        if !self.ledger.assets.contains_key(&event.asset_id) {
            return Err(IclError::AssetNotFound(event.asset_id));
        }

        if event.event_type.is_empty() {
            return Err(IclError::IntegrityViolation("Event type is required".into()));
        }
        
        Ok(())
    }

    pub fn validate_entry(&self, entry: &LedgerEntry) -> IclResult<()> {
        if !self.ledger.assets.contains_key(&entry.asset_id) {
            return Err(IclError::AssetNotFound(entry.asset_id));
        }

        if !self.ledger.entries.is_empty() {
            let last_entry = &self.ledger.entries[self.ledger.entries.len() - 1];
            if entry.timestamp < last_entry.timestamp {
                return Err(IclError::IntegrityViolation("Ledger entries must be time-ordered".into()));
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
        for event in &self.ledger.events {
            if let Err(e) = self.validate_event(event) {
                errors.push(format!("Event {}: {}", event.event_id, e));
            }
        }

        // Check entries
        for entry in &self.ledger.entries {
            if let Err(e) = self.validate_entry(entry) {
                errors.push(format!("Entry {}: {}", entry.entry_id, e));
            }
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
                    "Cannot add event with timestamp before last recorded event".into()
                ));
            }
        }
        Ok(())
    }
    
    pub fn validate_depreciation_period(
        &self,
        asset_id: Uuid,
        start: DateTime<Utc>,
        end: DateTime<Utc>
    ) -> IclResult<()> {
        if start >= end {
            return Err(IclError::InvalidDateRange {
                start: start.to_rfc3339(),
                end: end.to_rfc3339(),
            });
        }

        let existing_depreciations: Vec<&CapitalEvent> = self.ledger.get_events_for_asset(asset_id)
            .into_iter()
            .filter(|e| e.event_type == "depreciation")
            .collect();
        
        for dep_event in existing_depreciations {
            if let (Some(existing_start), Some(existing_end)) = (
                dep_event.details.get("start_date").and_then(|v| v.as_str()),
                dep_event.details.get("end_date").and_then(|v| v.as_str())
            ) {
                if let (Ok(ex_start), Ok(ex_end)) = (
                    DateTime::parse_from_rfc3339(existing_start),
                    DateTime::parse_from_rfc3339(existing_end)
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
            proofs_by_asset.entry(proof.asset_id).or_default().push(proof);
        }

        for (asset_id, proofs) in proofs_by_asset {
            let mut sorted_proofs = proofs;
            sorted_proofs.sort_by_key(|p| p.timestamp);
            
            for i in 1..sorted_proofs.len() {
                let prev = sorted_proofs[i - 1];
                let curr = sorted_proofs[i];
                
                if let (Some(prev_hash), Some(curr_prev_hash)) = 
                    (&prev.proof_hash, &curr.previous_proof_hash) 
                {
                    if prev_hash != curr_prev_hash {
                        errors.push(format!(
                            "Proof chain break for asset {}: proof {} references wrong previous hash",
                            asset_id, curr.proof_id
                        ));
                    }
                }
            }
        }
        
        errors
    }
}