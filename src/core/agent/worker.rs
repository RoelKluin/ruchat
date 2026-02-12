use crate::providers::llm::ollama::ask::generate_oneshot;
use anyhow::Result;
use ollama_rs::Ollama;
use ollama_rs::models::ModelOptions;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Define an Enum for processors to allow Serde to serialize the config.
// Function pointers cannot be serialized.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProcessorType {
    None,
    TrimWhitespace,
    JsonExtract,
    WrapMarkdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Agent {
    pub name: String,
    pub model: String,
    pub role: String, // System prompt component
    pub initial_instruction: String,

    #[serde(skip)]
    pub options: Option<ModelOptions>,

    pub intermediate_target: Option<String>,
    pub files: Vec<PathBuf>,

    pub preprocessor: ProcessorType,
    pub postprocessor: ProcessorType,
}

impl Agent {
    pub fn new(model: String, role: String) -> Self {
        Self {
            name: "worker".into(),
            model,
            role,
            initial_instruction: String::new(),
            options: None,
            intermediate_target: None,
            files: vec![],
            preprocessor: ProcessorType::None,
            postprocessor: ProcessorType::None,
        }
    }

    pub async fn process(&mut self, ollama: &Ollama, input: String) -> Result<String> {
        let processed_input = self.run_preprocessor(input);

        let file_context = self.load_files().await?;

        // Structure the prompt
        let full_prompt = format!(
            "SYSTEM: {}\n\nCONTEXT:\n{}\n\nTASK:\n{}",
            self.role, file_context, processed_input
        );

        let response =
            generate_oneshot(ollama, &self.model, &full_prompt, self.options.clone()).await?;

        Ok(self.run_postprocessor(response))
    }

    fn run_preprocessor(&self, input: String) -> String {
        match self.preprocessor {
            ProcessorType::TrimWhitespace => input.trim().to_string(),
            _ => input,
        }
    }

    fn run_postprocessor(&self, input: String) -> String {
        match self.postprocessor {
            ProcessorType::JsonExtract => {
                // Logic to extract JSON block
                input
            }
            _ => input,
        }
    }

    async fn load_files(&self) -> Result<String> {
        let mut buffer = String::new();
        for path in &self.files {
            if path.exists() {
                let content = tokio::fs::read_to_string(path).await?;
                buffer.push_str(&format!("\n--- File: {:?} ---\n{}\n", path, content));
            }
        }
        Ok(buffer)
    }
}
