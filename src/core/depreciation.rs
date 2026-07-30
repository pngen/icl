use crate::core::error::*;
use crate::core::types::*;
use chrono::{DateTime, Datelike, Months, Utc};

pub fn calculate_depreciation(
    asset: &IntelligenceAsset,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    salvage_value: f64,
    rate_multiplier: f64,
) -> IclResult<(f64, f64)> {
    if start_date >= end_date {
        return Err(IclError::InvalidDateRange {
            start: start_date.to_rfc3339(),
            end: end_date.to_rfc3339(),
        });
    }
    if start_date < asset.created_at {
        return Err(IclError::DepreciationError(
            "Depreciation period cannot begin before asset capitalization".into(),
        ));
    }

    if !asset.initial_value.is_finite() || asset.initial_value <= 0.0 {
        return Err(IclError::DepreciationError(
            "Initial value must be positive".into(),
        ));
    }

    if asset.useful_life_months <= 0 {
        return Err(IclError::DepreciationError(
            "Useful life must be positive".into(),
        ));
    }

    let current_value = asset.current_value.unwrap_or(asset.initial_value);
    if !current_value.is_finite() || current_value < 0.0 {
        return Err(IclError::DepreciationError(
            "Current value must be finite and non-negative".into(),
        ));
    }

    if !salvage_value.is_finite() || salvage_value < 0.0 {
        return Err(IclError::DepreciationError(
            "Salvage value cannot be negative".into(),
        ));
    }

    if salvage_value > asset.initial_value {
        return Err(IclError::DepreciationError(
            "Salvage value cannot exceed initial value".into(),
        ));
    }

    if salvage_value > current_value {
        return Err(IclError::DepreciationError(
            "Salvage value cannot exceed current carrying value".into(),
        ));
    }

    let (depreciation_amount, new_value) = match asset.depreciation_method {
        DepreciationMethod::Linear => {
            linear_depreciation(asset, start_date, end_date, salvage_value)
        }
        DepreciationMethod::DecliningBalance => declining_balance_depreciation(
            asset,
            start_date,
            end_date,
            salvage_value,
            rate_multiplier,
        ),
    }?;

    if !depreciation_amount.is_finite() || depreciation_amount < 0.0 {
        return Err(IclError::DepreciationError(
            "Depreciation amount must be finite and non-negative".into(),
        ));
    }

    if !new_value.is_finite() || new_value < 0.0 {
        return Err(IclError::DepreciationError(
            "New carrying value must be finite and non-negative".into(),
        ));
    }

    if new_value > current_value {
        return Err(IclError::DepreciationError(
            "Depreciation cannot increase current carrying value".into(),
        ));
    }

    Ok((depreciation_amount, new_value))
}

/// Calendar months between two instants, including a deterministic fraction of
/// the next calendar month for partial periods.
fn month_position(anchor: DateTime<Utc>, instant: DateTime<Utc>) -> f64 {
    let rough_months = ((instant.year() - anchor.year()) * 12 + instant.month() as i32
        - anchor.month() as i32)
        .max(0) as u32;
    let mut whole_months = rough_months;
    while whole_months > 0
        && anchor
            .checked_add_months(Months::new(whole_months))
            .is_none_or(|candidate| candidate > instant)
    {
        whole_months -= 1;
    }
    while anchor
        .checked_add_months(Months::new(whole_months.saturating_add(1)))
        .is_some_and(|candidate| candidate <= instant)
    {
        whole_months = whole_months.saturating_add(1);
    }

    let period_start = match anchor.checked_add_months(Months::new(whole_months)) {
        Some(period_start) => period_start,
        None => return whole_months as f64,
    };
    if period_start >= instant {
        return whole_months as f64;
    }
    let next_anchor = match anchor.checked_add_months(Months::new(whole_months.saturating_add(1))) {
        Some(next_anchor) => next_anchor,
        None => return whole_months as f64,
    };
    let month_seconds = match (next_anchor - period_start).to_std() {
        Ok(duration) => duration.as_secs_f64(),
        Err(_) => return whole_months as f64,
    };
    let partial_seconds = match (instant - period_start).to_std() {
        Ok(duration) => duration.as_secs_f64(),
        Err(_) => return whole_months as f64,
    };
    whole_months as f64 + (partial_seconds / month_seconds).clamp(0.0, 1.0)
}

fn months_between(anchor: DateTime<Utc>, start: DateTime<Utc>, end: DateTime<Utc>) -> f64 {
    month_position(anchor, end) - month_position(anchor, start)
}

fn linear_depreciation(
    asset: &IntelligenceAsset,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    salvage_value: f64,
) -> IclResult<(f64, f64)> {
    let months = months_between(asset.created_at, start_date, end_date);

    if months <= 0.0 {
        return Ok((0.0, asset.current_value.unwrap_or(asset.initial_value)));
    }

    let depreciable_base = asset.initial_value - salvage_value;
    let monthly_rate = 1.0 / asset.useful_life_months as f64;
    let max_depreciation = depreciable_base * monthly_rate * months;

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
    rate_multiplier: f64,
) -> IclResult<(f64, f64)> {
    let months = months_between(asset.created_at, start_date, end_date);

    if months <= 0.0 {
        return Ok((0.0, asset.current_value.unwrap_or(asset.initial_value)));
    }

    if !rate_multiplier.is_finite() || rate_multiplier <= 0.0 {
        return Err(IclError::DepreciationError(
            "Rate multiplier must be positive".into(),
        ));
    }

    let rate = rate_multiplier / asset.useful_life_months as f64;
    if !rate.is_finite() || rate >= 1.0 {
        return Err(IclError::DepreciationError(
            "Declining-balance monthly rate must be less than one".into(),
        ));
    }
    let current_value = asset.current_value.unwrap_or(asset.initial_value);
    let projected_value = current_value * (1.0 - rate).powf(months);
    let new_value = projected_value.max(salvage_value);
    let depreciation_amount = current_value - new_value;
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
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            status: AssetStatus::Active,
            current_value: Some(12000.0),
        }
    }

    #[test]
    fn test_months_between() {
        let start = Utc.with_ymd_and_hms(2024, 1, 15, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 7, 15, 0, 0, 0).unwrap();
        assert_eq!(months_between(start, start, end), 6.0);

        let month_end_start = Utc.with_ymd_and_hms(2024, 1, 31, 0, 0, 0).unwrap();
        let month_end = Utc.with_ymd_and_hms(2024, 2, 29, 0, 0, 0).unwrap();
        assert_eq!(
            months_between(month_end_start, month_end_start, month_end),
            1.0
        );

        let partial_end = Utc.with_ymd_and_hms(2024, 1, 31, 0, 0, 0).unwrap();
        let partial = months_between(
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            partial_end,
        );
        assert!(partial > 0.9 && partial < 1.0);

        let short_start = Utc.with_ymd_and_hms(2024, 1, 31, 23, 59, 0).unwrap();
        let short_crossing = months_between(
            short_start,
            short_start,
            Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap(),
        );
        assert!(short_crossing > 0.0 && short_crossing < 0.01);

        let anchor = Utc.with_ymd_and_hms(2024, 1, 31, 0, 0, 0).unwrap();
        let split = Utc.with_ymd_and_hms(2024, 2, 29, 0, 0, 0).unwrap();
        let finish = Utc.with_ymd_and_hms(2024, 3, 30, 0, 0, 0).unwrap();
        let unsplit = months_between(anchor, anchor, finish);
        let partitioned =
            months_between(anchor, anchor, split) + months_between(anchor, split, finish);
        assert!((unsplit - partitioned).abs() < 1e-12);
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

    #[test]
    fn test_declining_balance_rejects_invalid_rate_multiplier() {
        let mut asset = test_asset();
        asset.depreciation_method = DepreciationMethod::DecliningBalance;
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();

        assert!(calculate_depreciation(&asset, start, end, 0.0, -1.0).is_err());
        assert!(calculate_depreciation(&asset, start, end, 0.0, f64::NAN).is_err());
    }

    #[test]
    fn test_depreciation_rejects_non_finite_values() {
        let mut asset = test_asset();
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();

        asset.current_value = Some(f64::NAN);
        assert!(calculate_depreciation(&asset, start, end, 0.0, 2.0).is_err());

        asset.current_value = Some(asset.initial_value);
        assert!(calculate_depreciation(&asset, start, end, f64::INFINITY, 2.0).is_err());
    }

    #[test]
    fn test_depreciation_rejects_non_positive_useful_life() {
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();

        for useful_life_months in [0, -1] {
            let mut asset = test_asset();
            asset.useful_life_months = useful_life_months;

            assert!(calculate_depreciation(&asset, start, end, 0.0, 2.0).is_err());
        }
    }

    #[test]
    fn test_depreciation_rejects_salvage_above_current_value() {
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();

        for method in [
            DepreciationMethod::Linear,
            DepreciationMethod::DecliningBalance,
        ] {
            let mut asset = test_asset();
            asset.depreciation_method = method;
            asset.current_value = Some(100.0);

            assert!(calculate_depreciation(&asset, start, end, 500.0, 2.0).is_err());
        }
    }

    #[test]
    fn test_depreciation_outputs_do_not_increase_current_value() {
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();

        for method in [
            DepreciationMethod::Linear,
            DepreciationMethod::DecliningBalance,
        ] {
            let mut asset = test_asset();
            asset.depreciation_method = method;
            asset.current_value = Some(1_000.0);

            let (depreciation_amount, new_value) =
                calculate_depreciation(&asset, start, end, 100.0, 2.0).unwrap();

            assert!(depreciation_amount.is_finite());
            assert!(depreciation_amount >= 0.0);
            assert!(new_value.is_finite());
            assert!(new_value >= 0.0);
            assert!(new_value <= asset.current_value.unwrap());
        }
    }
}
