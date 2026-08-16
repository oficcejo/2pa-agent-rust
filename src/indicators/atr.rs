/// Average True Range (ATR) — full and incremental (Wilder smoothing).

#[derive(Debug, Clone, PartialEq)]
pub struct AtrState {
    pub last: f64,
    pub period: usize,
    pub count: usize,
    pub prev_close: f64,
    pub sum_tr: f64,
}

pub fn make_atr_state(period: usize) -> AtrState {
    AtrState {
        last: f64::NAN,
        period,
        count: 0,
        prev_close: f64::NAN,
        sum_tr: 0.0,
    }
}

pub fn true_range(high: f64, low: f64, prev_close: f64) -> f64 {
    let hl = (high - low).abs();
    if prev_close.is_nan() {
        return hl;
    }
    let hc = (high - prev_close).abs();
    let lc = (low - prev_close).abs();
    hl.max(hc).max(lc)
}

pub fn atr_full(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<f64> {
    if period < 1 || highs.len() != lows.len() || highs.len() != closes.len() {
        return vec![f64::NAN; highs.len()];
    }
    let n = highs.len();
    let mut result = vec![f64::NAN; n];
    if n < period {
        return result;
    }

    let mut trs = Vec::with_capacity(n);
    for i in 0..n {
        let prev_c = if i > 0 { closes[i - 1] } else { f64::NAN };
        trs.push(true_range(highs[i], lows[i], prev_c));
    }

    let seed: f64 = trs[..period].iter().sum::<f64>() / (period as f64);
    result[period - 1] = seed;
    let mut prev_atr = seed;

    for i in period..n {
        prev_atr = (prev_atr * (period as f64 - 1.0) + trs[i]) / (period as f64);
        result[i] = prev_atr;
    }
    result
}

pub fn atr_incremental(state: &AtrState, high: f64, low: f64, close: f64) -> AtrState {
    let period = state.period;
    let count = state.count + 1;
    let tr = true_range(high, low, state.prev_close);

    if count < period {
        AtrState {
            last: f64::NAN,
            period,
            count,
            prev_close: close,
            sum_tr: state.sum_tr + tr,
        }
    } else if count == period {
        let seed = (state.sum_tr + tr) / (period as f64);
        AtrState {
            last: seed,
            period,
            count,
            prev_close: close,
            sum_tr: 0.0,
        }
    } else {
        let new_last = (state.last * (period as f64 - 1.0) + tr) / (period as f64);
        AtrState {
            last: new_last,
            period,
            count,
            prev_close: close,
            sum_tr: 0.0,
        }
    }
}

pub fn state_after_atr(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> AtrState {
    let mut state = make_atr_state(period);
    for i in 0..highs.len() {
        state = atr_incremental(&state, highs[i], lows[i], closes[i]);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atr_full() {
        let highs = vec![10.0, 11.0, 12.0, 11.5];
        let lows = vec![9.0, 10.0, 11.0, 10.5];
        let closes = vec![9.5, 10.5, 11.5, 11.0];
        let atr = atr_full(&highs, &lows, &closes, 2);
        assert!(atr[0].is_nan());
        assert!(!atr[1].is_nan());
    }
}
