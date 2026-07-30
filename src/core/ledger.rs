use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use crate::core::error::*;
use crate::core::types::*;

pub struct IntelligenceCapitalLedger {
    pub assets: HashMap<Uuid, IntelligenceAsset>,
    pub events: Vec<CapitalEvent>,
    pub entries: Vec<LedgerEntry>,
    pub journal_entries: Vec<JournalEntry>,
    pub proofs: Vec<CapitalProof>,
    proof_key: Uuid,
    state_commitment: Option<String>,
}

impl IntelligenceCapitalLedger {
    pub fn new() -> Self {
        Self {
            assets: HashMap::new(),
            events: Vec::new(),
            entries: Vec::new(),
            journal_entries: Vec::new(),
            proofs: Vec::new(),
            proof_key: Uuid::new_v4(),
            state_commitment: None,
        }
    }
}

impl std::fmt::Debug for IntelligenceCapitalLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IntelligenceCapitalLedger")
            .field("assets", &self.assets)
            .field("events", &self.events)
            .field("entries", &self.entries)
            .field("journal_entries", &self.journal_entries)
            .field("proofs", &self.proofs)
            .finish_non_exhaustive()
    }
}

impl Default for IntelligenceCapitalLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl IntelligenceCapitalLedger {
    fn compute_state_commitment(&self) -> Option<String> {
        use sha2::{Digest, Sha256};

        let serialized = serde_json::to_vec(&(
            &self.assets,
            &self.events,
            &self.entries,
            &self.journal_entries,
            &self.proofs,
        ))
        .ok()?;
        let mut hasher = Sha256::new();
        hasher.update(self.proof_key.as_bytes());
        hasher.update(serialized);
        Some(format!("{:x}", hasher.finalize()))
    }

    pub(crate) fn storage_is_untampered(&self) -> bool {
        match &self.state_commitment {
            Some(commitment) => self.compute_state_commitment().as_ref() == Some(commitment),
            None => {
                self.assets.is_empty()
                    && self.events.is_empty()
                    && self.entries.is_empty()
                    && self.journal_entries.is_empty()
                    && self.proofs.is_empty()
            }
        }
    }

    fn ensure_storage_untampered(&self) -> IclResult<()> {
        if self.storage_is_untampered() {
            Ok(())
        } else {
            Err(IclError::IntegrityViolation(
                "Ledger storage was modified outside an audited transition".into(),
            ))
        }
    }

    fn refresh_state_commitment(&mut self) {
        self.state_commitment = self.compute_state_commitment();
    }

    pub fn create_asset(
        &mut self,
        asset_id: Uuid,
        owner: String,
        initial_value: f64,
        depreciation_method: DepreciationMethod,
        useful_life_months: i32,
    ) -> IclResult<IntelligenceAsset> {
        self.create_asset_at(
            asset_id,
            owner,
            initial_value,
            depreciation_method,
            useful_life_months,
            Utc::now(),
        )
    }

    pub fn create_asset_at(
        &mut self,
        asset_id: Uuid,
        owner: String,
        initial_value: f64,
        depreciation_method: DepreciationMethod,
        useful_life_months: i32,
        created_at: DateTime<Utc>,
    ) -> IclResult<IntelligenceAsset> {
        if created_at > Utc::now() {
            return Err(IclError::InvalidAsset(
                "Asset capitalization timestamp cannot be in the future".into(),
            ));
        }
        if self.assets.contains_key(&asset_id) {
            return Err(IclError::AssetAlreadyExists(asset_id));
        }

        let asset = IntelligenceAsset {
            asset_id,
            owner,
            initial_value,
            depreciation_method,
            useful_life_months,
            created_at,
            status: AssetStatus::Active,
            current_value: Some(initial_value),
        };

        Self::validate_asset_data(&asset)?;
        let event = CapitalEvent {
            event_id: Uuid::new_v4(),
            asset_id,
            event_type: "capitalization".to_string(),
            timestamp: asset.created_at,
            details: {
                let mut details = HashMap::new();
                details.insert("amount".to_string(), serde_json::json!(initial_value));
                details.insert("owner".to_string(), serde_json::json!(&asset.owner));
                details.insert(
                    "depreciation_method".to_string(),
                    serde_json::json!(depreciation_method.to_string()),
                );
                details.insert(
                    "useful_life_months".to_string(),
                    serde_json::json!(useful_life_months),
                );
                details
            },
        };
        let journal = JournalEntry {
            entry_id: Uuid::new_v4(),
            event_id: event.event_id,
            timestamp: event.timestamp,
            debit_account: AccountType::Asset,
            credit_account: AccountType::CapitalizationSource,
            amount: initial_value,
            description: "Asset capitalization".to_string(),
            metadata: {
                let mut metadata = HashMap::new();
                metadata.insert(
                    "asset_id".to_string(),
                    serde_json::json!(asset_id.to_string()),
                );
                metadata.insert("owner".to_string(), serde_json::json!(&asset.owner));
                metadata.insert(
                    "initial_value".to_string(),
                    serde_json::json!(initial_value),
                );
                metadata
            },
        };
        self.commit_transition(asset.clone(), true, event, vec![journal])?;
        Ok(asset)
    }

    fn validate_asset_data(asset: &IntelligenceAsset) -> IclResult<()> {
        if asset.owner.trim().is_empty() {
            return Err(IclError::InvalidAsset("Owner cannot be empty".into()));
        }
        if !asset.initial_value.is_finite() || asset.initial_value <= 0.0 {
            return Err(IclError::InvalidAsset(
                "Initial value must be positive".into(),
            ));
        }
        if asset.useful_life_months <= 0 {
            return Err(IclError::InvalidAsset(
                "Useful life must be positive".into(),
            ));
        }
        if let Some(current_value) = asset.current_value {
            if !current_value.is_finite()
                || current_value < 0.0
                || current_value > asset.initial_value
            {
                return Err(IclError::InvalidAsset(
                    "Current value must be finite and between zero and initial value".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn record_event(&mut self, event: CapitalEvent) -> IclResult<()> {
        self.ensure_storage_untampered()?;
        if event.event_type == "capitalization" {
            return Err(IclError::InvalidEvent(
                "Capitalization must be recorded with create_asset".into(),
            ));
        }

        if matches!(
            event.event_type.as_str(),
            "allocation" | "utilization" | "depreciation" | "retirement"
        ) {
            let mut asset = self
                .assets
                .get(&event.asset_id)
                .cloned()
                .ok_or(IclError::AssetNotFound(event.asset_id))?;
            let amount = self.validate_event_data(&event, None)?;
            let mut journals = Vec::new();
            match event.event_type.as_str() {
                "allocation" => {
                    let from_owner = event
                        .details
                        .get("from_owner")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    let to_owner = event
                        .details
                        .get("to_owner")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    if from_owner != asset.owner {
                        return Err(IclError::InvalidEvent(
                            "Allocation source owner does not match the asset".into(),
                        ));
                    }
                    asset.owner = to_owner.to_string();
                }
                "utilization" => {}
                "depreciation" => {
                    let previous = finite_detail(&event, "previous_value")?;
                    let new_value = finite_detail(&event, "new_value")?;
                    let start = event
                        .details
                        .get("start_date")
                        .and_then(|value| value.as_str())
                        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                        .map(|value| value.with_timezone(&Utc))
                        .ok_or_else(|| {
                            IclError::InvalidEvent("Invalid depreciation start_date".into())
                        })?;
                    let end = event
                        .details
                        .get("end_date")
                        .and_then(|value| value.as_str())
                        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                        .map(|value| value.with_timezone(&Utc))
                        .ok_or_else(|| {
                            IclError::InvalidEvent("Invalid depreciation end_date".into())
                        })?;
                    if end > event.timestamp {
                        return Err(IclError::InvalidEvent(
                            "Depreciation cannot be recorded before its period ends".into(),
                        ));
                    }
                    crate::core::integrity::IntegrityChecker::new(self)
                        .validate_depreciation_period(event.asset_id, start, end)?;
                    let current = asset.current_value.ok_or_else(|| {
                        IclError::InvalidAsset("Asset has no current value".into())
                    })?;
                    if !amounts_equal(previous, current) {
                        return Err(IclError::InvalidEvent(
                            "Depreciation previous value does not match the asset".into(),
                        ));
                    }
                    let salvage = finite_detail(&event, "salvage_value")?;
                    let rate = finite_detail(&event, "rate_multiplier")?;
                    let (expected_amount, expected_new_value) =
                        crate::core::depreciation::calculate_depreciation(
                            &asset, start, end, salvage, rate,
                        )?;
                    if !amounts_equal(amount, expected_amount)
                        || !amounts_equal(new_value, expected_new_value)
                    {
                        return Err(IclError::InvalidEvent(
                            "Depreciation event does not match the configured schedule".into(),
                        ));
                    }
                    asset.current_value = Some(new_value);
                    if new_value <= salvage {
                        asset.status = AssetStatus::Depreciated;
                    }
                    if amount > 0.0 {
                        journals.push(JournalEntry {
                            entry_id: Uuid::new_v4(),
                            event_id: event.event_id,
                            timestamp: event.timestamp,
                            debit_account: AccountType::DepreciationExpense,
                            credit_account: AccountType::AccumulatedDepreciation,
                            amount,
                            description: "Asset depreciation".to_string(),
                            metadata: {
                                let mut metadata = event.details.clone();
                                metadata.insert(
                                    "asset_id".to_string(),
                                    serde_json::json!(event.asset_id.to_string()),
                                );
                                metadata
                            },
                        });
                    }
                }
                "retirement" => {
                    let current = asset.current_value.ok_or_else(|| {
                        IclError::InvalidAsset("Asset has no current value".into())
                    })?;
                    let retired_value = finite_detail(&event, "retired_value")?;
                    let accumulated = finite_detail(&event, "accumulated_depreciation")?;
                    if !amounts_equal(retired_value, current)
                        || !amounts_equal(accumulated, asset.initial_value - current)
                    {
                        return Err(IclError::InvalidEvent(
                            "Retirement values do not match the asset".into(),
                        ));
                    }
                    asset.current_value = Some(0.0);
                    asset.status = AssetStatus::Retired;
                    for (debit_account, amount, description) in [
                        (
                            AccountType::AccumulatedDepreciation,
                            accumulated,
                            "Remove accumulated depreciation on retirement",
                        ),
                        (
                            AccountType::RetirementLoss,
                            retired_value,
                            "Asset retirement write-off",
                        ),
                    ] {
                        if amount > 0.0 {
                            journals.push(JournalEntry {
                                entry_id: Uuid::new_v4(),
                                event_id: event.event_id,
                                timestamp: event.timestamp,
                                debit_account,
                                credit_account: AccountType::Asset,
                                amount,
                                description: description.to_string(),
                                metadata: {
                                    let mut metadata = HashMap::new();
                                    metadata.insert(
                                        "asset_id".to_string(),
                                        serde_json::json!(event.asset_id.to_string()),
                                    );
                                    metadata
                                },
                            });
                        }
                    }
                }
                _ => unreachable!(),
            }
            return self.commit_transition(asset, false, event, journals);
        }

        Err(IclError::InvalidEvent(format!(
            "Unsupported event type: {}",
            event.event_type
        )))
    }

    pub fn record_journal_entry(&mut self, journal_entry: JournalEntry) -> IclResult<()> {
        self.ensure_storage_untampered()?;
        let event = self
            .events
            .iter()
            .find(|event| event.event_id == journal_entry.event_id)
            .ok_or_else(|| {
                IclError::InvalidEntry("Journal entry must reference a recorded event".into())
            })?;
        self.validate_journal_entry(&journal_entry, event, &[])?;
        self.journal_entries.push(journal_entry);
        self.refresh_state_commitment();
        Ok(())
    }

    pub(crate) fn commit_transition(
        &mut self,
        asset: IntelligenceAsset,
        is_new_asset: bool,
        event: CapitalEvent,
        journal_entries: Vec<JournalEntry>,
    ) -> IclResult<()> {
        self.ensure_storage_untampered()?;
        Self::validate_asset_data(&asset)?;
        if asset.asset_id != event.asset_id {
            return Err(IclError::InvalidEvent(
                "Transition event must reference the updated asset".into(),
            ));
        }
        if is_new_asset {
            if self.assets.contains_key(&asset.asset_id) {
                return Err(IclError::AssetAlreadyExists(asset.asset_id));
            }
        } else if !self.assets.contains_key(&asset.asset_id) {
            return Err(IclError::AssetNotFound(asset.asset_id));
        }

        let amount = self.validate_event_data(&event, Some(asset.asset_id))?;
        let mut staged_ids = Vec::with_capacity(journal_entries.len());
        for (index, journal_entry) in journal_entries.iter().enumerate() {
            if matches!(
                event.event_type.as_str(),
                "capitalization" | "depreciation" | "retirement"
            ) && journal_entries[..index].iter().any(|previous| {
                previous.event_id == journal_entry.event_id
                    && previous.debit_account == journal_entry.debit_account
            }) {
                return Err(IclError::InvalidEntry(
                    "Duplicate journal posting for capital event".into(),
                ));
            }
            self.validate_journal_entry(journal_entry, &event, &staged_ids)?;
            staged_ids.push(journal_entry.entry_id);
        }

        self.assets.insert(asset.asset_id, asset);
        self.append_event(event, amount);
        self.journal_entries.extend(journal_entries);
        self.refresh_state_commitment();
        Ok(())
    }

    fn validate_event_data(
        &self,
        event: &CapitalEvent,
        candidate_asset: Option<Uuid>,
    ) -> IclResult<f64> {
        if !self.assets.contains_key(&event.asset_id) && candidate_asset != Some(event.asset_id) {
            return Err(IclError::AssetNotFound(event.asset_id));
        }
        if event.event_type.trim().is_empty() {
            return Err(IclError::InvalidEvent("Event type cannot be empty".into()));
        }
        if event.timestamp > Utc::now() {
            return Err(IclError::InvalidEvent(
                "Event timestamp cannot be in the future".into(),
            ));
        }
        if self
            .assets
            .get(&event.asset_id)
            .is_some_and(|asset| asset.status == AssetStatus::Retired)
        {
            return Err(IclError::AssetRetired(event.asset_id));
        }
        if self
            .events
            .iter()
            .any(|existing| existing.event_id == event.event_id)
        {
            return Err(IclError::InvalidEvent("Event id already exists".into()));
        }
        if let Some(last) = self.events.last() {
            if event.timestamp < last.timestamp {
                return Err(IclError::IntegrityViolation(
                    "Cannot add event with timestamp before last recorded event".into(),
                ));
            }
        }

        let amount = Self::event_amount(event)?;
        match event.event_type.as_str() {
            "capitalization" => {
                if amount <= 0.0
                    || event
                        .details
                        .get("owner")
                        .and_then(|value| value.as_str())
                        .is_none_or(|owner| owner.trim().is_empty())
                {
                    return Err(IclError::InvalidEvent(
                        "Capitalization event must contain a positive amount and owner".into(),
                    ));
                }
            }
            "allocation" => {
                let from_owner = event
                    .details
                    .get("from_owner")
                    .and_then(|value| value.as_str());
                let to_owner = event
                    .details
                    .get("to_owner")
                    .and_then(|value| value.as_str());
                if from_owner.is_none_or(|owner| owner.trim().is_empty())
                    || to_owner.is_none_or(|owner| owner.trim().is_empty())
                {
                    return Err(IclError::InvalidEvent(
                        "Allocation event must contain both owners".into(),
                    ));
                }
            }
            "utilization" if amount <= 0.0 => {
                return Err(IclError::InvalidEvent(
                    "Utilization amount must be positive".into(),
                ));
            }
            "utilization" => {}
            "depreciation" => {
                let start = event
                    .details
                    .get("start_date")
                    .and_then(|value| value.as_str())
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .ok_or_else(|| {
                        IclError::InvalidEvent(
                            "Depreciation event must contain a valid start_date".into(),
                        )
                    })?;
                let end = event
                    .details
                    .get("end_date")
                    .and_then(|value| value.as_str())
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .ok_or_else(|| {
                        IclError::InvalidEvent(
                            "Depreciation event must contain a valid end_date".into(),
                        )
                    })?;
                let previous = finite_detail(event, "previous_value")?;
                let new_value = finite_detail(event, "new_value")?;
                let salvage = finite_detail(event, "salvage_value")?;
                let rate = finite_detail(event, "rate_multiplier")?;
                if start >= end
                    || previous < 0.0
                    || new_value < 0.0
                    || new_value > previous
                    || salvage < 0.0
                    || salvage > previous
                    || rate <= 0.0
                    || !amounts_equal(previous - new_value, amount)
                {
                    return Err(IclError::InvalidEvent(
                        "Depreciation event values do not reconcile".into(),
                    ));
                }
            }
            "retirement" => {
                let retired_value = finite_detail(event, "retired_value")?;
                let accumulated = finite_detail(event, "accumulated_depreciation")?;
                if retired_value < 0.0 || accumulated < 0.0 {
                    return Err(IclError::InvalidEvent(
                        "Retirement values must be non-negative".into(),
                    ));
                }
            }
            _ => {
                return Err(IclError::InvalidEvent(format!(
                    "Unsupported event type: {}",
                    event.event_type
                )))
            }
        }
        Ok(amount)
    }

    pub(crate) fn event_amount(event: &CapitalEvent) -> IclResult<f64> {
        match event.details.get("amount") {
            Some(value) => {
                let amount = value
                    .as_f64()
                    .ok_or_else(|| IclError::InvalidEvent("Event amount must be numeric".into()))?;
                if !amount.is_finite() || amount < 0.0 {
                    return Err(IclError::InvalidEvent(
                        "Event amount must be finite and non-negative".into(),
                    ));
                }
                Ok(amount)
            }
            None => Ok(0.0),
        }
    }

    fn validate_journal_entry(
        &self,
        journal_entry: &JournalEntry,
        event: &CapitalEvent,
        staged_ids: &[Uuid],
    ) -> IclResult<()> {
        if !journal_entry.amount.is_finite() || journal_entry.amount <= 0.0 {
            return Err(IclError::InvalidEntry(
                "Journal entry amount must be positive".into(),
            ));
        }
        if journal_entry.debit_account == journal_entry.credit_account {
            return Err(IclError::InvalidEntry(
                "Debit and credit accounts must differ".into(),
            ));
        }
        if journal_entry.event_id != event.event_id {
            return Err(IclError::InvalidEntry(
                "Journal entry must reference its capital event".into(),
            ));
        }
        if journal_entry.timestamp < event.timestamp {
            return Err(IclError::InvalidEntry(
                "Journal entry cannot predate its capital event".into(),
            ));
        }
        if journal_entry.description.trim().is_empty() {
            return Err(IclError::InvalidEntry(
                "Journal entry description cannot be empty".into(),
            ));
        }
        if self
            .journal_entries
            .iter()
            .any(|entry| entry.entry_id == journal_entry.entry_id)
            || staged_ids.contains(&journal_entry.entry_id)
        {
            return Err(IclError::InvalidEntry(
                "Journal entry id already exists".into(),
            ));
        }

        let valid_pair = match event.event_type.as_str() {
            "capitalization" => {
                journal_entry.debit_account == AccountType::Asset
                    && journal_entry.credit_account == AccountType::CapitalizationSource
            }
            "depreciation" => {
                journal_entry.debit_account == AccountType::DepreciationExpense
                    && journal_entry.credit_account == AccountType::AccumulatedDepreciation
            }
            "retirement" => {
                journal_entry.credit_account == AccountType::Asset
                    && matches!(
                        journal_entry.debit_account,
                        AccountType::AccumulatedDepreciation | AccountType::RetirementLoss
                    )
            }
            _ => false,
        };
        if !valid_pair {
            return Err(IclError::InvalidEntry(format!(
                "Invalid account pair for {} event",
                event.event_type
            )));
        }

        let expected_amount = match event.event_type.as_str() {
            "capitalization" | "depreciation" => Some(
                Self::event_amount(event)
                    .map_err(|error| IclError::InvalidEntry(error.to_string()))?,
            ),
            "retirement" if journal_entry.debit_account == AccountType::AccumulatedDepreciation => {
                Some(
                    finite_detail(event, "accumulated_depreciation")
                        .map_err(|error| IclError::InvalidEntry(error.to_string()))?,
                )
            }
            "retirement" if journal_entry.debit_account == AccountType::RetirementLoss => Some(
                finite_detail(event, "retired_value")
                    .map_err(|error| IclError::InvalidEntry(error.to_string()))?,
            ),
            _ => None,
        };
        if expected_amount.is_some_and(|expected| {
            expected <= 0.0 || !amounts_equal(journal_entry.amount, expected)
        }) {
            return Err(IclError::InvalidEntry(
                "Journal amount does not match its capital event".into(),
            ));
        }
        if expected_amount.is_some()
            && self.journal_entries.iter().any(|existing| {
                existing.event_id == journal_entry.event_id
                    && existing.debit_account == journal_entry.debit_account
            })
        {
            return Err(IclError::InvalidEntry(
                "Duplicate journal posting for capital event".into(),
            ));
        }

        let metadata_asset_id = journal_entry
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
            return Err(IclError::InvalidEntry(
                "Journal metadata asset_id does not match its event".into(),
            ));
        }
        Ok(())
    }

    fn append_event(&mut self, event: CapitalEvent, amount: f64) {
        let entry = LedgerEntry {
            entry_id: Uuid::new_v4(),
            event_id: event.event_id,
            asset_id: event.asset_id,
            timestamp: event.timestamp,
            amount,
            description: event.event_type.clone(),
            metadata: event.details.clone(),
        };
        self.events.push(event);
        self.entries.push(entry);
    }

    pub fn generate_proof(
        &mut self,
        asset_id: Uuid,
        event_id: Option<Uuid>,
    ) -> IclResult<CapitalProof> {
        let integrity_errors =
            crate::core::integrity::IntegrityChecker::new(self).check_all_integrity();
        if let Some(error) = integrity_errors.first() {
            return Err(IclError::IntegrityViolation(format!(
                "Cannot generate proof for an invalid ledger: {}",
                error
            )));
        }
        if !self.assets.contains_key(&asset_id) {
            return Err(IclError::AssetNotFound(asset_id));
        }

        let proof_event = match event_id {
            Some(event_id) => Some(
                self.events
                    .iter()
                    .find(|event| event.event_id == event_id && event.asset_id == asset_id)
                    .ok_or_else(|| {
                        IclError::InvalidEvent(
                            "Proof event must belong to the requested asset".into(),
                        )
                    })?,
            ),
            None => None,
        };

        let previous_proof = self.proofs.iter().rfind(|p| p.asset_id == asset_id);
        let previous_hash = match previous_proof {
            Some(previous) => {
                if !self.proof_hash_is_valid(previous) {
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
            .events
            .iter()
            .filter(|event| event.asset_id == asset_id)
            .map(|event| event.timestamp)
            .max()
        {
            proof_timestamp = proof_timestamp.max(latest_event_timestamp);
        }

        let asset = self.assets.get(&asset_id).unwrap();
        let mut content = crate::core::proofs::asset_proof_content(asset);
        if let Some(event) = proof_event {
            content.insert("event".to_string(), serde_json::json!(event));
        }

        let proof = CapitalProof {
            proof_id: Uuid::new_v4(),
            asset_id,
            event_id,
            timestamp: proof_timestamp,
            origin: "ICL".to_string(),
            previous_proof_hash: previous_hash,
            content,
            proof_hash: None,
        };

        let mut updated_proof = proof;
        updated_proof.proof_hash = Some(updated_proof.compute_hash());
        updated_proof.content.insert(
            "_ledger_authentication".to_string(),
            serde_json::json!(self.sign_proof(&updated_proof)),
        );

        self.proofs.push(updated_proof.clone());
        self.refresh_state_commitment();
        Ok(updated_proof)
    }

    pub(crate) fn sign_proof(&self, proof: &CapitalProof) -> String {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(self.proof_key.as_bytes());
        hasher.update(proof.compute_hash().as_bytes());
        format!("{:x}", hasher.finalize())
    }

    pub(crate) fn proof_hash_is_valid(&self, proof: &CapitalProof) -> bool {
        let computed = proof.compute_hash();
        proof.proof_hash.as_deref() == Some(computed.as_str())
            && proof
                .content
                .get("_ledger_authentication")
                .and_then(|value| value.as_str())
                == Some(self.sign_proof(proof).as_str())
    }

    pub fn get_asset(&self, asset_id: Uuid) -> Option<&IntelligenceAsset> {
        self.assets.get(&asset_id)
    }

    pub fn get_asset_mut(&mut self, asset_id: Uuid) -> Option<&mut IntelligenceAsset> {
        self.assets.get_mut(&asset_id)
    }

    pub fn get_events_for_asset(&self, asset_id: Uuid) -> Vec<&CapitalEvent> {
        self.events
            .iter()
            .filter(|event| event.asset_id == asset_id)
            .collect()
    }

    pub fn get_entries_for_asset(&self, asset_id: Uuid) -> Vec<&LedgerEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.asset_id == asset_id)
            .collect()
    }

    pub fn get_journal_entries_for_asset(&self, asset_id: Uuid) -> Vec<&JournalEntry> {
        let asset_events = self.get_events_for_asset(asset_id);
        let event_ids: std::collections::HashSet<Uuid> =
            asset_events.iter().map(|e| e.event_id).collect();

        self.journal_entries
            .iter()
            .filter(|entry| event_ids.contains(&entry.event_id))
            .collect()
    }

    pub fn verify_journal_balance(&self) -> bool {
        if !self.storage_is_untampered() {
            return false;
        }
        let mut seen_ids = std::collections::HashSet::new();
        let entries_valid = self.journal_entries.iter().all(|entry| {
            let event = match self
                .events
                .iter()
                .find(|event| event.event_id == entry.event_id)
            {
                Some(event) => event,
                None => return false,
            };
            let expected_amount = match event.event_type.as_str() {
                "capitalization"
                    if entry.debit_account == AccountType::Asset
                        && entry.credit_account == AccountType::CapitalizationSource =>
                {
                    Self::event_amount(event).ok()
                }
                "depreciation"
                    if entry.debit_account == AccountType::DepreciationExpense
                        && entry.credit_account == AccountType::AccumulatedDepreciation =>
                {
                    Self::event_amount(event).ok()
                }
                "retirement"
                    if entry.debit_account == AccountType::AccumulatedDepreciation
                        && entry.credit_account == AccountType::Asset =>
                {
                    event
                        .details
                        .get("accumulated_depreciation")
                        .and_then(|value| value.as_f64())
                }
                "retirement"
                    if entry.debit_account == AccountType::RetirementLoss
                        && entry.credit_account == AccountType::Asset =>
                {
                    event
                        .details
                        .get("retired_value")
                        .and_then(|value| value.as_f64())
                }
                "capitalization" | "depreciation" | "retirement" => return false,
                _ => return false,
            };
            seen_ids.insert(entry.entry_id)
                && entry.amount.is_finite()
                && entry.amount > 0.0
                && entry.debit_account != entry.credit_account
                && entry.timestamp >= event.timestamp
                && expected_amount
                    .is_some_and(|expected| expected > 0.0 && amounts_equal(entry.amount, expected))
        });
        entries_valid
            && self.events.iter().all(|event| {
                let journals: Vec<&JournalEntry> = self
                    .journal_entries
                    .iter()
                    .filter(|journal| journal.event_id == event.event_id)
                    .collect();
                match event.event_type.as_str() {
                    "capitalization" => journals.len() == 1,
                    "depreciation" => Self::event_amount(event)
                        .ok()
                        .is_some_and(|amount| journals.len() == usize::from(amount > 0.0)),
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
                                journals.len()
                                    == usize::from(accumulated > 0.0) + usize::from(retired > 0.0)
                            }
                            _ => false,
                        }
                    }
                    _ => journals.is_empty(),
                }
            })
    }

    pub fn export_audit_trail(&self, format: &str) -> IclResult<String> {
        let integrity_errors =
            crate::core::integrity::IntegrityChecker::new(self).check_all_integrity();
        if let Some(error) = integrity_errors.first() {
            return Err(IclError::IntegrityViolation(format!(
                "Cannot export an invalid ledger: {}",
                error
            )));
        }
        match format {
            "json" => {
                let mut assets = self.assets.values().collect::<Vec<_>>();
                assets.sort_by_key(|asset| asset.asset_id);
                let data = serde_json::json!({
                    "version": "1.0.0",
                    "exported_at": Utc::now().to_rfc3339(),
                    "assets": assets,
                    "events": &self.events,
                    "entries": &self.entries,
                    "journal_entries": &self.journal_entries,
                    "proofs": &self.proofs,
                });
                serde_json::to_string_pretty(&data).map_err(IclError::from)
            }
            "csv" => {
                let mut csv =
                    String::from("entry_id,event_id,asset_id,timestamp,amount,description\n");
                for entry in &self.entries {
                    csv.push_str(&format!(
                        "{},{},{},{},{},{}\n",
                        entry.entry_id,
                        entry.event_id,
                        entry.asset_id,
                        entry.timestamp.to_rfc3339(),
                        entry.amount,
                        csv_field(&entry.description)
                    ));
                }
                Ok(csv)
            }
            _ => Err(IclError::UnsupportedFormat(format.to_string())),
        }
    }

    pub fn asset_count(&self) -> usize {
        self.assets.len()
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn finite_detail(event: &CapitalEvent, key: &str) -> IclResult<f64> {
    event
        .details
        .get(key)
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite())
        .ok_or_else(|| IclError::InvalidEvent(format!("Event field {} must be finite", key)))
}

fn amounts_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

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

    fn event_at(asset_id: Uuid, event_id: Uuid, timestamp: chrono::DateTime<Utc>) -> CapitalEvent {
        let mut details = HashMap::new();
        details.insert("amount".to_string(), serde_json::json!(100.0));

        CapitalEvent {
            event_id,
            asset_id,
            event_type: "utilization".to_string(),
            timestamp,
            details,
        }
    }

    fn journal_entry(entry_id: Uuid, amount: f64) -> JournalEntry {
        JournalEntry {
            entry_id,
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            debit_account: AccountType::Asset,
            credit_account: AccountType::AccumulatedDepreciation,
            amount,
            description: "entry".to_string(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn rejects_non_finite_financial_values() {
        let mut ledger = IntelligenceCapitalLedger::new();
        assert!(ledger
            .create_asset(
                Uuid::new_v4(),
                "finance".to_string(),
                f64::NAN,
                DepreciationMethod::Linear,
                12,
            )
            .is_err());
        assert!(ledger
            .create_asset(
                Uuid::new_v4(),
                "finance".to_string(),
                f64::INFINITY,
                DepreciationMethod::Linear,
                12,
            )
            .is_err());

        assert!(ledger
            .record_journal_entry(journal_entry(Uuid::new_v4(), f64::NAN))
            .is_err());
        assert!(ledger
            .record_journal_entry(journal_entry(Uuid::new_v4(), f64::INFINITY))
            .is_err());
    }

    #[test]
    fn record_event_rejects_duplicate_and_retroactive_events() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        let first_time = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        let earlier_time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let event_id = Uuid::new_v4();

        ledger
            .record_event(event_at(asset_id, event_id, first_time))
            .unwrap();

        let duplicate = event_at(asset_id, event_id, first_time);
        assert!(ledger.record_event(duplicate).is_err());

        let retroactive = event_at(asset_id, Uuid::new_v4(), earlier_time);
        assert!(ledger.record_event(retroactive).is_err());
        assert_eq!(ledger.event_count(), 2);
    }

    #[test]
    fn record_event_rejects_non_numeric_amount() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        let mut event = event_at(
            asset_id,
            Uuid::new_v4(),
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        event
            .details
            .insert("amount".to_string(), serde_json::json!("not-a-number"));

        assert!(ledger.record_event(event).is_err());
        assert_eq!(ledger.event_count(), 1);
    }

    #[test]
    fn record_event_rejects_negative_amount() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        let mut event = event_at(asset_id, Uuid::new_v4(), Utc::now());
        event
            .details
            .insert("amount".to_string(), serde_json::json!(-1.0));

        assert!(ledger.record_event(event).is_err());
        assert_eq!(ledger.events.len(), 1);
        assert_eq!(ledger.entries.len(), 1);
    }

    #[test]
    fn record_event_rejects_future_timestamp() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        let event = event_at(asset_id, Uuid::new_v4(), Utc::now() + Duration::minutes(1));

        assert!(ledger.record_event(event).is_err());
        assert_eq!(ledger.events.len(), 1);
    }

    #[test]
    fn journal_entry_ids_must_be_unique() {
        let mut ledger = IntelligenceCapitalLedger::new();
        create_asset(&mut ledger);
        let duplicate = ledger.journal_entries[0].clone();
        assert!(ledger.record_journal_entry(duplicate).is_err());
        assert_eq!(ledger.journal_entries.len(), 1);
    }

    #[test]
    fn journal_entries_require_real_events_and_distinct_accounts() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let orphan = journal_entry(Uuid::new_v4(), 100.0);
        assert!(ledger.record_journal_entry(orphan).is_err());

        let asset_id = create_asset(&mut ledger);
        let event_id = Uuid::new_v4();
        ledger
            .record_event(event_at(asset_id, event_id, Utc::now()))
            .unwrap();
        let mut same_account = journal_entry(Uuid::new_v4(), 100.0);
        same_account.event_id = event_id;
        same_account.credit_account = same_account.debit_account;
        assert!(ledger.record_journal_entry(same_account).is_err());
        assert_eq!(ledger.journal_entries.len(), 1);
    }

    #[test]
    fn canonical_queries_and_integrity_see_event_mutation() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = create_asset(&mut ledger);
        ledger
            .record_event(event_at(asset_id, Uuid::new_v4(), Utc::now()))
            .unwrap();
        ledger
            .events
            .last_mut()
            .unwrap()
            .details
            .insert("amount".to_string(), serde_json::json!(999.0));

        assert_eq!(
            ledger
                .get_events_for_asset(asset_id)
                .last()
                .unwrap()
                .details
                .get("amount")
                .and_then(|value| value.as_f64()),
            Some(999.0)
        );
        assert!(crate::core::integrity::IntegrityChecker::new(&ledger)
            .check_all_integrity()
            .iter()
            .any(|error| error.contains("does not match its source event")));
    }

    #[test]
    fn csv_export_preserves_embedded_delimiters() {
        let mut ledger = IntelligenceCapitalLedger::new();
        create_asset(&mut ledger);

        let csv = ledger.export_audit_trail("csv").unwrap();
        assert!(csv.starts_with("entry_id,event_id,asset_id,timestamp,amount,description\n"));
        assert_eq!(
            csv_field("quoted,\n\"description\""),
            "\"quoted,\n\"\"description\"\"\""
        );
    }

    #[test]
    fn generate_proof_rejects_event_from_another_asset() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let first_asset = create_asset(&mut ledger);
        let second_asset = create_asset(&mut ledger);
        let event_id = Uuid::new_v4();

        ledger
            .record_event(event_at(
                second_asset,
                event_id,
                Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            ))
            .unwrap();

        assert!(ledger.generate_proof(first_asset, Some(event_id)).is_err());
    }
}
