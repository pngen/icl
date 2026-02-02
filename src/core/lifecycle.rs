use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::core::types::*;
use crate::core::ledger::IntelligenceCapitalLedger;
use crate::core::depreciation::calculate_depreciation;
use crate::core::error::*;

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
        useful_life_months: i32
    ) -> IclResult<IntelligenceAsset> {
        let asset = self.ledger.create_asset(
            asset_id,
            owner,
            initial_value,
            depreciation_method,
            useful_life_months
        )?;

        let journal_entry = JournalEntry {
            entry_id: Uuid::new_v4(),
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            debit_account: AccountType::Asset,
            credit_account: AccountType::AccumulatedDepreciation,
            amount: initial_value,
            description: "Asset capitalization".to_string(),
            metadata: {
                let mut map = std::collections::HashMap::new();
                map.insert("asset_id".to_string(), serde_json::Value::String(asset_id.to_string()));
                map.insert("owner".to_string(), serde_json::Value::String(asset.owner.clone()));
                map.insert("initial_value".to_string(), serde_json::json!(initial_value));
                map
            }
        };
        
        self.ledger.record_journal_entry(journal_entry)?;
        
        Ok(asset)
    }

    pub fn allocate(&mut self, asset_id: Uuid, target_owner: String) -> IclResult<CapitalEvent> {
        let asset = self.ledger.get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;
        
        if asset.status == AssetStatus::Retired {
            return Err(IclError::AssetRetired(asset_id));
        }
        
        let old_owner = asset.owner.clone();
        
        let mut updated_asset = self.ledger.assets.get(&asset_id).unwrap().clone();
        updated_asset.owner = target_owner.clone();
        self.ledger.assets.insert(asset_id, updated_asset);
        
        let event = CapitalEvent {
            event_id: Uuid::new_v4(),
            asset_id,
            event_type: "allocation".to_string(),
            timestamp: Utc::now(),
            details: {
                let mut map = std::collections::HashMap::new();
                map.insert("from_owner".to_string(), serde_json::Value::String(old_owner));
                map.insert("to_owner".to_string(), serde_json::Value::String(target_owner));
                map
            }
        };
        
        self.ledger.record_event(event.clone())?;
        Ok(event)
    }

    pub fn utilize(&mut self, asset_id: Uuid, amount: f64) -> IclResult<CapitalEvent> {
        if !self.ledger.assets.contains_key(&asset_id) {
            return Err(IclError::AssetNotFound(asset_id));
        }
        
        if amount <= 0.0 {
            return Err(IclError::InvalidEvent("Utilization amount must be positive".into()));
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
            }
        };
        
        self.ledger.record_event(event.clone())?;
        Ok(event)
    }

    pub fn depreciate(
        &mut self,
        asset_id: Uuid,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
        salvage_value: f64,
        rate_multiplier: f64
    ) -> IclResult<CapitalEvent> {
        let asset = self.ledger.get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;
        
        if asset.status == AssetStatus::Retired {
            return Err(IclError::AssetRetired(asset_id));
        }

        use crate::core::integrity::IntegrityChecker;
        let mut checker = IntegrityChecker::new(self.ledger);
        checker.validate_depreciation_period(asset_id, start_date, end_date)?;

        let previous_value = asset.current_value.unwrap_or(asset.initial_value);
        let (depreciation_amount, new_value) = calculate_depreciation(
            asset,
            start_date,
            end_date,
            salvage_value,
            rate_multiplier
        )?;

        let mut updated_asset = self.ledger.assets.get(&asset_id).unwrap().clone();
        updated_asset.current_value = Some(new_value);
        if new_value <= salvage_value {
            updated_asset.status = AssetStatus::Depreciated;
        }
        self.ledger.assets.insert(asset_id, updated_asset);

        let event = CapitalEvent {
            event_id: Uuid::new_v4(),
            asset_id,
            event_type: "depreciation".to_string(),
            timestamp: Utc::now(),
            details: {
                let mut map = std::collections::HashMap::new();
                map.insert("amount".to_string(), serde_json::json!(depreciation_amount));
                map.insert("start_date".to_string(), serde_json::Value::String(start_date.to_rfc3339()));
                map.insert("end_date".to_string(), serde_json::Value::String(end_date.to_rfc3339()));
                map.insert("salvage_value".to_string(), serde_json::json!(salvage_value));
                map.insert("rate_multiplier".to_string(), serde_json::json!(rate_multiplier));
                map.insert("previous_value".to_string(), serde_json::json!(previous_value));
                map.insert("new_value".to_string(), serde_json::json!(new_value));
                map
            }
        };
        
        self.ledger.record_event(event.clone())?;
        
        if depreciation_amount > 0.0 {
            let journal_entry = JournalEntry {
                entry_id: Uuid::new_v4(),
                event_id: event.event_id,
                timestamp: Utc::now(),
                debit_account: AccountType::DepreciationExpense,
                credit_account: AccountType::AccumulatedDepreciation,
                amount: depreciation_amount,
                description: "Asset depreciation".to_string(),
                metadata: {
                    let mut map = std::collections::HashMap::new();
                    map.insert("asset_id".to_string(), serde_json::Value::String(asset_id.to_string()));
                    map.insert("previous_value".to_string(), serde_json::json!(previous_value));
                    map.insert("new_value".to_string(), serde_json::json!(new_value));
                    for (k, v) in &event.details {
                        map.insert(k.clone(), v.clone());
                    }
                    map
                }
            };
            
            self.ledger.record_journal_entry(journal_entry)?;
        }
        
        Ok(event)
    }

    pub fn retire(&mut self, asset_id: Uuid) -> IclResult<CapitalEvent> {
        let asset = self.ledger.get_asset(asset_id)
            .ok_or(IclError::AssetNotFound(asset_id))?;
        
        if asset.status == AssetStatus::Retired {
            return Err(IclError::AssetRetired(asset_id));
        }
        
        let remaining_value = asset.current_value;
        let mut updated_asset = self.ledger.assets.get(&asset_id).unwrap().clone();
        updated_asset.status = AssetStatus::Retired;
        updated_asset.current_value = Some(0.0);
        self.ledger.assets.insert(asset_id, updated_asset);

        let event = CapitalEvent {
            event_id: Uuid::new_v4(),
            asset_id,
            event_type: "retirement".to_string(),
            timestamp: Utc::now(),
            details: {
                let mut map = std::collections::HashMap::new();
                map.insert("retired_value".to_string(), serde_json::json!(remaining_value.unwrap_or(0.0)));
                map
            },
        };
        
        self.ledger.record_event(event.clone())?;
        
        if let Some(current_value) = remaining_value {
            if current_value > 0.0 {
                let journal_entry = JournalEntry {
                    entry_id: Uuid::new_v4(),
                    event_id: event.event_id,
                    timestamp: Utc::now(),
                    debit_account: AccountType::AccumulatedDepreciation,
                    credit_account: AccountType::Asset,
                    amount: current_value,
                    description: "Asset retirement write-off".to_string(),
                    metadata: {
                        let mut map = std::collections::HashMap::new();
                        map.insert("asset_id".to_string(), serde_json::Value::String(asset_id.to_string()));
                        map.insert("retired_value".to_string(), serde_json::json!(current_value));
                        map
                    }
                };
                
                self.ledger.record_journal_entry(journal_entry)?;
            }
        }
        
        Ok(event)
    }

    pub fn get_asset_summary(&self, asset_id: Uuid) -> IclResult<serde_json::Value> {
        let asset = self.ledger.get_asset(asset_id)
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