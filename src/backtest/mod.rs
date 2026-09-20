//! Production-grade backtesting system for Price Action trading systems.
//! Features Programmatic Gating, closed HTF resampling without lookahead bias,
//! persistent decision caching, OKX contract specs simulation, and deep quantitative metrics.

pub mod account;
pub mod cache;
pub mod data;
pub mod engine;
pub mod matcher;
pub mod types;

pub use account::{PositionSide, VirtualAccount, VirtualPosition};
pub use cache::{CachedDecision, DecisionCache};
pub use data::{detect_gaps, fetch_candles_okx, generate_synthetic_candles, generate_synthetic_candles_for_strategy, load_candles_from_file, timeframe_to_ms};
pub use engine::BacktestEngine;
pub use matcher::{OrderMatcher, PendingOrder};
pub use types::{
    BacktestConfig, BacktestDataSource, BacktestJobStatus, BacktestMetrics, BacktestReport,
    BacktestTrade, EquityPoint,
};
