// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::ffi::OsString;

use anyhow::Result;
use clap::Args;

use crate::tasks::util::{run_cargo_step, workspace_root};

/// Builds the ensure_no_std harness for an embedded target.
#[derive(Args, Default)]
pub struct TestNoStdCommand {
    /// Target triple to compile (defaults to thumbv7m-none-eabi).
    #[arg(long, default_value = "thumbv7m-none-eabi")]
    pub target: String,

    /// Compile artefacts in release mode.
    #[arg(long)]
    pub release: bool,

    /// Propagate --frozen to the cargo invocations.
    #[arg(long)]
    pub frozen: bool,
}

impl TestNoStdCommand {
    pub fn run(&self) -> Result<()> {
        let workspace = workspace_root();
        let project_dir = workspace.join("tests/ensure_no_std");

        let mut interpreter_args = vec![
            OsString::from("check"),
            OsString::from("--package"),
            OsString::from("regorus"),
            OsString::from("--lib"),
            OsString::from("--no-default-features"),
            OsString::from("--features"),
            OsString::from("opa-no-std"),
            OsString::from("--target"),
            OsString::from(&self.target),
        ];
        let mut harness_args = vec![
            OsString::from("build"),
            OsString::from("--target"),
            OsString::from(&self.target),
        ];
        if self.release {
            interpreter_args.push(OsString::from("--release"));
            harness_args.push(OsString::from("--release"));
        }
        if self.frozen {
            interpreter_args.push(OsString::from("--frozen"));
            harness_args.push(OsString::from("--frozen"));
        }
        run_cargo_step(
            &workspace,
            "cargo check (interpreter opa-no-std)",
            interpreter_args,
        )?;
        run_cargo_step(
            &project_dir,
            "cargo build (tests/ensure_no_std)",
            harness_args,
        )?;
        Ok(())
    }
}
