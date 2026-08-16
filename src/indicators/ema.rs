/// Exponential Moving Average (EMA) — full and incremental.

#[derive(Debug, Clone, PartialEq)]
pub struct EmaState {
    pub last: f64,
    pub period: usize,
    pub count: usize,
    pub sum: f64,
}

pub fn make_ema_state(period: usize) -> EmaState {
    EmaState {
        last: f64::NAN,
        period,
        count: 0,
        sum: 0.0,
    }
}

pub fn ema_full(values: &[f64], period: usize) -> Vec<f64> {
    if period < 1 {
        return vec![f64::NAN; values.len()];
    }
    let n = values.len();
    let mut result = vec![f64::NAN; n];
    if n < period {
        return result;
    }

    let alpha = 2.0 / (period as f64 + 1.0);
    let seed: f64 = values[..period].iter().sum::<f64>() / (period as f64);
    result[period - 1] = seed;
    let mut prev = seed;

    for i in period..n {
        prev = values[i] * alpha + prev * (1.0 - alpha);
        result[i] = prev;
    }
    result
}

pub fn ema_incremental(state: &EmaState, x: f64) -> EmaState {
    let period = state.period;
    let count = state.count + 1;
    let alpha = 2.0 / (period as f64 + 1.0);

    if count < period {
        EmaState {
            last: f64::NAN,
            period,
            count,
            sum: state.sum + x,
        }
    } else if count == period {
        let seed = (state.sum + x) / (period as f64);
        EmaState {
            last: seed,
            period,
            count,
            sum: 0.0,
        }
    } else {
        let new_last = x * alpha + state.last * (1.0 - alpha);
        EmaState {
            last: new_last,
            period,
            count,
            sum: 0.0,
        }
    }
}

pub fn state_after(values: &[f64], period: usize) -> EmaState {
    let mut state = make_ema_state(period);
    for &v in values {
        state = ema_incremental(&state, v);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ema_full() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let ema = ema_full(&data, 3);
        assert!(ema[0].is_nan());
        assert!(ema[1].is_nan());
        assert!((ema[2] - 2.0).abs() < 1e-6); // mean(1,2,3) = 2.0
        // alpha = 2 / 4 = 0.5
        // ema[3] = 4 * 0.5 + 2.0 * 0.5 = 3.0
        assert!((ema[3] - 3.0).abs() < 1e-6);
        // ema[4] = 5 * 0.5 + 3.0 * 0.5 = 4.0
        assert!((ema[4] - 4.0).abs() < 1e-6);
    }
}
