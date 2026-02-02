use chrono::{DateTime, Utc, Datelike};
use crate::core::types::*;
use crate::core::error::*;

pub fn calculate_depreciation(
    asset: &IntelligenceAsset,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    salvage_value: f64,
    rate_multiplier: f64
) -> IclResult<(f64, f64)> {
    if start_date >= end_date {
        return Err(IclError::InvalidDateRange {
            start: start_date.to_rfc3339(),
            end: end_date.to_rfc3339(),
        });
    }

    if salvage_value < 0.0 {
        return Err(IclError::DepreciationError("Salvage value cannot be negative".into()));
    }

    if salvage_value > asset.initial_value {
        return Err(IclError::DepreciationError("Salvage value cannot exceed initial value".into()));
    }

    match asset.depreciation_method {
        DepreciationMethod::Linear => {
            linear_depreciation(asset, start_date, end_date, salvage_value)
        },
        DepreciationMethod::DecliningBalance => {
            declining_balance_depreciation(asset, start_date, end_date, salvage_value, rate_multiplier)
        },
    }
}

/// Calculate months between two dates
fn months_between(start: DateTime<Utc>, end: DateTime<Utc>) -> i32 {
    let years = end.year() - start.year();
    let months = end.month() as i32 - start.month() as i32;
    let total_months = years * 12 + months;
    
    // Adjust for partial months
    if end.day() < start.day() {
        (total_months - 1).max(0)
    } else {
        total_months.max(0)
    }
}

fn linear_depreciation(
    asset: &IntelligenceAsset,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    salvage_value: f64
) -> IclResult<(f64, f64)> {
    let months = months_between(start_date, end_date);
    
    if months <= 0 {
        return Ok((0.0, asset.current_value.unwrap_or(asset.initial_value)));
    }

    let depreciable_base = asset.initial_value - salvage_value;
    let monthly_rate = 1.0 / asset.useful_life_months as f64;
    let max_depreciation = depreciable_base * monthly_rate * months as f64;
    
    let current = asset.current_value.unwrap_or(asset.initial_value);
    let depreciation_amount = max_depreciation.min(current - salvage_value).max(0.0);
    let new_value = (current - depreciation_amount).max(salvage_value);
    
    Ok((depreciation_amount, new_value))
}

fn declining_balance_depreciation(
    asset: &IntelligenceAsset,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    salvage_value: f64,
    rate_multiplier: f64
) -> IclResult<(f64, f64)> {
    let months = months_between(start_date, end_date);
    
    if months <= 0 {
        return Ok((0.0, asset.current_value.unwrap_or(asset.initial_value)));
    }

    let rate = rate_multiplier / asset.useful_life_months as f64;
    let mut current_value = asset.current_value.unwrap_or(asset.initial_value);
    
    let mut depreciation_amount = 0.0;
    for _ in 0..months {
        let monthly_depreciation = current_value * rate;
        if current_value - monthly_depreciation < salvage_value {
            depreciation_amount += current_value - salvage_value;
            current_value = salvage_value;
            break;
        } else {
            depreciation_amount += monthly_depreciation;
            current_value -= monthly_depreciation;
        }
    }
    
    let new_value = current_value.max(salvage_value);
    Ok((depreciation_amount, new_value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn test_asset() -> IntelligenceAsset {
        IntelligenceAsset {
            asset_id: uuid::Uuid::new_v4(),
            owner: "Test".into(),
            initial_value: 12000.0,
            depreciation_method: DepreciationMethod::Linear,
            useful_life_months: 12,
            created_at: Utc::now(),
            status: AssetStatus::Active,
            current_value: Some(12000.0),
        }
    }

    #[test]
    fn test_months_between() {
        let start = Utc.with_ymd_and_hms(2024, 1, 15, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 7, 15, 0, 0, 0).unwrap();
        assert_eq!(months_between(start, end), 6);
    }

    #[test]
    fn test_linear_depreciation() {
        let asset = test_asset();
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 7, 1, 0, 0, 0).unwrap();
        let (dep, new_val) = calculate_depreciation(&asset, start, end, 0.0, 2.0).unwrap();
        assert!((dep - 6000.0).abs() < 0.01);
        assert!((new_val - 6000.0).abs() < 0.01);
    }
}