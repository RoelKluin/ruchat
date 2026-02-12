use std::process::Command;

/// Run cargo tests and return the output as a String.
fn run_cargo_test() -> String {
    let output = Command::new("cargo")
        .args(["test", "--", "--nocapture"])
        .output()
        .expect("failed to execute tests");

    // Parse the output to highlight failures in Red in the TUI
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// main function for RuChat
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ruchat::run().await.map_err(|e| {
        eprintln!("Application error: {}", e);
        e
    })?;
    Ok(())
}
