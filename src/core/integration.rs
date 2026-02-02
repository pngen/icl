use uuid::Uuid;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    financial_systems: Vec<String>, // Simulated financial systems
}

impl IntegrationAdapter {
    pub fn new() -> Self {
        Self {
            icae_data: std::collections::HashMap::new(),
            financial_systems: vec![],
        }
    }

    pub fn consume_icae_attribution(&mut self, attribution_data: &serde_json::Value) -> IclResult<()> {
        // In a real implementation, we'd validate and process the data
        if let Some(obj) = attribution_data.as_object() {
            for (key, value) in obj {
                if let Ok(attribution) = serde_json::from_value::<ICAEAttribution>(value.clone()) {
                    if attribution.inference_cost < 0.0 {
                        return Err(IclError::IntegrationError(
                            format!("Invalid inference cost for {}: must be non-negative", key)
                        ));
                    }
                    self.icae_data.insert(key.clone(), attribution);
                } else {
                    return Err(IclError::IntegrationError(
                        format!("Invalid attribution data format for {}", key)
                    ));
                }
            }
            Ok(())
        } else {
            Err(IclError::IntegrationError("Attribution data must be an object".into()))
        }
    }

    pub fn emit_to_financial_system(&self, event: &serde_json::Value) -> IclResult<bool> {
        if event.is_null() {
            return Err(IclError::IntegrationError("Event cannot be null".into()));
        }
        // Production: integrate with actual financial systems
        Ok(true)
    }

    pub fn validate_attribution(&self, asset_id: Uuid, _execution_details: &serde_json::Value) -> bool {
        self.icae_data.contains_key(&asset_id.to_string())
    }

    pub fn get_execution_attribution(&self, asset_id: Uuid) -> Option<&ICAEAttribution> {
        self.icae_data.get(&asset_id.to_string())
    }

    pub fn reconcile_with_financial_systems(&self) -> serde_json::Value {
        serde_json::json!({
            "status": "reconciled",
            "timestamp": Utc::now().to_rfc3339(),
            "attribution_count": self.icae_data.len(),
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