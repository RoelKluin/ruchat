use crate::cli::config::ConfigArgs;
use anyhow::Result;
use chroma::client::{ChromaAuthMethod, ChromaHttpClientOptions, ChromaRetryOptions};
use chroma::ChromaHttpClient;
use clap::Parser;
use http::{HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Parser, Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct ChromaClientConfigArgs {
    /// URL of the ChromaDB server.
    #[arg(
        short = 'C',
        long,
        env = "CHROMA_SERVER",
        default_value = "http://localhost:8000",
        help_heading = "Chroma Connection"
    )]
    pub chroma_server: String,

    /// Optional authentication token for the ChromaDB instance.
    #[arg(
        short = 't',
        long,
        env = "CHROMA_TOKEN",
        help_heading = "Chroma Connection"
    )]
    pub chroma_token: Option<String>,

    /// Maximum number of times to retry a failed request.
    #[arg(
        long,
        default_value_t = 3,
        hide_short_help = true,
        hide_long_help = false,
        hide_default_value = true,
        help_heading = "Advanced Retry"
    )]
    pub max_retries: usize,

    /// Minimum delay (in milliseconds) between retries.
    #[arg(
        long,
        default_value_t = 10,
        hide_short_help = true,
        hide_long_help = false,
        hide_default_value = true,
        help_heading = "Advanced Retry"
    )]
    pub min_delay: u64,

    /// Maximum delay (in milliseconds) between retries.
    #[arg(
        long,
        default_value_t = 100,
        hide_default_value = true,
        hide_short_help = true,
        hide_long_help = false,
        help_heading = "Advanced Retry"
    )]
    pub max_delay: u64,

    /// Whether to apply a random jitter to the retry delay to prevent thundering herds.
    #[arg(long, default_value_t = true, action = clap::ArgAction::SetTrue, help_heading = "Advanced Retry", hide_short_help = true, hide_long_help = false)]
    pub jitter: bool,

    /// The tenant identifier used for multi-tenancy environments.
    #[arg(
        long,
        default_value = "default_tenant",
        help_heading = "Chroma Connection"
    )]
    pub tenant_id: Option<String>,

    /// The name of the database within the Chroma instance.
    #[arg(
        short = 'd',
        long,
        default_value = "default",
        help_heading = "Chroma Connection"
    )]
    pub chroma_database: Option<String>,

    #[command(flatten)]
    pub config: ConfigArgs,
}

impl ChromaClientConfigArgs {
    /// Access a running Chroma server to store and retrieve data for embeddings.
    ///
    /// This function creates a client for interacting with a Chroma server. It
    /// supports authentication using tokens and can connect to a specified server
    /// and database.
    ///
    /// # Returns
    ///
    /// A `Result` containing the `ChromaClient` or an error.
    pub(crate) async fn create_client(&self) -> Result<ChromaHttpClient> {
        let mut cfg = self.config.load().await?;
        self.config.merge_into(cfg.clone(), &mut cfg);

        // Apply config values (lowest priority)
        let mut args = self.clone();
        if let Some(v) = cfg.get("chroma") {
            args.update_from_json(v)?;
        }

        // Your original logic, unchanged
        if let Some(token) = args.chroma_token.as_ref() {
            let endpoint = args.chroma_server.parse()?;
            let value = HeaderValue::from_str(token.as_str())?;
            let header = HeaderName::from_static("x_chroma_token");
            let auth_method = ChromaAuthMethod::HeaderAuth { header, value };
            let retry_options = ChromaRetryOptions {
                max_retries: args.max_retries,
                min_delay: Duration::from_millis(args.min_delay),
                max_delay: Duration::from_secs(args.max_delay),
                jitter: args.jitter,
            };
            let client = ChromaHttpClientOptions {
                endpoint,
                auth_method,
                retry_options,
                tenant_id: args.tenant_id.clone(),
                database_name: args.chroma_database.clone(),
            };
            Ok(ChromaHttpClient::new(client))
        } else {
            Ok(ChromaHttpClient::new(Default::default()))
        }
    }

    pub(crate) fn update_from_json(&mut self, val: &Value) -> Result<()> {
        val.as_object()
            .ok_or_else(|| {
                anyhow::anyhow!("Expected a JSON object to update ChromaClientConfigArgs")
            })?
            .iter()
            .for_each(|(key, value)| match key.as_str() {
                "chroma_server" => {
                    self.chroma_server = value.as_str().unwrap_or(&self.chroma_server).to_string()
                }
                "chroma_token" => self.chroma_token = value.as_str().map(|s| s.to_string()),
                "max_retries" => {
                    self.max_retries = value.as_u64().unwrap_or(self.max_retries as u64) as usize
                }
                "min_delay" => self.min_delay = value.as_u64().unwrap_or(self.min_delay),
                "max_delay" => self.max_delay = value.as_u64().unwrap_or(self.max_delay),
                "jitter" => self.jitter = value.as_bool().unwrap_or(self.jitter),
                "tenant_id" => self.tenant_id = value.as_str().map(|s| s.to_string()),
                "chroma_database" => self.chroma_database = value.as_str().map(|s| s.to_string()),
                _ => eprintln!(
                    "Warning: Unrecognized field '{}' in ChromaClientConfigArgs JSON",
                    key
                ),
            });
        Ok(())
    }
}

impl Default for ChromaClientConfigArgs {
    fn default() -> Self {
        Self {
            chroma_server: "http://localhost:8000".to_string(),
            chroma_token: None,
            max_retries: 3,
            min_delay: 100,
            max_delay: 10,
            jitter: true,
            tenant_id: Some("default_tenant".to_string()),
            chroma_database: Some("default".to_string()),
            config: ConfigArgs::default(),
        }
    }
}
