//! Continual-learning layer.
//!
//! Closes the four-step loop borrowed from Reef's design — serve, observe,
//! grow, commit — on top of the existing decision/audit records:
//!
//! * **Serve** — already existed: analysis produces a decision, the executor
//!   submits it and appends an audit row.
//! * **Observe** (`reconciler`, `feedback`, `outcome_store`) — resolves each
//!   submitted order against the venue and records a structured result: realised
//!   R, MFE/MAE, holding time, fees and exit reason.
//! * **Grow** (`experience_writer`) — promotes qualified results into the
//!   experience library that the strategist prompt reads, replacing the
//!   hand-maintained case files.
//! * **Commit** (`artifact`, `evaluation`) — prompts are versioned artifacts,
//!   and a candidate revision must not degrade the incumbent before it may be
//!   activated. Activation is always a separate human step: nothing here
//!   promotes a strategy to live traffic on its own.
//!
//! Two deliberate departures from Reef: there is no online weight training
//! (this is an execution client, not a trainer), and no automatic hot-swap of
//! live strategy behaviour, because a strategy that silently changes under
//! open positions is a risk, not a feature.

pub mod artifact;
pub mod evaluation;
pub mod experience_writer;
pub mod feedback;
pub mod outcome_store;
pub mod reconciler;

pub use artifact::{ActivePrompt, PromptArtifact, PromptArtifactStore, PromptIndex};
pub use evaluation::{
    compare_candidate, evaluate_versions, group_by_prompt_version, metrics_for, EvaluationPolicy,
    StrategyMetrics, Verdict,
};
pub use experience_writer::ExperienceWriter;
pub use feedback::{
    qualify, ExitReason, Excursion, QualificationPolicy, TradeOutcome, PNL_SOURCE_BROKER,
    PNL_SOURCE_MODEL,
};
pub use outcome_store::OutcomeStore;
pub use reconciler::{
    build_outcome, classify_management, ReconcileReport, Reconciler, ResolutionInput, Resolved,
    SignalContext,
};

/// Default prompt artifact name used by the strategy protocol.
pub const STRATEGY_PROMPT_NAME: &str = "strategy_v1";
