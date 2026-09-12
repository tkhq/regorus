// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use alloc::string::ToString as _;
use anyhow::Result;

use crate::utils::limits::{EvaluationBudgetConfig, EvaluationBudgetError};
use crate::{Engine, Value};

fn engine_with_policy(policy: &str) -> Result<Engine> {
    let mut engine = Engine::new();
    engine.add_policy("budget.rego".to_string(), policy.to_string())?;
    Ok(engine)
}

fn budget_error(error: &anyhow::Error) -> EvaluationBudgetError {
    *error
        .downcast_ref::<EvaluationBudgetError>()
        .expect("budget failures retain their typed error")
}

#[test]
fn evaluation_work_is_repeatable_and_resets_between_calls() -> Result<()> {
    let mut engine = engine_with_policy(
        r#"
package budget

increment(x) := x + 1
nested(x) := increment(x)
answer := nested(input.value)
"#,
    )?;
    engine.set_input(Value::from_json_str(r#"{"value": 4}"#)?);

    let first = engine.eval_rule("data.budget.answer".to_string())?;
    let first_metrics = engine.evaluation_metrics();
    let second = engine.eval_rule("data.budget.answer".to_string())?;
    let second_metrics = engine.evaluation_metrics();

    assert_eq!(first, Value::from(5));
    assert_eq!(second, first);
    assert!(first_metrics.consumed > 0);
    assert_eq!(second_metrics, first_metrics);

    engine.set_evaluation_budget_config(EvaluationBudgetConfig {
        limit: first_metrics.consumed,
    });
    assert_eq!(
        engine.eval_rule("data.budget.answer".to_string())?,
        Value::from(5)
    );
    assert_eq!(engine.evaluation_metrics(), first_metrics);
    Ok(())
}

#[test]
fn low_budget_fails_deterministically_and_nested_calls_share_it() -> Result<()> {
    let mut engine = engine_with_policy(
        r#"
package budget

leaf(x) := x + 1
middle(x) := leaf(x)
answer := middle(input.value)
"#,
    )?;
    engine.set_input(Value::from_json_str(r#"{"value": 10}"#)?);
    engine.eval_rule("data.budget.answer".to_string())?;
    let required = engine.evaluation_metrics().consumed;
    let limit = required.saturating_sub(1);
    engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit });

    let first = engine
        .eval_rule("data.budget.answer".to_string())
        .expect_err("nested evaluation must exhaust the shared parent budget");
    let first_error = budget_error(&first);
    let first_metrics = engine.evaluation_metrics();
    let second = engine
        .eval_rule("data.budget.answer".to_string())
        .expect_err("the same work must fail again");

    assert_eq!(budget_error(&second), first_error);
    assert_eq!(engine.evaluation_metrics(), first_metrics);
    assert_eq!(
        first_error,
        EvaluationBudgetError {
            consumed: limit.saturating_add(1),
            limit,
        }
    );
    Ok(())
}

#[test]
fn comprehension_work_grows_with_iterations() -> Result<()> {
    let mut engine = engine_with_policy(
        r#"
package budget

values := [(x * 2) | x := input.values[_]]
"#,
    )?;

    engine.set_input(Value::from_json_str(r#"{"values": [1]}"#)?);
    engine.eval_rule("data.budget.values".to_string())?;
    let one_item = engine.evaluation_metrics().consumed;

    engine.set_input(Value::from_json_str(r#"{"values": [1, 2, 3, 4, 5]}"#)?);
    engine.eval_rule("data.budget.values".to_string())?;
    let five_items = engine.evaluation_metrics().consumed;

    assert!(five_items > one_item);
    assert!(five_items.saturating_sub(one_item) >= 4);
    Ok(())
}

#[test]
fn with_state_is_restored_after_budget_error_and_next_call_is_fresh() -> Result<()> {
    let mut engine = engine_with_policy(
        r#"
package budget

answer := input.value
"#,
    )?;
    engine.set_input(Value::from_json_str(r#"{"value": 7}"#)?);

    // The modifier application and replacement expression consume units 4 and 5.
    // Unit 6 enters nested evaluation, after the override is installed.
    engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 5 });
    let error = engine
        .eval_query(
            "data.budget.answer with input.value as 99".to_string(),
            false,
        )
        .expect_err("with evaluation must be budgeted");
    assert!(matches!(budget_error(&error), EvaluationBudgetError { .. }));

    engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 1_000 });
    let result = engine.eval_rule("data.budget.answer".to_string())?;
    assert_eq!(result, Value::from(7));
    assert!(engine.evaluation_metrics().consumed < 1_000);
    Ok(())
}
