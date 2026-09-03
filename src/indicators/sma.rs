/// Simple Moving Average (SMA) — full and incremental.

#[derive(Debug, Clone, PartialEq)]
pub struct SmaState {
    pub period: usize,
    pub window: Vec<f64>,
}

pub fn make_sma_state(period: usize) -> SmaState {
    SmaState {
        period,
        window: Vec::with_capacity(period),
    }
}

pub fn sma_full(values: &[f64], period: usize) -> Vec<f64> {
    if period < 1 {
        return vec![f64::NAN; values.len()];
    }
    let n = values.len();
    let mut result = vec![f64::NAN; n];
    if n < period {
        return result;
    }

    let mut sum: f64 = values[..period].iter().sum();
    result[period - 1] = sum / (period as f64);

    for i in period..n {
        sum += values[i] - values[i - period];
        result[i] = sum / (period as f64);
    }
    result
}

pub fn sma_incremental(state: &mut SmaState, x: f64) -> f64 {
    if state.period == 0 {
        return f64::NAN;
    }
    state.window.push(x);
    if state.window.len() > state.period {
        state.window.remove(0);
    }
    if state.window.len() == state.period {
        let sum: f64 = state.window.iter().sum();
        sum / (state.period as f64)
    } else {
        f64::NAN
    }
}

/// Calculate the per-bar slope of SMA over a lookback window (e.g. 5 or 10 bars)
pub fn sma_slope(sma_values: &[f64], lookback: usize) -> Vec<f64> {
    let n = sma_values.len();
    let mut slopes = vec![f64::NAN; n];
    if lookback == 0 || n <= lookback {
        return slopes;
    }
    for i in lookback..n {
        let cur = sma_values[i];
        let prev = sma_values[i - lookback];
        if !cur.is_nan() && !prev.is_nan() {
            slopes[i] = (cur - prev) / (lookback as f64);
        }
    }
    slopes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sma_full() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let sma3 = sma_full(&data, 3);
        assert!(sma3[0].is_nan());
        assert!(sma3[1].is_nan());
        assert!((sma3[2] - 2.0).abs() < 1e-6); // mean(1,2,3) = 2.0
        assert!((sma3[3] - 3.0).abs() < 1e-6); // mean(2,3,4) = 3.0
        assert!((sma3[4] - 4.0).abs() < 1e-6); // mean(3,4,5) = 4.0
    }
}
