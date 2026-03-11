pub(crate) enum Tool {
    Memorize { content: String },
    Shell { command: String },
}

pub(crate) struct ToolCall {
    pub(crate) name: String,
    pub(crate) content: String,
}
impl ToolCall {
    pub(crate) fn parse(output: &str) -> Option<Self> {
        // Simple string parsing to detect TOOL CALLS in the format: ### TOOL CALL: TOOL_NAME\nCONTENT\n### END TOOL CALL
        let re = regex::Regex::new(r"### TOOL CALL: (\w+)\n(.*?)\n### END TOOL CALL").ok()?;
        re.captures(output).and_then(|caps| Some(Self {
            name: caps.get(1)?.as_str().to_string(),
            content: caps.get(2)?.as_str().to_string(),
        }))
    }
    pub(crate) fn to_tool(&self) -> Option<Tool> {
        match self.name.as_str() {
            "MEMORIZE" => Some(Tool::Memorize { content: self.content.clone() }),
            "SHELL" => Some(Tool::Shell { command: self.content.clone() }),
            _ => None,
        }
    }
}

