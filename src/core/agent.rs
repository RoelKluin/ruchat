#[cfg(test)]
mod evals;
pub(crate) mod event;
pub(crate) mod json_extract;
pub(crate) mod llm_client;
pub(crate) mod manager;
pub(crate) mod pipeline;
pub(crate) mod protocol;
pub(crate) mod role;
pub(crate) mod team;
pub(crate) mod templates;
pub(crate) mod tokens;
pub(crate) mod tools;
pub(crate) mod types;

use crate::core::embed::{EmbedArgs, UpsertMode};
use crate::core::orchestrator::TaskType;
use crate::providers::llm::ollama::get_dynamic_history_limit;
use crate::providers::vector::chroma::query::Query;
use crate::{Result, RuChatError, options::get_options};
use event::{AgentEvent, StreamItem, send_event};
use llm_client::{LlmClient, VectorStore};
use ollama_rs::generation::chat::ChatMessage;
use ollama_rs::models::ModelOptions;
use protocol::Validation;
use role::Role;
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
pub(crate) use team::Team;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use types::{Context, TurnKind};

/// Trailing words considered by `has_runaway_repetition`, bounding its cost regardless of how
/// long the accumulated output has grown — a loop worth killing is, by definition, still
/// repeating right up to the newest tokens, so only the tail needs checking.
const REPETITION_CHECK_TAIL_WORDS: usize = 300;
/// A repeated unit shorter than this is too easy to hit by coincidence (e.g. "pub" appearing
/// twice in a struct definition) to treat as a genuine decoding loop.
const MIN_REPEAT_UNIT_WORDS: usize = 6;
/// How many times in a row the same unit must repeat before it's a runaway loop rather than a
/// model naturally restating something once.
const MIN_REPEAT_COUNT: usize = 3;

/// True when the tail of `text` is the same word-sequence repeating immediately, back-to-back,
/// at least `min_repeats` times, with the repeated unit at least `min_unit_words` words long —
/// a real-run failure mode where a local model gets stuck regenerating the same phrase or
/// sentence instead of producing new content, streaming tokens (all green, no pause between
/// turns — indistinguishable from real progress on the console) until it hits its generation
/// limit. `query_stream` uses this to cut a live generation short instead of waiting it out.
fn has_runaway_repetition(text: &str, min_unit_words: usize, min_repeats: usize) -> bool {
    let mut words: Vec<&str> = text
        .split_whitespace()
        .rev()
        .take(REPETITION_CHECK_TAIL_WORDS)
        .collect();
    words.reverse();

    if words.len() < min_unit_words * min_repeats {
        return false;
    }
    let max_unit = words.len() / min_repeats;
    for unit_len in min_unit_words..=max_unit {
        let mut repeats = 1;
        let mut end = words.len();
        while end >= unit_len * 2
            && words[end - unit_len..end] == words[end - 2 * unit_len..end - unit_len]
        {
            repeats += 1;
            end -= unit_len;
        }
        if repeats >= min_repeats {
            return true;
        }
    }
    false
}

pub(crate) struct Agent {
    options: ModelOptions,
    agent_config: HashMap<String, Value>,
    pub(super) embed_args: Option<EmbedArgs>,
    cfg: Value,
}

impl Agent {
    pub(crate) async fn new(
        config: &mut Value,
        role: &str,
        required: bool,
        task_type: Option<&TaskType>,
        cfg: Value,
    ) -> Result<Self> {
        if let Some(agent_val) = config.get(role) {
            // Check if it's a raw JSON string (from CLI) or an Object (from json! macro)
            let options_str = match agent_val.as_str() {
                Some(s) => s.to_string(),
                None => agent_val.to_string(),
            };
            let (options, mut agent_config) = get_options(&options_str).await?;
            agent_config.insert("role".to_string(), Value::String(role.to_string()));
            if let Some(task) = task_type {
                agent_config.insert(
                    "task_hint".to_string(),
                    serde_json::Value::String(task.to_string()),
                );
            }

            let embed_args = agent_config
                .remove("embed_args")
                .and_then(|v| serde_json::from_value(v).ok());

            Ok(Self {
                options,
                agent_config,
                embed_args,
                cfg,
            })
        } else if required {
            Err(RuChatError::MissingAgent(role.to_string()))
        } else {
            Err(RuChatError::Is("Optional agent missing".into()))
        }
    }
    pub(crate) fn remove_str(&mut self, key: &str) -> Result<String> {
        let v = self
            .agent_config
            .remove(key)
            .ok_or(RuChatError::Is(format!(
                "No {key} to remove in agent config"
            )))?;
        if let Some(s) = v.as_str() {
            Ok(s.to_string())
        } else if v.is_object() {
            Ok(v.to_string())
        } else {
            Err(RuChatError::Is(format!(
                "Value for {key} is not a string in agent config {:?})",
                self.agent_config
            )))
        }
    }

    pub(crate) fn get_str(&self, key: &str) -> Result<&str> {
        self.agent_config
            .get(key)
            .ok_or(RuChatError::Is(format!("No {key} in agent config")))?
            .as_str()
            .ok_or(RuChatError::Is(format!(
                "Value for {key} is not a string in agent config {:?})",
                self.agent_config
            )))
    }
    pub(crate) async fn retrieve_and_generate(
        &self,
        client: &dyn VectorStore,
        ollama: &dyn LlmClient,
        q: Query,
    ) -> Result<String> {
        let model = self.get_str("embed_model").unwrap_or("all-minilm:l6-v2");
        q.query(client, ollama, model).await
    }
    pub(crate) fn get_dynamic_history_limit(&self) -> u64 {
        get_dynamic_history_limit(self.get_str("model").unwrap_or(""))
    }

    pub(super) async fn embed(
        &self,
        prompt: &str,
        mode: UpsertMode,
        ctx: &mut Context,
        msg: &str,
    ) -> Result<()> {
        if let Some(args) = self.embed_args.as_ref() {
            args.embed(prompt, mode, &self.cfg).await
        } else {
            EmbedArgs::default().embed(prompt, mode, &self.cfg).await
        }
        .map(|()| {
            let msg = format!("\n### SYSTEM: {msg}");
            ctx.push_turn(TurnKind::System, "MEMORIZE", msg);
        })
    }

    pub(crate) async fn query_stream(
        &mut self,
        ollama: &dyn LlmClient,
        ctx: &mut Context,
        tx: &mpsc::Sender<Result<StreamItem>>,
    ) -> Result<()> {
        let role = self.get_str("role")?.to_lowercase();
        let role = Role::from_str(role.as_str())?;

        let model = self.get_str("model")?;

        // System instructions and retrieved/untrusted content now ride as
        // distinct chat messages instead of one concatenated string — see
        // prior review note on prompt-injection surface via `documents_view`.
        let (system_text, user_text) = role.build_chat_messages(
            self.get_str("task").ok(),
            ctx,
            self.get_str("task_hint").ok(),
            self.get_str("approval_signal").ok(),
        )?;
        // Still refreshes this run's live file under `ruchat_traces/` with this turn's full
        // context/history before every query (useful when inspecting a stuck run), but doesn't
        // announce it on the visible stream — each role already gets its own colored banner
        // (`role.get_color()`, sent below), so a "[Role's input] querying 'model'..." line on
        // every single turn (every role, every round) added noise without telling the user
        // anything the banner didn't already. The model each role uses is now summarized once,
        // at the very start of the run — see `Orchestrator::run_stage_machine`.
        ctx.trace(tx, String::new()).await;
        let messages = vec![
            ChatMessage::system(system_text),
            ChatMessage::user(user_text),
        ];

        if let Ok(msg) = self.get_str("status_msg") {
            send_event(tx, AgentEvent::StatusUpdate(msg.to_string())).await?;
        }
        // Inject the color change into the stream
        send_event(tx, AgentEvent::ColorChange(role.get_color())).await?;

        let mut stream = ollama.chat_stream(model, messages).await?;

        ctx.output.clear();
        while let Some(res) = stream.next().await {
            let chunk = res?; // already RuChatError via ChatStream's Item type
            ctx.output.push_str(&chunk.message.content);
            tx.send(Ok(StreamItem::ChatChunk(chunk)))
                .await
                .map_err(|e| RuChatError::Is(e.to_string()))?;
            // Dropping `stream` below (out of scope at the end of this function) closes the
            // underlying request instead of waiting for the model to finish on its own — this
            // is what actually stops a real run: nothing else in the stage machine watches a
            // generation while it's in flight.
            if has_runaway_repetition(&ctx.output, MIN_REPEAT_UNIT_WORDS, MIN_REPEAT_COUNT) {
                let msg = format!(
                    "{role} generation stopped early: detected the same {MIN_REPEAT_UNIT_WORDS}+ \
                    word phrase repeating {MIN_REPEAT_COUNT}+ times in a row instead of new \
                    content."
                );
                ctx.push_turn(TurnKind::System, "Orchestrator", msg.clone());
                ctx.trace(tx, msg).await;
                break;
            }
        }
        send_event(tx, AgentEvent::ColorChange(Role::no_color())).await?;
        Ok(())
    }
    pub(super) async fn execute_and_verify(&self, ctx: &mut Context) -> Result<Validation> {
        let call = match tools::parse_tool_call(&ctx.output) {
            Ok(call) => call,
            Err(_) => return Ok(Validation::Skip),
        };

        match call.tool {
            tools::ToolName::ApplyPatch => {
                let diff = call.args["diff"].as_str().unwrap_or_default();
                Validation::apply_patch(diff, ctx).await
            }
            tools::ToolName::Memorize => self
                .embed(
                    call.args["content"].as_str().unwrap_or_default(),
                    UpsertMode::Upsert,
                    ctx,
                    "Information successfully memorized.",
                )
                .await
                .map_or_else(
                    |e| Ok(Validation::Failure(e.to_string())),
                    |_| Ok(Validation::Success),
                ),
            // Retrieve/GitLog/GitBlame/GitDiff/CargoCheck/CargoClippy/etc. are dispatched
            // earlier in Stage::Implement, not here — reaching one of them at verify-time means
            // the Worker called a read-only tool a second time in the same round (its one
            // information-lookup reask was already spent) instead of acting on what it already
            // has. Give it a specific, actionable correction rather than a generic "unexpected
            // tool" dump — this is a common failure mode for local models that keep
            // "researching" instead of switching to making the change.
            other => Ok(Validation::Failure(format!(
                "refused: you called '{other:?}' again instead of applying a change. You \
                already used this round's one information-lookup, and its result is already in \
                your context above — re-running the same (or any other) read-only tool won't \
                add anything new. You must now emit exactly one apply_patch (or memorize) \
                tool_call to actually make the change."
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use llm_client::ChatStream;
    use ollama_rs::generation::chat::ChatMessageResponse;
    use ollama_rs::generation::embeddings::GenerateEmbeddingsResponse;
    use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
    use serde_json::json;

    #[test]
    fn has_runaway_repetition_catches_a_phrase_looping_three_times() {
        let text = "intro words here as identified in the clippy output \
            as identified in the clippy output as identified in the clippy output";
        assert!(has_runaway_repetition(text, 6, 3));
    }

    #[test]
    fn has_runaway_repetition_ignores_normal_prose() {
        let text = "This plan removes the unused options field from the Agent struct in \
            src/core/agent.rs and updates every reference to it.";
        assert!(!has_runaway_repetition(text, 6, 3));
    }

    #[test]
    fn has_runaway_repetition_ignores_a_short_natural_repeat() {
        // "pub" repeating twice in a struct definition — below the min unit length, must not
        // trigger even though it's a literal repeat.
        let text = "pub id string pub name string pub id string pub name string pub id string";
        assert!(!has_runaway_repetition(text, 6, 3));
    }

    #[test]
    fn has_runaway_repetition_requires_the_full_repeat_count() {
        let twice = "alpha bravo charlie delta echo foxtrot alpha bravo charlie delta echo foxtrot";
        assert!(!has_runaway_repetition(twice, 6, 3));
        let thrice = format!("{twice} alpha bravo charlie delta echo foxtrot");
        assert!(has_runaway_repetition(&thrice, 6, 3));
    }

    /// Streams one word per chunk, exactly like a real Ollama response arriving token by token —
    /// needed to prove `query_stream` actually stops consuming the stream early, not just that
    /// the predicate it calls is correct in isolation.
    struct WordChunkClient {
        words: Vec<&'static str>,
    }

    #[async_trait]
    impl LlmClient for WordChunkClient {
        async fn chat_stream(
            &self,
            _model: &str,
            _messages: Vec<ChatMessage>,
        ) -> Result<ChatStream> {
            let responses: Vec<Result<ChatMessageResponse>> = self
                .words
                .iter()
                .map(|w| {
                    Ok(ChatMessageResponse {
                        model: "fake".to_string(),
                        created_at: String::new(),
                        message: ChatMessage::assistant(format!("{w} ")),
                        logprobs: None,
                        done: false,
                        final_data: None,
                    })
                })
                .collect();
            Ok(Box::pin(tokio_stream::iter(responses)))
        }

        async fn generate_embeddings(
            &self,
            _req: GenerateEmbeddingsRequest,
        ) -> Result<GenerateEmbeddingsResponse> {
            unreachable!("not exercised by this test")
        }
    }

    #[tokio::test]
    async fn query_stream_stops_early_on_a_runaway_repetition_loop() {
        let mut config = json!({"Worker": {"model": "fake"}});
        let mut agent = Agent::new(&mut config, "Worker", true, None, json!({}))
            .await
            .unwrap();
        let mut ctx = Context::new("goal".to_string());
        let (tx, mut rx) = mpsc::channel(200);
        // Drain the channel concurrently so `query_stream`'s bounded `tx.send` never blocks.
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let client = WordChunkClient {
            words: vec![
                "alpha",
                "bravo",
                "charlie",
                "delta",
                "echo",
                "foxtrot",
                "alpha",
                "bravo",
                "charlie",
                "delta",
                "echo",
                "foxtrot",
                "alpha",
                "bravo",
                "charlie",
                "delta",
                "echo",
                "foxtrot",
                "CANARY_SHOULD_NOT_BE_REACHED",
            ],
        };

        agent.query_stream(&client, &mut ctx, &tx).await.unwrap();

        assert!(
            !ctx.output.contains("CANARY_SHOULD_NOT_BE_REACHED"),
            "the stream should have stopped before this chunk arrived, got: {}",
            ctx.output
        );
        assert!(
            ctx.turns
                .iter()
                .any(|t| t.kind == TurnKind::System && t.content.contains("stopped early")),
            "expected a System turn recording why generation was cut short"
        );
    }
}
