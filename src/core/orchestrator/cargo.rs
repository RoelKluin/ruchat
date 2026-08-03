use crate::{Result, RuChatError};
use std::time::Duration;
use tokio::process::Command;

/// Memory ceiling applied to every cargo subprocess this crate spawns (`RLIMIT_AS`, virtual
/// address space) — a backstop against a runaway or hallucinated build (e.g. a dependency bump
/// that pulls in something pathological) consuming unbounded host memory. Generous enough for a
/// normal `cargo check`/`cargo test` on a project this size; bump it if a legitimate build ever
/// hits it.
#[cfg(unix)]
const MAX_MEMORY_BYTES: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB

/// Applies `RLIMIT_AS` (memory) and `RLIMIT_CPU` (CPU time, not wall-clock — see
/// `cpu_seconds_for`) to `cmd` before it's spawned. Both are inherited by every process cargo
/// itself forks (rustc, build scripts, proc-macros), since POSIX rlimits survive fork/exec — so
/// setting them once on the `cargo` invocation covers its whole subtree. A backstop behind the
/// existing `tokio::time::timeout` wall-clock caps already used at every call site, not a
/// replacement for them: a hung build still gets killed by the timeout even if it stays under
/// these limits. No-op on non-unix targets (`setrlimit` is POSIX-only; nothing in this repo
/// currently needs Windows support — see `TODO.md` if that changes).
#[cfg(unix)]
pub(crate) fn limit_resources(cmd: &mut Command, wall_clock_secs: u64) {
    let cpu_seconds = cpu_seconds_for(wall_clock_secs);
    // SAFETY: the closure only calls `setrlimit`, an async-signal-safe libc syscall wrapper
    // that neither allocates nor touches Rust-managed state — safe to run in the forked child
    // between fork() and exec(), which is the narrow window `pre_exec`'s safety contract requires.
    unsafe {
        cmd.pre_exec(move || {
            set_rlimit(libc::RLIMIT_AS, MAX_MEMORY_BYTES)?;
            set_rlimit(libc::RLIMIT_CPU, cpu_seconds)?;
            Ok(())
        });
    }
}

#[cfg(not(unix))]
pub(crate) fn limit_resources(_cmd: &mut Command, _wall_clock_secs: u64) {}

/// `RLIMIT_CPU` counts total CPU time consumed, not wall-clock time — a parallel build across
/// N cores can burn up to N seconds of CPU time per wall-clock second, so scaling by the host's
/// available parallelism (rather than using `wall_clock_secs` directly) avoids the limit firing
/// on a legitimate parallel build well before its wall-clock timeout would.
#[cfg(unix)]
fn cpu_seconds_for(wall_clock_secs: u64) -> u64 {
    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get() as u64)
        .unwrap_or(4);
    wall_clock_secs.saturating_mul(parallelism)
}

#[cfg(unix)]
fn set_rlimit(resource: libc::__rlimit_resource_t, limit: u64) -> std::io::Result<()> {
    let rl = libc::rlimit {
        rlim_cur: limit as libc::rlim_t,
        rlim_max: limit as libc::rlim_t,
    };
    if unsafe { libc::setrlimit(resource, &rl) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Read-only compile check, reusing the same 30s-timeout pattern as
/// `protocol::Validation::run_cargo_check`. Distinct call site: this is a
/// Worker-invoked, on-demand inspection (no rejection/turn semantics),
/// whereas `Validation::run_build_and_test` is the automatic `Stage::Test`
/// gate. Kept as two functions rather than merged to avoid coupling the
/// Tester's rejection flow to what the Worker sees mid-Implement.
pub(crate) async fn cargo_check() -> Result<String> {
    let mut cmd = Command::new("cargo");
    cmd.args(["check", "--message-format=short"]);
    limit_resources(&mut cmd, 30);
    let output = tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| RuChatError::InternalError("cargo check timed out after 30s".into()))?
        .map_err(|e| RuChatError::InternalError(format!("cargo check failed to run: {e}")))?;
    Ok(format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

/// `cargo tree --duplicates` — lists dependency versions duplicated in the
/// resolved graph. Fast, read-only, no timeout needed beyond a generous cap.
pub(crate) async fn cargo_dupes() -> Result<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        Command::new("cargo").args(["tree", "--duplicates"]).output(),
    )
    .await
    .map_err(|_| RuChatError::InternalError("cargo tree timed out after 20s".into()))?
    .map_err(|e| RuChatError::InternalError(format!("cargo tree failed to run: {e}")))?;
    if !output.status.success() {
        return Err(RuChatError::InternalError(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        Ok("No duplicate dependency versions found.".into())
    } else {
        Ok(stdout.into_owned())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn cpu_seconds_for_scales_with_available_parallelism() {
        let parallelism = std::thread::available_parallelism()
            .map(|n| n.get() as u64)
            .unwrap_or(4);
        assert_eq!(cpu_seconds_for(10), 10 * parallelism);
    }

    // End-to-end: spawns a real child process and reads back its own rlimits (via the shell
    // builtin `ulimit`, which reports what the kernel actually enforced) rather than just
    // checking that `pre_exec` was registered — the only way to be sure `setrlimit` inside the
    // fork/exec window actually took effect on the child.
    #[tokio::test]
    async fn limit_resources_applies_rlimit_as_and_cpu_to_child_process() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "ulimit -v; ulimit -t"]);
        limit_resources(&mut cmd, 10);
        let output = cmd.output().await.expect("failed to run sh");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut lines = stdout.lines();
        let mem_kb: u64 = lines
            .next()
            .expect("ulimit -v output")
            .trim()
            .parse()
            .expect("ulimit -v should print a number of KiB");
        let cpu_s: u64 = lines
            .next()
            .expect("ulimit -t output")
            .trim()
            .parse()
            .expect("ulimit -t should print a number of seconds");
        assert_eq!(mem_kb * 1024, MAX_MEMORY_BYTES);
        assert_eq!(cpu_s, cpu_seconds_for(10));
    }
}
