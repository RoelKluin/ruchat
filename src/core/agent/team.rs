use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A named, persisted `Orchestrator` preset — `ruchat manager` lets a user
/// save/select these instead of retyping a full `--agentic` JSON blob each
/// time. `config` uses exactly the same shape `Orchestrator::new` and
/// `AskArgs::into_config` expect (`Architect`/`Worker`/`Validator`/`Critics`/...
/// keys); a `Team` is not a separate execution engine, it's just a saved
/// config plus a goal.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Team {
    pub name: String,
    pub goal: String,
    pub config: Value,
}

impl Team {
    pub fn new(name: String, goal: String, config: Value) -> Self {
        Self { name, goal, config }
    }
}
