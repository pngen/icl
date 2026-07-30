use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::core::depreciation::calculate_depreciation;
use crate::core::error::*;
use crate::core::ledger::IntelligenceCapitalLedger;
use crate::core::types::*;

#[derive(Debug)]
pub struct IntelligenceCapitalLifecycle<'a> {
    pub ledger: &'a mut IntelligenceCapitalLedger,
}

impl<'a> IntelligenceCapitalLifecycle<'a> {
    pub fn new(ledger: &'a mut IntelligenceCapitalLedger) -> Self {
        Self { ledger }
    }

    pub fn capitalize(
        &mut self,
        asset_id: Uuid,
        owner: String,
        initial_value: f64,
        depreciation_method: DepreciationMethod,
        useful_life_months: i32,
    ) -> IclResult<IntelligenceAsset> {
        self.ledger.create_asset(
            asset_id,
            owner,
            initial_value,
            depreciation_method,
            useful_life_months,
        )
    }

    pub fn allocate(&mut self, asset_id: Uuid, target_owner: String) -> IclResult<CapitalEvent> {
        if target_owner.is_empty() {
            return Err(IclError::InvalidAsset("Owner cannot be empty".into()));
        }

        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;

        if asset.status == AssetStatus::Retired {
            return Err(IclError::AssetRetired(asset_id));
        }
        let old_owner = asset.owner.clone();

        let mut updated_asset = asset.clone();
        updated_asset.owner = target_owner.clone();

        let event = CapitalEvent {
            event_id: Uuid::new_v4(),
            asset_id,
            event_type: "allocation".to_string(),
            timestamp: Utc::now(),
            details: {
                let mut map = std::collections::HashMap::new();
                map.insert(
                    "from_owner".to_string(),
                    serde_json::Value::String(old_owner),
                );
                map.insert(
                    "to_owner".to_string(),
                    serde_json::Value::String(target_owner),
                );
                map
            },
        };

        self.ledger
            .commit_transition(updated_asset, false, event.clone(), vec![])?;
        Ok(event)
    }

    pub fn utilize(&mut self, asset_id: Uuid, amount: f64) -> IclResult<CapitalEvent> {
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;

        if asset.status == AssetStatus::Retired {
            return Err(IclError::AssetRetired(asset_id));
        }
        if !amount.is_finite() || amount <= 0.0 {
            return Err(IclError::InvalidEvent(
                "Utilization amount must be positive".into(),
            ));
        }

        let event = CapitalEvent {
            event_id: Uuid::new_v4(),
            asset_id,
            event_type: "utilization".to_string(),
            timestamp: Utc::now(),
            details: {
                let mut map = std::collections::HashMap::new();
                map.insert("amount".to_string(), serde_json::json!(amount));
                map
            },
        };

        self.ledger
            .commit_transition(asset.clone(), false, event.clone(), vec![])?;
        Ok(event)
    }

    pub fn depreciate(
        &mut self,
        asset_id: Uuid,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
        salvage_value: f64,
        rate_multiplier: f64,
    ) -> IclResult<CapitalEvent> {
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;

        if asset.status == AssetStatus::Retired {
            return Err(IclError::AssetRetired(asset_id));
        }
        let recorded_at = Utc::now();
        if end_date > recorded_at {
            return Err(IclError::DepreciationError(
                "Depreciation cannot be recorded before its period ends".into(),
            ));
        }

        use crate::core::integrity::IntegrityChecker;
        let checker = IntegrityChecker::new(self.ledger);
        checker.validate_depreciation_period(asset_id, start_date, end_date)?;

        let previous_value = asset.current_value.unwrap_or(asset.initial_value);
        let (depreciation_amount, new_value) =
            calculate_depreciation(asset, start_date, end_date, salvage_value, rate_multiplier)?;

        let mut updated_asset = asset.clone();
        updated_asset.current_value = Some(new_value);
        if new_value <= salvage_value {
            updated_asset.status = AssetStatus::Depreciated;
        }

        let event = CapitalEvent {
            event_id: Uuid::new_v4(),
            asset_id,
            event_type: "depreciation".to_string(),
            timestamp: recorded_at,
            details: {
                let mut map = std::collections::HashMap::new();
                map.insert("amount".to_string(), serde_json::json!(depreciation_amount));
                map.insert(
                    "start_date".to_string(),
                    serde_json::Value::String(start_date.to_rfc3339()),
                );
                map.insert(
                    "end_date".to_string(),
                    serde_json::Value::String(end_date.to_rfc3339()),
                );
                map.insert(
                    "salvage_value".to_string(),
                    serde_json::json!(salvage_value),
                );
                map.insert(
                    "rate_multiplier".to_string(),
                    serde_json::json!(rate_multiplier),
                );
                map.insert(
                    "previous_value".to_string(),
                    serde_json::json!(previous_value),
                );
                map.insert("new_value".to_string(), serde_json::json!(new_value));
                map
            },
        };

        let journal_entries = if depreciation_amount > 0.0 {
            vec![JournalEntry {
                entry_id: Uuid::new_v4(),
                event_id: event.event_id,
                timestamp: event.timestamp,
                debit_account: AccountType::DepreciationExpense,
                credit_account: AccountType::AccumulatedDepreciation,
                amount: depreciation_amount,
                description: "Asset depreciation".to_string(),
                metadata: {
                    let mut map = std::collections::HashMap::new();
                    map.insert(
                        "asset_id".to_string(),
                        serde_json::Value::String(asset_id.to_string()),
                    );
                    map.insert(
                        "previous_value".to_string(),
                        serde_json::json!(previous_value),
                    );
                    map.insert("new_value".to_string(), serde_json::json!(new_value));
                    for (k, v) in &event.details {
                        map.insert(k.clone(), v.clone());
                    }
                    map
                },
            }]
        } else {
            vec![]
        };

        self.ledger
            .commit_transition(updated_asset, false, event.clone(), journal_entries)?;

        Ok(event)
    }

    pub fn retire(&mut self, asset_id: Uuid) -> IclResult<CapitalEvent> {
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;

        if asset.status == AssetStatus::Retired {
            return Err(IclError::AssetRetired(asset_id));
        }

        let remaining_value = asset.current_value.ok_or_else(|| {
            IclError::InvalidAsset("Asset must have a current value before retirement".into())
        })?;
        if !remaining_value.is_finite()
            || remaining_value < 0.0
            || remaining_value > asset.initial_value
        {
            return Err(IclError::InvalidAsset(
                "Asset current value is invalid".into(),
            ));
        }
        let accumulated_depreciation = asset.initial_value - remaining_value;
        let mut updated_asset = asset.clone();
        updated_asset.status = AssetStatus::Retired;
        updated_asset.current_value = Some(0.0);

        let event = CapitalEvent {
            event_id: Uuid::new_v4(),
            asset_id,
            event_type: "retirement".to_string(),
            timestamp: Utc::now(),
            details: {
                let mut map = std::collections::HashMap::new();
                map.insert(
                    "retired_value".to_string(),
                    serde_json::json!(remaining_value),
                );
                map.insert(
                    "accumulated_depreciation".to_string(),
                    serde_json::json!(accumulated_depreciation),
                );
                map
            },
        };

        let mut journal_entries = Vec::new();
        if accumulated_depreciation > 0.0 {
            journal_entries.push(JournalEntry {
                entry_id: Uuid::new_v4(),
                event_id: event.event_id,
                timestamp: event.timestamp,
                debit_account: AccountType::AccumulatedDepreciation,
                credit_account: AccountType::Asset,
                amount: accumulated_depreciation,
                description: "Remove accumulated depreciation on retirement".to_string(),
                metadata: {
                    let mut map = std::collections::HashMap::new();
                    map.insert(
                        "asset_id".to_string(),
                        serde_json::Value::String(asset_id.to_string()),
                    );
                    map.insert(
                        "accumulated_depreciation".to_string(),
                        serde_json::json!(accumulated_depreciation),
                    );
                    map
                },
            });
        }
        if remaining_value > 0.0 {
            journal_entries.push(JournalEntry {
                entry_id: Uuid::new_v4(),
                event_id: event.event_id,
                timestamp: event.timestamp,
                debit_account: AccountType::RetirementLoss,
                credit_account: AccountType::Asset,
                amount: remaining_value,
                description: "Asset retirement write-off".to_string(),
                metadata: {
                    let mut map = std::collections::HashMap::new();
                    map.insert(
                        "asset_id".to_string(),
                        serde_json::Value::String(asset_id.to_string()),
                    );
                    map.insert(
                        "retired_value".to_string(),
                        serde_json::json!(remaining_value),
                    );
                    map
                },
            });
        }

        self.ledger
            .commit_transition(updated_asset, false, event.clone(), journal_entries)?;

        Ok(event)
    }

    pub fn get_asset_summary(&self, asset_id: Uuid) -> IclResult<serde_json::Value> {
        if !self.ledger.storage_is_untampered() {
            return Err(IclError::IntegrityViolation(
                "Cannot summarize unaudited ledger state".into(),
            ));
        }
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;

        let events = self.ledger.get_events_for_asset(asset_id);
        let journal_entries = self.ledger.get_journal_entries_for_asset(asset_id);

        Ok(serde_json::json!({
            "asset": asset,
            "event_count": events.len(),
            "journal_entry_count": journal_entries.len(),
            "total_depreciation": events.iter()
                .filter(|e| e.event_type == "depreciation")
                .filter_map(|e| e.details.get("amount").and_then(|v| v.as_f64()))
                .sum::<f64>(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn create_lifecycle_ledger() -> (IntelligenceCapitalLedger, Uuid) {
        let mut ledger = IntelligenceCapitalLedger::new();
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
        (ledger, asset_id)
    }

    #[test]
    fn allocate_rejects_empty_owner_without_mutating_asset() {
        let (mut ledger, asset_id) = create_lifecycle_ledger();
        let mut lifecycle = IntelligenceCapitalLifecycle::new(&mut ledger);

        assert!(lifecycle.allocate(asset_id, "".to_string()).is_err());
        assert_eq!(
            lifecycle.ledger.get_asset(asset_id).unwrap().owner,
            "finance"
        );
    }

    #[test]
    fn utilize_rejects_retired_assets_and_non_finite_amounts() {
        let (mut ledger, asset_id) = create_lifecycle_ledger();
        let mut lifecycle = IntelligenceCapitalLifecycle::new(&mut ledger);

        assert!(lifecycle.utilize(asset_id, f64::NAN).is_err());
        lifecycle.retire(asset_id).unwrap();
        assert!(matches!(
            lifecycle.utilize(asset_id, 100.0),
            Err(IclError::AssetRetired(id)) if id == asset_id
        ));
    }

    fn append_future_event(ledger: &mut IntelligenceCapitalLedger, asset_id: Uuid) {
        let mut details = std::collections::HashMap::new();
        details.insert("amount".to_string(), serde_json::json!(1.0));
        ledger
            .record_event(CapitalEvent {
                event_id: Uuid::new_v4(),
                asset_id,
                event_type: "utilization".to_string(),
                timestamp: Utc::now(),
                details,
            })
            .unwrap();
        let future = Utc::now() + Duration::days(1);
        ledger.events.last_mut().unwrap().timestamp = future;
        ledger.entries.last_mut().unwrap().timestamp = future;
    }

    #[test]
    fn lifecycle_transitions_are_atomic_when_event_append_is_rejected() {
        let (mut ledger, asset_id) = create_lifecycle_ledger();
        append_future_event(&mut ledger, asset_id);
        let before_asset = ledger.get_asset(asset_id).unwrap().clone();
        let before_counts = (
            ledger.events.len(),
            ledger.entries.len(),
            ledger.journal_entries.len(),
        );

        {
            let mut lifecycle = IntelligenceCapitalLifecycle::new(&mut ledger);
            assert!(lifecycle
                .allocate(asset_id, "operations".to_string())
                .is_err());
            assert!(lifecycle
                .depreciate(
                    asset_id,
                    Utc::now() - Duration::days(60),
                    Utc::now() - Duration::days(30),
                    0.0,
                    2.0,
                )
                .is_err());
            assert!(lifecycle.retire(asset_id).is_err());
            assert!(lifecycle
                .capitalize(
                    Uuid::new_v4(),
                    "finance".to_string(),
                    500.0,
                    DepreciationMethod::Linear,
                    12,
                )
                .is_err());
        }

        assert_eq!(ledger.get_asset(asset_id), Some(&before_asset));
        assert_eq!(
            (
                ledger.events.len(),
                ledger.entries.len(),
                ledger.journal_entries.len(),
            ),
            before_counts
        );
        assert_eq!(ledger.asset_count(), 1);
    }

    #[test]
    fn complete_lifecycle_has_traceable_reconciled_postings() {
        let mut ledger = IntelligenceCapitalLedger::new();
        let asset_id = Uuid::new_v4();
        ledger
            .create_asset_at(
                asset_id,
                "finance".to_string(),
                1200.0,
                DepreciationMethod::Linear,
                12,
                Utc::now() - Duration::days(365),
            )
            .unwrap();
        {
            let mut lifecycle = IntelligenceCapitalLifecycle::new(&mut ledger);
            lifecycle
                .allocate(asset_id, "operations".to_string())
                .unwrap();
            lifecycle.utilize(asset_id, 25.0).unwrap();
            lifecycle
                .depreciate(
                    asset_id,
                    Utc::now() - Duration::days(60),
                    Utc::now() - Duration::days(29),
                    0.0,
                    2.0,
                )
                .unwrap();
            lifecycle.retire(asset_id).unwrap();
        }

        assert_eq!(ledger.get_events_for_asset(asset_id).len(), 5);
        assert_eq!(ledger.get_entries_for_asset(asset_id).len(), 5);
        assert_eq!(ledger.get_journal_entries_for_asset(asset_id).len(), 4);
        assert!(ledger.verify_journal_balance());
        assert!(crate::core::integrity::IntegrityChecker::new(&ledger)
            .check_all_integrity()
            .is_empty());
        let capitalization = &ledger.journal_entries[0];
        assert_eq!(capitalization.debit_account, AccountType::Asset);
        assert_eq!(
            capitalization.credit_account,
            AccountType::CapitalizationSource
        );
        assert_eq!(capitalization.event_id, ledger.events[0].event_id);
        let retirement_event_id = ledger.events.last().unwrap().event_id;
        let retirement_total: f64 = ledger
            .journal_entries
            .iter()
            .filter(|entry| entry.event_id == retirement_event_id)
            .map(|entry| entry.amount)
            .sum();
        assert!((retirement_total - 1200.0).abs() < 1e-9);
    }
}
