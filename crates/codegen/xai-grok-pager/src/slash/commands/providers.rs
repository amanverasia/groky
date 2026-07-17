//! `/providers` -- manage model providers and API keys (alias: `/login`).
//!
//! Opens the provider picker modal. The xAI row inside the modal
//! dispatches the unchanged OAuth login flow ([`Action::Login`]).

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand};

pub struct ProvidersCommand;

impl SlashCommand for ProvidersCommand {
    fn name(&self) -> &str {
        "providers"
    }

    fn aliases(&self) -> &[&str] {
        &["login"]
    }

    fn description(&self) -> &str {
        "Manage model providers, API keys, and login"
    }

    fn usage(&self) -> &str {
        "/providers"
    }

    fn run(&self, _ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
        CommandResult::Action(Action::OpenProviders)
    }
}

// NOTE: the pager's in-crate test harness does not currently compile
// (pre-existing breakage unrelated to this module); this test is in place
// for when it is repaired. The public-surface assertions live in
// `tests/providers_ui.rs`.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_run_dispatches_open_providers() {
        let models = crate::acp::model_state::ModelState::default();
        let mut ctx = crate::slash::commands::tests::make_ctx(&models);
        let result = ProvidersCommand.run(&mut ctx, "");
        assert!(matches!(
            result,
            CommandResult::Action(Action::OpenProviders)
        ));
    }
}
