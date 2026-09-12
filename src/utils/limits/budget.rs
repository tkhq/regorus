// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::fmt;

/// Configuration for deterministic interpreter evaluation budgeting.
///
/// A budget unit represents one semantic interpreter checkpoint. A unit is
/// consumed when the interpreter enters an expression or statement dispatcher,
/// a query or rule body, a rule or user-function call, a builtin or extension
/// call, or a compound-expression helper (assignment, collection construction,
/// comprehension, or output assembly). A unit is also consumed for every loop
/// or comprehension iteration, every `with` modifier application, every
/// virtual-document lookup, and uncached rule work. A compound operation
/// can therefore consume multiple units as it reaches nested checkpoints. Nested
/// evaluation uses the same budget as its top-level evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvaluationBudgetConfig {
    /// Maximum number of semantic work units an evaluation may consume.
    pub limit: u64,
}

/// Metrics for the most recently started top-level interpreter evaluation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EvaluationMetrics {
    /// Semantic work units consumed, including the unit that exceeded a limit.
    pub consumed: u64,
}

/// Typed error returned when deterministic semantic work exceeds its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvaluationBudgetError {
    /// Work units consumed, including the unit that exceeded the limit.
    pub consumed: u64,
    /// Configured semantic work-unit limit.
    pub limit: u64,
}

impl fmt::Display for EvaluationBudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "evaluation exceeded deterministic work budget (consumed={}, limit={})",
            self.consumed, self.limit
        )
    }
}

impl core::error::Error for EvaluationBudgetError {}

/// Deterministic semantic-work tracker for one interpreter evaluation.
#[derive(Clone, Copy, Debug)]
pub struct EvaluationBudget {
    config: Option<EvaluationBudgetConfig>,
    consumed: u64,
    active: bool,
}

impl EvaluationBudget {
    pub(crate) const fn new(config: Option<EvaluationBudgetConfig>) -> Self {
        Self {
            config,
            consumed: 0,
            active: false,
        }
    }

    pub(crate) const fn config(&self) -> Option<EvaluationBudgetConfig> {
        self.config
    }

    pub(crate) const fn reset(&mut self) {
        self.consumed = 0;
        self.active = true;
    }

    pub(crate) const fn pause(&mut self) {
        self.active = false;
    }

    pub(crate) const fn metrics(&self) -> EvaluationMetrics {
        EvaluationMetrics {
            consumed: self.consumed,
        }
    }

    pub(crate) const fn consume(&mut self) -> core::result::Result<(), EvaluationBudgetError> {
        if !self.active {
            return Ok(());
        }
        self.consumed = self.consumed.saturating_add(1);
        if let Some(config) = self.config {
            if self.consumed > config.limit {
                return Err(EvaluationBudgetError {
                    consumed: self.consumed,
                    limit: config.limit,
                });
            }
        }
        Ok(())
    }
}
