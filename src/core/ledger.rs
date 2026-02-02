use std::collections::HashMap;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::core::types::*;
use crate::core::error::*;

#[derive(Debug)]
pub struct IntelligenceCapitalLedger {
    pub assets: HashMap<Uuid, IntelligenceAsset>,
    pub events: Vec<CapitalEvent>,
    pub entries: Vec<LedgerEntry>,
    pub journal_entries: Vec<JournalEntry>,
    pub proofs: Vec<CapitalProof>,
    
    // Indexes for performance
    _events_by_asset: HashMap<Uuid, Vec<CapitalEvent>>,
    _entries_by_asset: HashMap<Uuid, Vec<LedgerEntry>>,
    _journal_entries_by_asset: HashMap<Uuid, Vec<JournalEntry>>,
}

impl IntelligenceCapitalLedger {
    pub fn new() -> Self {
        Self {
            assets: HashMap::new(),
            events: Vec::new(),
            entries: Vec::new(),
            journal_entries: Vec::new(),
            proofs: Vec::new(),
            _events_by_asset: HashMap::new(),
            _entries_by_asset: HashMap::new(),
            _journal_entries_by_asset: HashMap::new(),
        }
    }
}

impl Default for IntelligenceCapitalLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl IntelligenceCapitalLedger {
    pub fn create_asset(
        &mut self,
        asset_id: Uuid,
        owner: String,
        initial_value: f64,
        depreciation_method: DepreciationMethod,
        useful_life_months: i32
    ) -> IclResult<IntelligenceAsset> {
        if self.assets.contains_key(&asset_id) {
            return Err(IclError::AssetAlreadyExists(asset_id));
        }

        if owner.is_empty() {
            return Err(IclError::InvalidAsset("Owner cannot be empty".into()));
        }

        if initial_value <= 0.0 {
            return Err(IclError::InvalidAsset("Initial value must be positive".into()));
        }

        if useful_life_months <= 0 {
            return Err(IclError::InvalidAsset("Useful life must be positive".into()));
        }

        let asset = IntelligenceAsset {
            asset_id,
            owner,
            initial_value,
            depreciation_method,
            useful_life_months,
            created_at: Utc::now(),
            status: AssetStatus::Active,
            current_value: Some(initial_value),
        };
        
        self.assets.insert(asset_id, asset.clone());
        Ok(asset)
    }

    pub fn record_event(&mut self, event: CapitalEvent) -> IclResult<()> {
        if !self.assets.contains_key(&event.asset_id) {
            return Err(IclError::AssetNotFound(event.asset_id));
        }

        if event.event_type.is_empty() {
            return Err(IclError::InvalidEvent("Event type cannot be empty".into()));
        }

        self.events.push(event.clone());

        self._events_by_asset.entry(event.asset_id).or_insert_with(Vec::new).push(event.clone());

        let entry = LedgerEntry {
            entry_id: Uuid::new_v4(),
            event_id: event.event_id,
            asset_id: event.asset_id,
            timestamp: event.timestamp,
            amount: event.details.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0),
            description: event.event_type.clone(),
            metadata: event.details.clone(),
        };
        
        self.entries.push(entry.clone());
        self._entries_by_asset.entry(event.asset_id).or_insert_with(Vec::new).push(entry);

        Ok(())
    }

    pub fn record_journal_entry(&mut self, journal_entry: JournalEntry) -> IclResult<()> {
        if journal_entry.amount <= 0.0 {
            return Err(IclError::InvalidEntry("Journal entry amount must be positive".into()));
        }

        self.journal_entries.push(journal_entry.clone());
        self._journal_entries_by_asset
            .entry(journal_entry.event_id)
            .or_insert_with(Vec::new)
            .push(journal_entry);
        Ok(())
    }

    pub fn generate_proof(&mut self, asset_id: Uuid, event_id: Option<Uuid>) -> IclResult<CapitalProof> {
        if !self.assets.contains_key(&asset_id) {
            return Err(IclError::AssetNotFound(asset_id));
        }

        let asset_proofs: Vec<&CapitalProof> = self.proofs.iter()
            .filter(|p| p.asset_id == asset_id)
            .collect();
        
        let previous_hash = if !asset_proofs.is_empty() {
            Some(asset_proofs.last().unwrap().proof_hash.clone().unwrap_or_default())
        } else {
            None
        };

        let asset = self.assets.get(&asset_id).unwrap();
        let mut content: HashMap<String, serde_json::Value> = HashMap::new();
        content.insert("asset_id".to_string(), serde_json::Value::String(asset.asset_id.to_string()));
        content.insert("owner".to_string(), serde_json::Value::String(asset.owner.clone()));
        content.insert("initial_value".to_string(), serde_json::json!(asset.initial_value));
        content.insert("depreciation_method".to_string(), serde_json::Value::String(asset.depreciation_method.to_string()));
        content.insert("useful_life_months".to_string(), serde_json::Value::Number(serde_json::Number::from(asset.useful_life_months)));
        content.insert("status".to_string(), serde_json::Value::String(asset.status.to_string()));
        content.insert("current_value".to_string(), serde_json::json!(asset.current_value.unwrap_or_default()));

        let proof = CapitalProof {
            proof_id: Uuid::new_v4(),
            asset_id,
            event_id,
            timestamp: Utc::now(),
            origin: "ICL".to_string(),
            previous_proof_hash: previous_hash.clone(),
            content,
            proof_hash: None,
        };
        
        let computed_hash = proof.compute_hash();
        let mut updated_proof = proof;
        updated_proof.proof_hash = Some(computed_hash);
        
        self.proofs.push(updated_proof.clone());
        Ok(updated_proof)
    }

    pub fn get_asset(&self, asset_id: Uuid) -> Option<&IntelligenceAsset> {
        self.assets.get(&asset_id)
    }

    pub fn get_asset_mut(&mut self, asset_id: Uuid) -> Option<&mut IntelligenceAsset> {
        self.assets.get_mut(&asset_id)
    }

    pub fn get_events_for_asset(&self, asset_id: Uuid) -> Vec<&CapitalEvent> {
        self._events_by_asset.get(&asset_id).map_or_else(Vec::new, |v| v.iter().collect())
    }

    pub fn get_entries_for_asset(&self, asset_id: Uuid) -> Vec<&LedgerEntry> {
        self._entries_by_asset.get(&asset_id).map_or_else(Vec::new, |v| v.iter().collect())
    }
    
    pub fn get_journal_entries_for_asset(&self, asset_id: Uuid) -> Vec<&JournalEntry> {
        let asset_events = self.get_events_for_asset(asset_id);
        let event_ids: std::collections::HashSet<Uuid> = asset_events.iter().map(|e| e.event_id).collect();
        
        self.journal_entries.iter()
            .filter(|entry| event_ids.contains(&entry.event_id))
            .collect()
    }
    
    pub fn verify_journal_balance(&self) -> bool {
        self.journal_entries.iter().all(|entry| entry.amount > 0.0)
    }
    
    pub fn export_audit_trail(&self, format: &str) -> IclResult<String> {
        match format {
            "json" => {
                let data = serde_json::json!({
                    "version": "1.0.0",
                    "exported_at": Utc::now().to_rfc3339(),
                    "assets": self.assets.values().collect::<Vec<_>>(),
                    "events": &self.events,
                    "entries": &self.entries,
                    "journal_entries": &self.journal_entries,
                    "proofs": &self.proofs,
                });
                serde_json::to_string_pretty(&data).map_err(IclError::from)
            },
            "csv" => {
                let mut csv = String::from("entry_id,event_id,asset_id,timestamp,amount,description\n");
                for entry in &self.entries {
                    csv.push_str(&format!(
                        "{},{},{},{},{},{}\n",
                        entry.entry_id,
                        entry.event_id,
                        entry.asset_id,
                        entry.timestamp.to_rfc3339(),
                        entry.amount,
                        entry.description.replace(',', ";")
                    ));
                }
                Ok(csv)
            },
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