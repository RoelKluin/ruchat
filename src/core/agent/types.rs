use crate::{Result, RuChatError};
use ollama_rs::generation::completion::GenerationResponse;
use serde_json::Value;
use tokio::sync::mpsc;

pub(crate) struct Context {
    pub(crate) goal: String,
    pub(crate) history: String,
    pub(crate) output: String,
    pub(crate) context: String,
    pub(crate) rejections: String,
    pub(crate) documents: String,
    pub(crate) config: Value,
}

impl Context {
    pub(crate) fn new(goal: String) -> Self {
        Self {
            goal,
            history: String::new(),
            output: String::new(),
            context: String::new(),
            rejections: String::new(),
            documents: String::new(),
            config: Value::Null,
        }
    }
    pub(crate) fn read_config_file(&mut self, path: &str) -> Result<()> {
        let config_str = std::fs::read_to_string(path)?;
        self.config = serde_json::from_str(&config_str)?;
        Ok(())
    }
    pub(crate) fn is_approved(&self) -> bool {
        self.rejections.is_empty()
    }
    pub(crate) async fn trace(
        &mut self,
        tx: &mpsc::Sender<Result<Vec<GenerationResponse>>>,
        err: String,
    ) {
        if !err.is_empty() {
            self.rejections.push_str(&format!("\n{err}"));
            tx.send(Err(RuChatError::Trace(err))).await.ok();
        }
        let trace_output = format!(
            "# Orchestration Trace\n\n## Goal\n{}\n\n## Context\n{}\n\n## History\n{}\n\n## Rejections\n{}",
            self.goal, self.context, self.history, self.rejections
        );
        let _ = tokio::fs::write(".ruchat_trace.md", trace_output).await;
    }
    pub(crate) fn build_collections_summary(&self) -> String {
        let mut summary = String::from("AVAILABLE COLLECTIONS (loaded from config):\n");

        if let Some(collections) = self.config.get("collections").and_then(|v| v.as_array()) {
            for coll in collections {
                let name = coll["name"].as_str().unwrap_or("unknown");
                let desc = coll["description"].as_str().unwrap_or("");
                let model = coll["embedding_model"].as_str().unwrap_or("unknown");
                let metadata: Vec<String> = coll["metadata_keys"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();

                let examples =
                    if let Some(exs) = coll.get("example_queries").and_then(|v| v.as_array()) {
                        exs.iter()
                            .map(|e| {
                                let q = e["query"].as_str().unwrap_or("");
                                let w = e.get("where").and_then(|v| v.as_str()).unwrap_or("none");
                                format!("    • query: \"{q}\"  where: \"{w}\"")
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        String::from("    (no examples provided)")
                    };

                summary.push_str(&format!(
                    "- **{name}**\n  Description: {desc}\n  Embedding model: {model}\n  Available metadata keys: {}\n  Collection-specific examples:\n{examples}\n\n",
                    metadata.join(", ")
                ));
            }
        } else {
            summary.push_str("(No collections defined in config — falling back to defaults)\n");
        }

        // Global settings
        if let Some(includes) = self
            .config
            .get("allowed_include_fields")
            .and_then(|v| v.as_array())
        {
            let inc_list: Vec<&str> = includes.iter().filter_map(|v| v.as_str()).collect();
            summary.push_str(&format!(
                "GLOBAL OPTIONS:\n- Allowed \"include\" fields (any combination): {}\n- Default n_results: {}\n",
                inc_list.join(", "),
                self.config.get("default_n_results").and_then(|v| v.as_u64()).unwrap_or(5)
            ));
        }

        summary
    }
    /// Apply debug imputations from a JSON file (only for the **first** agent in a debug sequence).
    /// Called exactly once per debug run.
    pub(crate) fn apply_debug_imputations(&mut self, imputations: &Value) {
        if let Some(d) = imputations.get("documents").and_then(|v| v.as_str()) {
            self.documents = d.to_string();
        }
        if let Some(c) = imputations.get("context").and_then(|v| v.as_str()) {
            self.context = c.to_string();
        }
        if let Some(h) = imputations.get("history").and_then(|v| v.as_str()) {
            self.history = h.to_string();
        }
    }
    pub(crate) async fn print_debug_info(
        &mut self,
        tx: &mpsc::Sender<Result<Vec<GenerationResponse>>>,
        role: &str,
    ) {
        let debug_info = format!(
            "DEBUG INFO FOR ROLE: {role}\n\nGOAL:\n{}\n\nCONTEXT:\n{}\n\nHISTORY:\n{}\n\nREJECTIONS:\n{}\n\nDOCUMENTS:\n{}",
            self.goal, self.context, self.history, self.rejections, self.documents
        );
        self.trace(tx, debug_info).await;
    }
}
