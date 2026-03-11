use serde::Deserialize;
use std::fmt;

#[derive(Debug, Clone)]
pub(crate) enum TaskType {
    RustRefactor,
    GitBisect,
    ShellAutomation,
    DebugCore,
}

impl fmt::Display for TaskType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            TaskType::RustRefactor => "Focus on memory safety and idiomatic Result usage.",
            TaskType::GitBisect => "Methodically narrow down the commit range using exit codes.",
            TaskType::ShellAutomation => "Write POSIX compliant scripts with verbose logging.",
            TaskType::DebugCore => "Check for race conditions and verify thread safety.",
            /*TaskType::RustRefactor2 => "Focus on ownership, lifetimes, and idiomatic patterns.",
            TaskType::GitBisect2 => "Analyze commit history to find regression points.",
            TaskType::ShellAutomation2 => "Write robust bash scripts with error handling (set -e).",
            TaskType::DebugCore2 => "Inspect stack traces and memory logs for bottlenecks.",*/
        };
        write!(f, "{s}")
    }
}

impl<'de> Deserialize<'de> for TaskType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.to_lowercase().as_str() {
            "rustrefactor" => TaskType::RustRefactor,
            "gitbisect" => TaskType::GitBisect,
            "shellautomation" => TaskType::ShellAutomation,
            "debugcore" => TaskType::DebugCore,
            _ => TaskType::ShellAutomation, // Default to ShellAutomation if unknown
        })
    }
}

