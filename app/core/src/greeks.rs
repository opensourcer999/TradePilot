pub const IV_MAX: f64 = 5.0;
/// How close the implied volatility search gets (the width of its last interval).
pub const IV_TOLERANCE: f64 = 0.0001;

pub fn norm_cdf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x < -8.0 {
        return 0.0;
    }
    if x > 8.0 {
        return 1.0;
    }
    let mut denominator = 3.0;
    let mut sum = 0.0;
    let mut term = x;
    while (sum + term).total_cmp(&sum).is_ne() {
        sum += term;
        term = term * x * x / denominator;
        denominator += 2.0;
    }
    let density = (-x * x / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt();
    0.5 + sum * density
}

pub fn bs_d1(spot: f64, strike: f64, t: f64, vol: f64, rate: f64, dividend: f64) -> f64 {
    ((spot / strike).ln() + (rate - dividend + vol * vol / 2.0) * t) / (vol * t.sqrt())
}

pub fn bs_price(spot: f64, strike: f64, t: f64, vol: f64, rate: f64, dividend: f64, is_put: bool) -> f64 {
    let d1 = bs_d1(spot, strike, t, vol, rate, dividend);
    let d2 = d1 - vol * t.sqrt();
    let spot_pv = spot * (-dividend * t).exp();
    let strike_pv = strike * (-rate * t).exp();
    if is_put {
        strike_pv * norm_cdf(-d2) - spot_pv * norm_cdf(-d1)
    } else {
        spot_pv * norm_cdf(d1) - strike_pv * norm_cdf(d2)
    }
}

pub fn bs_delta(spot: f64, strike: f64, t: f64, vol: f64, rate: f64, dividend: f64, is_put: bool) -> f64 {
    let d1 = bs_d1(spot, strike, t, vol, rate, dividend);
    let call_delta = norm_cdf(d1);
    if is_put {
        call_delta - 1.0
    } else {
        call_delta
    }
}

pub fn implied_vol(spot: f64, strike: f64, t: f64, rate: f64, dividend: f64, market_price: f64, is_put: bool) -> f64 {
    let (mut low, mut high) = (0.0, IV_MAX);
    while high - low > IV_TOLERANCE {
        let mid = f64::midpoint(low, high);
        if bs_price(spot, strike, t, mid, rate, dividend, is_put) > market_price {
            high = mid;
        } else {
            low = mid;
        }
    }
    f64::midpoint(high, low)
}

/// The implied volatility and the delta of an option (Black-Scholes) priced at `price`: `spot`
/// is the underlying's price, `days` the time to expiration in years (the formula's `t`, despite
/// the name), `rate` and `dividend` as yearly fractions. Returns `(volatility, delta)`; the
/// volatility is searched between 0 and `IV_MAX`.
#[must_use]
pub fn calculate_greeks(spot: f64, strike: f64, days: f64, price: f64, rate: f64, dividend: f64, is_put: bool) -> (f64, f64) {
    let volatility = implied_vol(spot, strike, days, rate, dividend, price, is_put);
    let delta = bs_delta(spot, strike, days, volatility, rate, dividend, is_put);
    (volatility, delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator_uses_elapsed_days_without_year_conversion() {
        let (volatility, delta) = calculate_greeks(100.0, 100.0, 1.0, 7.965_567_455, 0.0, 0.0, false);
        assert!((volatility - 0.2).abs() <= IV_TOLERANCE);
        assert!((delta - 0.539_827_837_3).abs() < 0.0001);
    }

    #[test]
    fn cdf_and_delta_follow_the_documented_rules() {
        assert_eq!(norm_cdf(-8.1), 0.0);
        assert_eq!(norm_cdf(8.1), 1.0);
        assert_eq!(norm_cdf(0.0), 0.5);
        assert!((norm_cdf(1.0) - 0.841_344_746_068_542_9).abs() < 1e-14);
        assert!(norm_cdf(f64::NAN).is_nan());
        let d1 = bs_d1(100.0, 100.0, 1.0, 0.2, 0.05, 0.1);
        let call = bs_delta(100.0, 100.0, 1.0, 0.2, 0.05, 0.1, false);
        let put = bs_delta(100.0, 100.0, 1.0, 0.2, 0.05, 0.1, true);
        assert_eq!(call, norm_cdf(d1));
        assert_eq!(put, call - 1.0);
    }
}
