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

        IntelligenceCapitalLedger::event_amount(event).map_err(|error| {
            IclError::IntegrityViolation(format!("Invalid event amount: {}", error))
        })?;

        if event.event_type == "depreciation" {
            let start = event
                .details
                .get("start_date")
                .and_then(|value| value.as_str())
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok());
            let end = event
                .details
                .get("end_date")
                .and_then(|value| value.as_str())
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok());
            if !matches!((start, end), (Some(start), Some(end)) if start < end) {
                return Err(IclError::IntegrityViolation(
                    "Depreciation event must contain a valid positive period".into(),
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

        let event = self
            .ledger
            .events
            .iter()
            .find(|event| event.event_id == entry.event_id)
            .ok_or_else(|| IclError::IntegrityViolation("Ledger entry has no event".into()))?;
        let expected_amount = IntelligenceCapitalLedger::event_amount(event).map_err(|error| {
            IclError::IntegrityViolation(format!("Invalid event amount: {}", error))
        })?;
        if entry.asset_id != event.asset_id
            || entry.timestamp != event.timestamp
            || entry.description != event.event_type
            || entry.metadata != event.details
            || entry.amount != expected_amount
        {
            return Err(IclError::IntegrityViolation(
                "Ledger entry does not match its source event".into(),
            ));
        }

        Ok(())
    }

    pub fn check_all_integrity(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if !self.ledger.storage_is_untampered() {
            errors.push("Ledger storage commitment does not match canonical records".to_string());
        }

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

        // Every canonical event must have exactly one matching derived ledger entry.
        let mut seen_entry_ids = std::collections::HashSet::new();
        let mut last_entry_timestamp = None;
        for entry in &self.ledger.entries {
            if !seen_entry_ids.insert(entry.entry_id) {
                errors.push(format!("Entry {}: duplicate entry id", entry.entry_id));
            }
            if let Err(error) = self.validate_entry(entry) {
                errors.push(format!("Entry {}: {}", entry.entry_id, error));
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
        for event in &self.ledger.events {
            let matching_entries = self
                .ledger
                .entries
                .iter()
                .filter(|entry| entry.event_id == event.event_id)
                .count();
            if matching_entries != 1 {
                errors.push(format!(
                    "Event {}: expected one derived ledger entry, found {}",
                    event.event_id, matching_entries
                ));
            }
        }

        let mut seen_journal_ids = std::collections::HashSet::new();
        for journal in &self.ledger.journal_entries {
            if !seen_journal_ids.insert(journal.entry_id) {
                errors.push(format!(
                    "Journal entry {}: duplicate entry id",
                    journal.entry_id
                ));
            }
            let event = match self
                .ledger
                .events
                .iter()
                .find(|event| event.event_id == journal.event_id)
            {
                Some(event) => event,
                None => {
                    errors.push(format!(
                        "Journal entry {}: references an unknown event",
                        journal.entry_id
                    ));
                    continue;
                }
            };
            if !journal.amount.is_finite()
                || journal.amount <= 0.0
                || journal.debit_account == journal.credit_account
                || journal.timestamp < event.timestamp
                || journal.description.trim().is_empty()
            {
                errors.push(format!(
                    "Journal entry {}: invalid accounting fields",
                    journal.entry_id
                ));
            }
            let valid_pair = match event.event_type.as_str() {
                "capitalization" => {
                    journal.debit_account == AccountType::Asset
                        && journal.credit_account == AccountType::CapitalizationSource
                }
                "depreciation" => {
                    journal.debit_account == AccountType::DepreciationExpense
                        && journal.credit_account == AccountType::AccumulatedDepreciation
                }
                "retirement" => {
                    journal.credit_account == AccountType::Asset
                        && matches!(
                            journal.debit_account,
                            AccountType::AccumulatedDepreciation | AccountType::RetirementLoss
                        )
                }
                _ => false,
            };
            if !valid_pair {
                errors.push(format!(
                    "Journal entry {}: invalid account pair for {} event",
                    journal.entry_id, event.event_type
                ));
            }
            let expected_amount = match event.event_type.as_str() {
                "capitalization" | "depreciation" => {
                    IntelligenceCapitalLedger::event_amount(event).ok()
                }
                "retirement" if journal.debit_account == AccountType::AccumulatedDepreciation => {
                    event
                        .details
                        .get("accumulated_depreciation")
                        .and_then(|value| value.as_f64())
                }
                "retirement" if journal.debit_account == AccountType::RetirementLoss => event
                    .details
                    .get("retired_value")
                    .and_then(|value| value.as_f64()),
                _ => Some(0.0),
            };
            if expected_amount
                .is_some_and(|expected| expected <= 0.0 || !amounts_equal(journal.amount, expected))
            {
                errors.push(format!(
                    "Journal entry {}: amount does not match event",
                    journal.entry_id
                ));
            }
            let metadata_asset_id = journal
                .metadata
                .get("asset_id")
                .and_then(|value| value.as_str());
            let requires_asset_metadata = matches!(
                event.event_type.as_str(),
                "capitalization" | "depreciation" | "retirement"
            );
            if (requires_asset_metadata && metadata_asset_id.is_none())
                || metadata_asset_id.is_some_and(|asset_id| asset_id != event.asset_id.to_string())
            {
                errors.push(format!(
                    "Journal entry {}: metadata asset does not match event",
                    journal.entry_id
                ));
            }
        }

        for event in &self.ledger.events {
            let journals: Vec<&JournalEntry> = self
                .ledger
                .journal_entries
                .iter()
                .filter(|journal| journal.event_id == event.event_id)
                .collect();
            let expected_count = match event.event_type.as_str() {
                "capitalization" => Some(1),
                "depreciation" => IntelligenceCapitalLedger::event_amount(event)
                    .ok()
                    .map(|amount| usize::from(amount > 0.0)),
                "retirement" => {
                    let accumulated = event
                        .details
                        .get("accumulated_depreciation")
                        .and_then(|value| value.as_f64());
                    let retired = event
                        .details
                        .get("retired_value")
                        .and_then(|value| value.as_f64());
                    match (accumulated, retired) {
                        (Some(accumulated), Some(retired)) => {
                            Some(usize::from(accumulated > 0.0) + usize::from(retired > 0.0))
                        }
                        _ => None,
                    }
                }
                _ => Some(0),
            };
            if expected_count.is_some_and(|expected| journals.len() != expected) {
                errors.push(format!(
                    "Event {}: expected {} journal postings, found {}",
                    event.event_id,
                    expected_count.unwrap_or_default(),
                    journals.len()
                ));
            }
            let mut seen_debits = std::collections::HashSet::new();
            if journals
                .iter()
                .any(|journal| !seen_debits.insert(journal.debit_account))
                && expected_count.is_some()
            {
                errors.push(format!(
                    "Event {}: duplicate journal posting",
                    event.event_id
                ));
            }
        }

        for asset in self.ledger.assets.values() {
            errors.extend(self.validate_asset_history(asset));
        }

        // Verify proof chain integrity
        let proof_errors = self.verify_proof_chain();
        errors.extend(proof_errors);

        errors
    }

    fn validate_asset_history(&self, asset: &IntelligenceAsset) -> Vec<String> {
        let mut errors = Vec::new();
        let events: Vec<&CapitalEvent> = self
            .ledger
            .events
            .iter()
            .filter(|event| event.asset_id == asset.asset_id)
            .collect();
        let capitalization_index = match events
            .iter()
            .position(|event| event.event_type == "capitalization")
        {
            Some(index) => index,
            None => {
                errors.push(format!(
                    "Asset {}: missing capitalization event",
                    asset.asset_id
                ));
                return errors;
            }
        };
        if capitalization_index != 0 {
            errors.push(format!(
                "Asset {}: capitalization is not its first event",
                asset.asset_id
            ));
        }
        if events
            .iter()
            .filter(|event| event.event_type == "capitalization")
            .count()
            != 1
        {
            errors.push(format!(
                "Asset {}: expected exactly one capitalization event",
                asset.asset_id
            ));
            return errors;
        }

        let capitalization = events[capitalization_index];
        let mut owner = match capitalization
            .details
            .get("owner")
            .and_then(|value| value.as_str())
        {
            Some(owner) if !owner.trim().is_empty() => owner.to_string(),
            _ => {
                errors.push(format!(
                    "Asset {}: capitalization owner is invalid",
                    asset.asset_id
                ));
                return errors;
            }
        };
        let initial_value = match capitalization
            .details
            .get("amount")
            .and_then(|value| value.as_f64())
        {
            Some(value) if value.is_finite() && value > 0.0 => value,
            _ => {
                errors.push(format!(
                    "Asset {}: capitalization amount is invalid",
                    asset.asset_id
                ));
                return errors;
            }
        };
        if !amounts_equal(initial_value, asset.initial_value) {
            errors.push(format!(
                "Asset {}: capitalization amount does not match initial value",
                asset.asset_id
            ));
        }
        if capitalization.timestamp != asset.created_at
            || capitalization
                .details
                .get("depreciation_method")
                .and_then(|value| value.as_str())
                != Some(asset.depreciation_method.to_string().as_str())
            || capitalization
                .details
                .get("useful_life_months")
                .and_then(|value| value.as_i64())
                != Some(asset.useful_life_months as i64)
        {
            errors.push(format!(
                "Asset {}: capitalization terms do not match stored policy",
                asset.asset_id
            ));
        }

        let mut current_value = initial_value;
        let mut status = AssetStatus::Active;
        for event in events.iter().skip(capitalization_index + 1) {
            match event.event_type.as_str() {
                "capitalization" => {}
                "allocation" => {
                    if status == AssetStatus::Retired {
                        errors.push(format!(
                            "Event {}: allocation follows retirement",
                            event.event_id
                        ));
                        continue;
                    }
                    let from_owner = event
                        .details
                        .get("from_owner")
                        .and_then(|value| value.as_str());
                    let to_owner = event
                        .details
                        .get("to_owner")
                        .and_then(|value| value.as_str());
                    if from_owner != Some(owner.as_str())
                        || to_owner.is_none_or(|value| value.trim().is_empty())
                    {
                        errors.push(format!(
                            "Event {}: allocation ownership chain is invalid",
                            event.event_id
                        ));
                    } else if let Some(to_owner) = to_owner {
                        owner = to_owner.to_string();
                    }
                }
                "utilization" => {
                    if status == AssetStatus::Retired
                        || IntelligenceCapitalLedger::event_amount(event)
                            .map_or(true, |amount| amount <= 0.0)
                    {
                        errors.push(format!(
                            "Event {}: invalid utilization transition",
                            event.event_id
                        ));
                    }
                }
                "depreciation" => {
                    let previous = event
                        .details
                        .get("previous_value")
                        .and_then(|value| value.as_f64());
                    let new_value = event
                        .details
                        .get("new_value")
                        .and_then(|value| value.as_f64());
                    let amount = IntelligenceCapitalLedger::event_amount(event).ok();
                    let valid = status != AssetStatus::Retired
                        && previous.is_some_and(|value| {
                            value.is_finite() && amounts_equal(value, current_value)
                        })
                        && new_value.is_some_and(|value| {
                            value.is_finite() && value >= 0.0 && value <= current_value
                        })
                        && matches!((previous, new_value, amount), (Some(previous), Some(new_value), Some(amount)) if amounts_equal(previous - new_value, amount));
                    if !valid {
                        errors.push(format!(
                            "Event {}: depreciation does not reconcile to carrying value",
                            event.event_id
                        ));
                    } else if let Some(new_value) = new_value {
                        current_value = new_value;
                        if event
                            .details
                            .get("salvage_value")
                            .and_then(|value| value.as_f64())
                            .is_some_and(|salvage| current_value <= salvage)
                        {
                            status = AssetStatus::Depreciated;
                        }
                    }
                }
                "retirement" => {
                    let retired_value = event
                        .details
                        .get("retired_value")
                        .and_then(|value| value.as_f64());
                    let accumulated_depreciation = event
                        .details
                        .get("accumulated_depreciation")
                        .and_then(|value| value.as_f64());
                    if status == AssetStatus::Retired
                        || retired_value.is_none_or(|value| {
                            !value.is_finite() || !amounts_equal(value, current_value)
                        })
                        || accumulated_depreciation.is_none_or(|value| {
                            !value.is_finite()
                                || !amounts_equal(value, initial_value - current_value)
                        })
                    {
                        errors.push(format!(
                            "Event {}: retirement does not reconcile to carrying value",
                            event.event_id
                        ));
                    }
                    current_value = 0.0;
                    status = AssetStatus::Retired;
                }
                _ => {}
            }
        }

        if owner != asset.owner
            || asset
                .current_value
                .is_none_or(|value| !amounts_equal(value, current_value))
            || status != asset.status
        {
            errors.push(format!(
                "Asset {}: stored state does not match its event history",
                asset.asset_id
            ));
        }
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
        let asset = self
            .ledger
            .assets
            .get(&asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;
        if start >= end {
            return Err(IclError::InvalidDateRange {
                start: start.to_rfc3339(),
                end: end.to_rfc3339(),
            });
        }
        if start < asset.created_at {
            return Err(IclError::DepreciationError(
                "Depreciation period cannot begin before capitalization".into(),
            ));
        }

        let existing_depreciations: Vec<&CapitalEvent> = self
            .ledger
            .get_events_for_asset(asset_id)
            .into_iter()
            .filter(|e| e.event_type == "depreciation")
            .collect();

        for dep_event in existing_depreciations {
            let existing_start = dep_event
                .details
                .get("start_date")
                .and_then(|value| value.as_str())
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc));
            let existing_end = dep_event
                .details
                .get("end_date")
                .and_then(|value| value.as_str())
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc));
            let (existing_start, existing_end) = match (existing_start, existing_end) {
                (Some(existing_start), Some(existing_end)) if existing_start < existing_end => {
                    (existing_start, existing_end)
                }
                _ => {
                    return Err(IclError::IntegrityViolation(format!(
                        "Depreciation event {} has an invalid period",
                        dep_event.event_id
                    )))
                }
            };
            if start < existing_end && end > existing_start {
                return Err(IclError::OverlappingDepreciation);
            }
        }

        Ok(())
    }

    pub fn verify_proof_chain(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut seen_proof_ids = std::collections::HashSet::new();
        let mut previous_by_asset: std::collections::HashMap<Uuid, &CapitalProof> =
            std::collections::HashMap::new();
        let verifier = crate::core::proofs::CapitalProofGenerator::new(self.ledger);

        for proof in &self.ledger.proofs {
            if !seen_proof_ids.insert(proof.proof_id) {
                errors.push(format!("Proof {} has a duplicate proof id", proof.proof_id));
            }
            if !self.ledger.proof_hash_is_valid(proof) {
                errors.push(format!(
                    "Proof {} has an invalid proof hash",
                    proof.proof_id
                ));
            } else if !verifier.verify_proof(proof) {
                errors.push(format!(
                    "Proof {} is not valid for the canonical ledger",
                    proof.proof_id
                ));
            }

            match previous_by_asset.get(&proof.asset_id) {
                Some(previous) => {
                    if proof.previous_proof_hash.as_deref() != previous.proof_hash.as_deref() {
                        errors.push(format!(
                            "Proof chain break for asset {} at proof {}",
                            proof.asset_id, proof.proof_id
                        ));
                    }
                    if proof.timestamp < previous.timestamp {
                        errors.push(format!(
                            "Proof {} predates the previous proof for asset {}",
                            proof.proof_id, proof.asset_id
                        ));
                    }
                }
                None if proof.previous_proof_hash.is_some() => {
                    errors.push(format!(
                        "Proof chain break for asset {}: first proof {} has a previous hash",
                        proof.asset_id, proof.proof_id
                    ));
                }
                None => {}
            }
            previous_by_asset.insert(proof.asset_id, proof);
        }

        errors
    }
}

fn amounts_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;

    fn create_asset(ledger: &mut IntelligenceCapitalLedger) -> Uuid {
        let asset_id = Uuid::new_v4();
        ledger
            .create_asset_at(
                asset_id,
                "finance".to_string(),
                1000.0,
                DepreciationMethod::Linear,
                12,
                Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
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
