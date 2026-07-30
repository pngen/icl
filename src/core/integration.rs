use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ICAEAttribution {
    pub asset_id: String,
    pub inference_cost: f64,
    pub execution_time: f64,
    pub timestamp: DateTime<Utc>,
    pub model_version: String,
}

#[derive(Debug)]
pub struct IntegrationAdapter {
    icae_data: std::collections::HashMap<String, ICAEAttribution>,
    attribution_history: std::collections::HashMap<String, ICAEAttribution>,
    financial_systems: Vec<String>,
}

impl IntegrationAdapter {
    pub fn new() -> Self {
        Self {
            icae_data: std::collections::HashMap::new(),
            attribution_history: std::collections::HashMap::new(),
            financial_systems: vec![],
        }
    }

    pub fn consume_icae_attribution(
        &mut self,
        attribution_data: &serde_json::Value,
    ) -> IclResult<()> {
        let obj = attribution_data.as_object().ok_or_else(|| {
            IclError::IntegrationError("Attribution data must be an object".into())
        })?;

        // Validate the complete batch before mutating stored attribution state.
        let mut validated = Vec::with_capacity(obj.len());
        let mut seen_asset_ids = std::collections::HashSet::new();
        for (key, value) in obj {
            let key_asset_id = Uuid::parse_str(key).map_err(|_| {
                IclError::IntegrationError(format!(
                    "Invalid attribution asset id key {}: must be a UUID",
                    key
                ))
            })?;
            let attribution =
                serde_json::from_value::<ICAEAttribution>(value.clone()).map_err(|_| {
                    IclError::IntegrationError(format!(
                        "Invalid attribution data format for {}",
                        key
                    ))
                })?;
            let embedded_asset_id = validate_attribution_fields(&attribution)?;

            if key_asset_id != embedded_asset_id {
                return Err(IclError::IntegrationError(format!(
                    "Attribution key {} does not match embedded asset id {}",
                    key, attribution.asset_id
                )));
            }

            let canonical_asset_id = key_asset_id.to_string();
            if !seen_asset_ids.insert(canonical_asset_id.clone()) {
                return Err(IclError::IntegrationError(format!(
                    "Duplicate canonical attribution asset id {}",
                    canonical_asset_id
                )));
            }
            if let Some(existing) = self.attribution_history.get(&canonical_asset_id) {
                if !attributions_match(existing, &attribution) {
                    return Err(IclError::IntegrationError(format!(
                        "Attribution for {} is immutable once recorded",
                        canonical_asset_id
                    )));
                }
                if !self.icae_data.contains_key(&canonical_asset_id) {
                    validated.push((canonical_asset_id, attribution));
                }
                continue;
            }

            validated.push((canonical_asset_id, attribution));
        }

        for (asset_id, attribution) in validated {
            self.attribution_history
                .insert(asset_id.clone(), attribution.clone());
            self.icae_data.insert(asset_id, attribution);
        }

        Ok(())
    }

    pub fn emit_to_financial_system(&self, event: &serde_json::Value) -> IclResult<bool> {
        if !event.is_object() {
            return Err(IclError::IntegrationError("Event must be an object".into()));
        }
        if self.financial_systems.is_empty() {
            return Err(IclError::IntegrationError(
                "No financial systems are configured".into(),
            ));
        }

        Err(IclError::IntegrationError(
            "No financial delivery connector is available to acknowledge the event".into(),
        ))
    }

    pub fn register_financial_system(&mut self, system: String) -> IclResult<()> {
        let system = system.trim();
        if system.is_empty() {
            return Err(IclError::IntegrationError(
                "Financial system name cannot be empty".into(),
            ));
        }

        if !self
            .financial_systems
            .iter()
            .any(|existing| existing == system)
        {
            self.financial_systems.push(system.to_string());
        }

        Ok(())
    }

    pub fn validate_attribution(
        &self,
        asset_id: Uuid,
        execution_details: &serde_json::Value,
    ) -> bool {
        let candidate = match serde_json::from_value::<ICAEAttribution>(execution_details.clone()) {
            Ok(candidate) => candidate,
            Err(_) => return false,
        };
        let candidate_asset_id = match validate_attribution_fields(&candidate) {
            Ok(candidate_asset_id) => candidate_asset_id,
            Err(_) => return false,
        };
        if candidate_asset_id != asset_id {
            return false;
        }

        let stored = match self.icae_data.get(&asset_id.to_string()) {
            Some(stored) => stored,
            None => return false,
        };
        let stored_asset_id = match validate_attribution_fields(stored) {
            Ok(stored_asset_id) => stored_asset_id,
            Err(_) => return false,
        };

        stored_asset_id == asset_id && attributions_match(stored, &candidate)
    }

    pub fn get_execution_attribution(&self, asset_id: Uuid) -> Option<&ICAEAttribution> {
        self.icae_data.get(&asset_id.to_string())
    }

    pub fn reconcile_with_financial_systems(&self) -> serde_json::Value {
        let status = if self.financial_systems.is_empty() {
            "not_configured"
        } else {
            "not_verified"
        };

        serde_json::json!({
            "status": status,
            "timestamp": Utc::now().to_rfc3339(),
            "attribution_count": self.icae_data.len(),
            "financial_system_count": self.financial_systems.len(),
        })
    }

    pub fn clear_attributions(&mut self) {
        self.icae_data.clear();
    }

    pub fn attribution_count(&self) -> usize {
        self.icae_data.len()
    }
}

impl Default for IntegrationAdapter {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_attribution_fields(attribution: &ICAEAttribution) -> IclResult<Uuid> {
    let asset_id = Uuid::parse_str(&attribution.asset_id).map_err(|_| {
        IclError::IntegrationError(format!(
            "Invalid embedded attribution asset id {}: must be a UUID",
            attribution.asset_id
        ))
    })?;

    if !attribution.inference_cost.is_finite() || attribution.inference_cost < 0.0 {
        return Err(IclError::IntegrationError(format!(
            "Invalid inference cost for {}: must be finite and non-negative",
            attribution.asset_id
        )));
    }
    if !attribution.execution_time.is_finite() || attribution.execution_time < 0.0 {
        return Err(IclError::IntegrationError(format!(
            "Invalid execution time for {}: must be finite and non-negative",
            attribution.asset_id
        )));
    }
    if attribution.model_version.trim().is_empty() {
        return Err(IclError::IntegrationError(format!(
            "Invalid model version for {}: cannot be empty",
            attribution.asset_id
        )));
    }

    Ok(asset_id)
}

fn attributions_match(stored: &ICAEAttribution, candidate: &ICAEAttribution) -> bool {
    let stored_asset_id = Uuid::parse_str(&stored.asset_id).ok();
    let candidate_asset_id = Uuid::parse_str(&candidate.asset_id).ok();

    stored_asset_id == candidate_asset_id
        && stored.inference_cost == candidate.inference_cost
        && stored.execution_time == candidate.execution_time
        && stored.timestamp == candidate.timestamp
        && stored.model_version == candidate.model_version
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attribution_value(
        asset_id: &str,
        inference_cost: f64,
        execution_time: f64,
        model_version: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "asset_id": asset_id,
            "inference_cost": inference_cost,
            "execution_time": execution_time,
            "timestamp": "2024-01-01T00:00:00Z",
            "model_version": model_version,
        })
    }

    fn attribution_batch(entries: Vec<(String, serde_json::Value)>) -> serde_json::Value {
        let object: serde_json::Map<String, serde_json::Value> = entries.into_iter().collect();
        serde_json::Value::Object(object)
    }

    #[test]
    fn attribution_batch_validation_is_atomic() {
        let mut adapter = IntegrationAdapter::new();
        let first_asset = Uuid::new_v4();
        let second_asset = Uuid::new_v4();
        let original = attribution_value(&first_asset.to_string(), 1.0, 2.0, "model-v1");

        adapter
            .consume_icae_attribution(&attribution_batch(vec![(
                first_asset.to_string(),
                original,
            )]))
            .unwrap();

        let invalid_batch = attribution_batch(vec![
            (
                first_asset.to_string(),
                attribution_value(&first_asset.to_string(), 99.0, 2.0, "model-v2"),
            ),
            (
                second_asset.to_string(),
                attribution_value(&second_asset.to_string(), 1.0, -1.0, "model-v1"),
            ),
        ]);

        assert!(adapter.consume_icae_attribution(&invalid_batch).is_err());
        assert_eq!(adapter.attribution_count(), 1);
        assert_eq!(
            adapter
                .get_execution_attribution(first_asset)
                .map(|attribution| attribution.inference_cost),
            Some(1.0)
        );
        assert!(adapter.get_execution_attribution(second_asset).is_none());
    }

    #[test]
    fn attribution_identity_is_canonical_and_immutable() {
        let mut adapter = IntegrationAdapter::new();
        let asset_id = Uuid::new_v4();
        let canonical = asset_id.to_string();
        let uppercase = canonical.to_uppercase();
        let original = attribution_value(&canonical, 1.0, 2.0, "model-v1");

        adapter
            .consume_icae_attribution(&attribution_batch(vec![(
                canonical.clone(),
                original.clone(),
            )]))
            .unwrap();
        adapter
            .consume_icae_attribution(&attribution_batch(vec![(canonical.clone(), original)]))
            .unwrap();

        let changed = attribution_value(&canonical, 2.0, 2.0, "model-v2");
        assert!(adapter
            .consume_icae_attribution(&attribution_batch(vec![(canonical.clone(), changed)]))
            .is_err());

        let aliases = attribution_batch(vec![
            (
                canonical.clone(),
                attribution_value(&canonical, 1.0, 2.0, "model-v1"),
            ),
            (
                uppercase.clone(),
                attribution_value(&uppercase, 1.0, 2.0, "model-v1"),
            ),
        ]);
        assert!(adapter.consume_icae_attribution(&aliases).is_err());
        assert_eq!(adapter.attribution_count(), 1);

        adapter.clear_attributions();
        assert_eq!(adapter.attribution_count(), 0);
        let restored = attribution_value(&asset_id.to_string(), 1.0, 2.0, "model-v1");
        adapter
            .consume_icae_attribution(&attribution_batch(vec![(canonical.clone(), restored)]))
            .unwrap();
        assert_eq!(adapter.attribution_count(), 1);
        assert!(adapter.get_execution_attribution(asset_id).is_some());

        adapter.clear_attributions();
        assert!(adapter
            .consume_icae_attribution(&attribution_batch(vec![(
                canonical,
                attribution_value(&asset_id.to_string(), 3.0, 2.0, "model-v3"),
            )]))
            .is_err());
    }

    #[test]
    fn attribution_identity_and_fields_are_validated() {
        let mut adapter = IntegrationAdapter::new();
        let outer_asset = Uuid::new_v4();
        let embedded_asset = Uuid::new_v4();

        let mismatch = attribution_batch(vec![(
            outer_asset.to_string(),
            attribution_value(&embedded_asset.to_string(), 1.0, 2.0, "model-v1"),
        )]);
        assert!(adapter.consume_icae_attribution(&mismatch).is_err());

        let malformed_id = attribution_batch(vec![(
            "not-a-uuid".to_string(),
            attribution_value("not-a-uuid", 1.0, 2.0, "model-v1"),
        )]);
        assert!(adapter.consume_icae_attribution(&malformed_id).is_err());

        let negative_cost = attribution_batch(vec![(
            outer_asset.to_string(),
            attribution_value(&outer_asset.to_string(), -1.0, 2.0, "model-v1"),
        )]);
        assert!(adapter.consume_icae_attribution(&negative_cost).is_err());

        let empty_model = attribution_batch(vec![(
            outer_asset.to_string(),
            attribution_value(&outer_asset.to_string(), 1.0, 2.0, "   "),
        )]);
        assert!(adapter.consume_icae_attribution(&empty_model).is_err());
        assert_eq!(adapter.attribution_count(), 0);

        let mut direct = ICAEAttribution {
            asset_id: outer_asset.to_string(),
            inference_cost: f64::NAN,
            execution_time: 2.0,
            timestamp: Utc::now(),
            model_version: "model-v1".to_string(),
        };
        assert!(validate_attribution_fields(&direct).is_err());
        direct.inference_cost = 1.0;
        direct.execution_time = f64::INFINITY;
        assert!(validate_attribution_fields(&direct).is_err());
    }

    #[test]
    fn validate_attribution_matches_execution_details_and_identity() {
        let mut adapter = IntegrationAdapter::new();
        let asset_id = Uuid::new_v4();
        let other_asset_id = Uuid::new_v4();
        let details = attribution_value(&asset_id.to_string(), 1.5, 2.5, "model-v1");

        adapter
            .consume_icae_attribution(&attribution_batch(vec![(
                asset_id.to_string(),
                details.clone(),
            )]))
            .unwrap();

        assert!(adapter.validate_attribution(asset_id, &details));
        assert!(!adapter.validate_attribution(other_asset_id, &details));

        let mut altered_details = details.clone();
        altered_details["execution_time"] = serde_json::json!(9.0);
        assert!(!adapter.validate_attribution(asset_id, &altered_details));

        let wrong_identity = attribution_value(&other_asset_id.to_string(), 1.5, 2.5, "model-v1");
        assert!(!adapter.validate_attribution(asset_id, &wrong_identity));
        assert!(!adapter.validate_attribution(asset_id, &serde_json::json!({})));
    }

    #[test]
    fn financial_operations_fail_closed_until_a_system_is_configured() {
        let mut adapter = IntegrationAdapter::new();
        let event = serde_json::json!({"event_id": Uuid::new_v4().to_string()});

        assert!(adapter.emit_to_financial_system(&event).is_err());
        assert!(adapter
            .emit_to_financial_system(&serde_json::json!("not-an-object"))
            .is_err());
        assert_eq!(
            adapter.reconcile_with_financial_systems()["status"],
            serde_json::json!("not_configured")
        );

        assert!(adapter
            .register_financial_system("   ".to_string())
            .is_err());
        adapter
            .register_financial_system("general-ledger".to_string())
            .unwrap();
        adapter
            .register_financial_system("general-ledger".to_string())
            .unwrap();

        assert!(adapter.emit_to_financial_system(&event).is_err());
        assert!(adapter
            .emit_to_financial_system(&serde_json::json!(null))
            .is_err());
        let reconciliation = adapter.reconcile_with_financial_systems();
        assert_eq!(reconciliation["status"], serde_json::json!("not_verified"));
        assert_eq!(
            reconciliation["financial_system_count"],
            serde_json::json!(1)
        );
    }
}
