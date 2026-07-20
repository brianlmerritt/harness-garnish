use crate::domain::ApiModelPrice;
use anyhow::{Result, bail};

const TOKEN_RATE_DENOMINATOR: u128 = 1_000_000;

pub fn calculate_api_cost_micros(
    price: &ApiModelPrice,
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_creation_input_tokens: u64,
    output_tokens: u64,
) -> Result<u64> {
    let categorized = cached_input_tokens
        .checked_add(cache_creation_input_tokens)
        .ok_or_else(|| anyhow::anyhow!("api.usage_overflow: categorized input tokens overflow"))?;
    if categorized > input_tokens {
        bail!("api.usage_inconsistent: cached and cache-creation tokens exceed input tokens");
    }
    let uncached_input_tokens = input_tokens - categorized;
    let numerator = u128::from(uncached_input_tokens)
        .checked_mul(u128::from(price.input_micros_per_million))
        .and_then(|value| {
            value.checked_add(
                u128::from(cached_input_tokens)
                    .checked_mul(u128::from(price.cached_input_micros_per_million))?,
            )
        })
        .and_then(|value| {
            value.checked_add(
                u128::from(cache_creation_input_tokens)
                    .checked_mul(u128::from(price.cache_creation_input_micros_per_million))?,
            )
        })
        .and_then(|value| {
            value.checked_add(
                u128::from(output_tokens)
                    .checked_mul(u128::from(price.output_micros_per_million))?,
            )
        })
        .ok_or_else(|| anyhow::anyhow!("api.cost_overflow: pricing multiplication overflow"))?;
    let rounded = numerator
        .checked_add(TOKEN_RATE_DENOMINATOR - 1)
        .ok_or_else(|| anyhow::anyhow!("api.cost_overflow: pricing rounding overflow"))?
        / TOKEN_RATE_DENOMINATOR;
    rounded
        .try_into()
        .map_err(|_| anyhow::anyhow!("api.cost_overflow: calculated cost exceeds u64"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn price() -> ApiModelPrice {
        ApiModelPrice {
            id: "price-fixture".into(),
            provider: "openai".into(),
            account: "default".into(),
            model: "model-fixture".into(),
            currency: "USD".into(),
            input_micros_per_million: 2_000_000,
            cached_input_micros_per_million: 500_000,
            cache_creation_input_micros_per_million: 2_500_000,
            output_micros_per_million: 8_000_000,
            effective_from: Utc::now(),
            effective_to: None,
            source: "fixture".into(),
            reason: "fixture".into(),
            created_at: Utc::now(),
            supersedes_id: None,
        }
    }

    #[test]
    fn exact_integer_cost_distinguishes_all_input_categories() {
        let result =
            calculate_api_cost_micros(&price(), 1_000_000, 200_000, 100_000, 50_000).unwrap();
        assert_eq!(result, 2_150_000);
    }

    #[test]
    fn fractional_micro_cost_rounds_up_once() {
        let mut price = price();
        price.input_micros_per_million = 1;
        price.cached_input_micros_per_million = 1;
        price.cache_creation_input_micros_per_million = 1;
        price.output_micros_per_million = 1;
        assert_eq!(calculate_api_cost_micros(&price, 1, 0, 0, 0).unwrap(), 1);
        assert_eq!(calculate_api_cost_micros(&price, 1, 0, 0, 1).unwrap(), 1);
    }

    #[test]
    fn inconsistent_categories_and_overflow_fail_closed() {
        assert!(calculate_api_cost_micros(&price(), 1, 1, 1, 0).is_err());
        let mut enormous = price();
        enormous.input_micros_per_million = u64::MAX;
        assert!(calculate_api_cost_micros(&enormous, u64::MAX, 0, 0, 0).is_err());
    }
}
