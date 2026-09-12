// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::ToString as _;
use alloc::sync::Arc;
use alloc::{vec, vec::Vec};
use anyhow::Result;
use core::sync::atomic::{AtomicUsize, Ordering};

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
    assert_eq!(first_error.limit, limit);
    assert!(first_error.consumed > limit);
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

#[test]
fn range_expansion_is_rejected_before_dispatch() -> Result<()> {
    let mut engine = engine_with_policy(
        r#"
package budget

answer := count(numbers.range(0, 10000000))
"#,
    )?;
    engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 50 });

    let error = engine
        .eval_rule("data.budget.answer".to_string())
        .expect_err("range expansion must exceed the preflight budget");
    let error = budget_error(&error);

    assert_eq!(error.limit, 50);
    assert!(error.consumed > error.limit);
    Ok(())
}

#[cfg(feature = "net")]
#[test]
fn cidr_expansion_is_rejected_before_dispatch() -> Result<()> {
    let mut engine = engine_with_policy(
        r#"
package budget

answer := count(net.cidr_expand("10.0.0.0/8"))
"#,
    )?;
    engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 50 });

    let error = engine
        .eval_rule("data.budget.answer".to_string())
        .expect_err("CIDR expansion must exceed the preflight budget");
    let error = budget_error(&error);

    assert_eq!(error.limit, 50);
    assert!(error.consumed > error.limit);
    Ok(())
}

#[test]
fn huge_bit_shifts_are_rejected_before_dispatch() -> Result<()> {
    for operation in ["bits.lsh", "bits.rsh"] {
        let policy = format!(
            r#"
package budget

answer := {operation}(1, 2000000000)
"#
        );
        let mut engine = engine_with_policy(&policy)?;
        engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 1_000 });

        let error = engine
            .eval_rule("data.budget.answer".to_string())
            .expect_err("a huge shift must exceed the preflight budget");
        assert_eq!(budget_error(&error).limit, 1_000);
    }
    Ok(())
}

#[test]
fn bigint_arithmetic_and_decimal_pow_are_precharged() -> Result<()> {
    let mut multiply = engine_with_policy(
        r#"
package budget

answer := 1234567890123456789012345678901234567890 * 9876543210987654321098765432109876543210
"#,
    )?;
    multiply.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 100 });
    let first = multiply
        .eval_rule("data.budget.answer".to_string())
        .expect_err("BigInt multiplication must be precharged");
    let second = multiply
        .eval_rule("data.budget.answer".to_string())
        .expect_err("BigInt multiplication accounting must repeat");
    assert_eq!(budget_error(&first), budget_error(&second));

    for call in [
        r#"units.parse("1e2000000000")"#,
        r#"units.parse("1e2000000000K")"#,
        r#"to_number("1e2000000000")"#,
    ] {
        let policy = format!("package budget\nanswer := {call}\n");
        let mut engine = engine_with_policy(&policy)?;
        engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 1_000 });
        let error = engine
            .eval_rule("data.budget.answer".to_string())
            .expect_err("decimal exponentiation must be rejected before dispatch");
        assert_eq!(budget_error(&error).limit, 1_000);
    }
    Ok(())
}

#[test]
fn sprintf_width_and_precision_are_rejected_before_dispatch() -> Result<()> {
    for format in ["%200000000d", "%0200000000d", "%.200000000f"] {
        let policy = format!(
            r#"
package budget

answer := sprintf("{format}", [1])
"#
        );
        let mut engine = engine_with_policy(&policy)?;
        engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 1_000 });
        let error = engine
            .eval_rule("data.budget.answer".to_string())
            .expect_err("sprintf expansion must be rejected before dispatch");
        assert_eq!(budget_error(&error).limit, 1_000);
    }
    Ok(())
}

#[test]
fn format_int_bigint_output_is_precharged() -> Result<()> {
    let mut engine = engine_with_policy(
        r#"
package budget

answer := format_int(1234567890123456789012345678901234567890, input)
"#,
    )?;
    engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 100 });
    for base in [2, 8, 10, 16] {
        engine.set_input(Value::from(base));
        let error = engine
            .eval_rule("data.budget.answer".to_string())
            .expect_err("BigInt formatting must exceed the preflight budget");
        assert_eq!(budget_error(&error).limit, 100);
    }
    Ok(())
}

#[test]
fn larger_builtin_values_consume_more_structural_work() -> Result<()> {
    let mut engine = engine_with_policy(
        r#"
package budget

answer := count(input)
"#,
    )?;
    engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 100_000 });

    engine.set_input(Value::from("x"));
    engine.eval_rule("data.budget.answer".to_string())?;
    let short_string = engine.evaluation_metrics().consumed;

    engine.set_input(Value::from("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"));
    engine.eval_rule("data.budget.answer".to_string())?;
    let long_string = engine.evaluation_metrics().consumed;

    engine.set_input(Value::from_array(vec![Value::from(1)]));
    engine.eval_rule("data.budget.answer".to_string())?;
    let short_array = engine.evaluation_metrics().consumed;

    engine.set_input(Value::from_array(
        (0..16).map(Value::from).collect::<Vec<_>>(),
    ));
    engine.eval_rule("data.budget.answer".to_string())?;
    let long_array = engine.evaluation_metrics().consumed;

    assert!(long_string > short_string);
    assert!(long_array > short_array);
    Ok(())
}

#[test]
fn bigint_magnitude_increases_structural_work() -> Result<()> {
    let mut small_engine =
        engine_with_policy("package budget\nanswer := count([18446744073709551616])\n")?;
    small_engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 100_000 });
    small_engine.eval_rule("data.budget.answer".to_string())?;
    let small = small_engine.evaluation_metrics().consumed;

    let mut large_engine = engine_with_policy(
        "package budget\nanswer := count([340282366920938463463374607431768211456])\n",
    )?;
    large_engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 100_000 });
    large_engine.eval_rule("data.budget.answer".to_string())?;
    let large = large_engine.evaluation_metrics().consumed;

    assert!(large > small);
    Ok(())
}

#[test]
fn unsafe_numeric_deserializers_are_default_denied() -> Result<()> {
    for call in [
        r#"json.unmarshal("1e2000000000")"#,
        r#"json.is_valid("1e2000000000")"#,
    ] {
        let policy = format!("package budget\nanswer := {call}\n");
        let mut engine = engine_with_policy(&policy)?;
        engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 1_000 });
        let error = engine
            .eval_rule("data.budget.answer".to_string())
            .expect_err("an unbounded numeric deserializer must be denied");
        assert!(error
            .to_string()
            .contains("has no deterministic work estimator"));
    }
    Ok(())
}

#[test]
fn extensions_are_rejected_before_invocation_while_budgeting() -> Result<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let extension_calls = calls.clone();
    let mut engine = engine_with_policy(
        r#"
package budget

answer := custom_repeat("x")
"#,
    )?;
    engine.add_extension(
        "custom_repeat".to_string(),
        1,
        Box::new(move |args: Vec<Value>| {
            extension_calls.fetch_add(1, Ordering::SeqCst);
            Ok(args.first().cloned().unwrap_or(Value::Undefined))
        }),
    )?;

    assert_eq!(
        engine.eval_rule("data.budget.answer".to_string())?,
        Value::from("x")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    engine.set_evaluation_budget_config(EvaluationBudgetConfig { limit: 1_000 });
    let error = engine
        .eval_rule("data.budget.answer".to_string())
        .expect_err("an extension without an estimator must be rejected");

    assert!(error
        .to_string()
        .contains("extension `custom_repeat` has no deterministic work estimator"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    Ok(())
}
