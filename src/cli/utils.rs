use anyhow::Result;
use std::error::Error;

// Deliberately unused — this exact function is the planted "unused code" fixture
// `core/agent/evals.rs`'s `agent_eval_architect_does_not_repeat_a_choice_the_real_content_disproved`
// scenario tests the Architect against by name. Do NOT delete as part of a dead-code cleanup;
// that eval's premise depends on it existing and staying unused.
#[allow(dead_code)]
pub(super) fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<dyn Error + Send + Sync + 'static>>
where
    T: std::str::FromStr,
    T::Err: Error + Send + Sync + 'static,
    U: std::str::FromStr,
    U::Err: Error + Send + Sync + 'static,
{
    match s.split_once(':') {
        Some((key, value)) => Ok((key.parse()?, value.parse()?)),
        None => Err(format!("invalid KEY:VALUE, no `:` found in `{}`", s).into()),
    }
}
