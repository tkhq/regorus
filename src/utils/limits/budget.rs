// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::fmt;

/// Version of the deterministic evaluation-work accounting contract.
///
/// Version 1 charges one unit for scalar values, plus a number's binary magnitude bytes or a
/// string's UTF-8 bytes. It charges one plus the element count plus child weights for arrays and
/// sets, and one plus twice the entry count plus key and value weights for objects. Collection
/// iteration charges one setup unit, then one slot plus the visited element weight, or two slots
/// plus visited key and value weights. References and cache hits charge their cloned scalar or one
/// shared-value unit instead of the full parent. Builtins charge argument-reference weights, a
/// declared preflight projection, and the actual result weight. Regex projections saturatingly
/// charge pattern² compilation, pattern × haystack search, and operation-specific output bounds.
/// Arithmetic charges a magnitude-based projection before evaluation. Equality and ordering charge
/// corresponding structure up to the smaller operand, capped at the current remaining budget plus
/// one. Membership scans charge visited slots and comparisons; BTree lookup charges
/// `11 * (ceil(log2(n)) + 1) * needle comparison bound`, covering every key in a Rust BTree
/// node with `B = 6`. Set operators charge input traversal,
/// `(n + m) * ceil(log2(n + m + 1)) * maximum element comparison bound`, and worst-case
/// result structure before execution, then charge the actual result. A budgeted call is rejected
/// before dispatch when its builtin or extension has no declared estimator. Version 1 describes
/// the unreleased accounting contract and can change until its first release.
pub const EVALUATION_ACCOUNTING_VERSION: u32 = 1;

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
    /// Semantic work units consumed, including a charge that exceeded a limit.
    pub consumed: u64,
}

/// Typed error returned when deterministic semantic work exceeds its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvaluationBudgetError {
    /// Work units consumed, including a charge that exceeded the limit.
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

    pub(crate) const fn is_limited(&self) -> bool {
        self.active && self.config.is_some()
    }

    pub(crate) const fn remaining(&self) -> Option<u64> {
        match (self.active, self.config) {
            (true, Some(config)) => Some(config.limit.saturating_sub(self.consumed)),
            _ => None,
        }
    }

    pub(crate) const fn consume(&mut self) -> core::result::Result<(), EvaluationBudgetError> {
        self.consume_n(1)
    }

    pub(crate) const fn consume_n(
        &mut self,
        units: u64,
    ) -> core::result::Result<(), EvaluationBudgetError> {
        if !self.active || units == 0 {
            return Ok(());
        }
        self.consumed = self.consumed.saturating_add(units);
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
