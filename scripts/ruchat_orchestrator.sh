#!/bin/bash
# Principal Software Engineer | Linux Automation Expert
# Dynamic AI Agent Orchestrator for Ruchat/Ollama

# example:
# ./scripts/ruchat_orchestrator.sh meta-gen "Generate a commandline to improve scripts/ruchat_orchestrator.sh"

# Summary of Session JSON structure:
#{
#  "history": ["./orchestrator.sh rust-analysis --file main.rs"],
#  "last_task": "rust-analysis",
#  "total_tokens": 14520,
#  "total_cost": 0.029,
#  "debate_logs": [
#    {
#      "file": "main.rs",
#      "safety_issue": "Potential use-after-free in FFI block",
#      "perf_issue": "APPROVED",
#      "timestamp": "14:20:05"
#    }
#  ]
#}

set -euo pipefail

# --- Configuration & Defaults ---
ITERATIONS=4
TEMP_WORKER=0.7
TEMP_STRICT=0.0
RUCHAT_BIN="target/release/ruchat"
HISTORY_FILE="/tmp/ruchat_full_history.log"
STRIP_CHATTER=true
DRY_RUN=false
STRICT_FORMAT="Requirement: Output raw data or code blocks only. No polite fillers. No conversational intros/outros. Use strictly technical language."
# --- Colors ---
C_ARCH='\033[1;32m' C_WORK='\033[1;34m' C_VALI='\033[1;33m'
C_CRIT='\033[1;31m' C_SUMM='\033[1;35m' NC='\033[0m'
C_PERF='\033[1;94m'
usage() {
    cat <<EOF
Usage: $0 <task> [options] [goal]

Core Meta-Tasks:
  meta-gen             Interpret natural language to generate orchestrator commands.
  prompt-opt           Optimize inter-agent prompts for token efficiency/clarity.

Development & Refactoring:
  rust-refactor        General code improvement and structural changes.
  rust-algo-optimize   Focus on Big-O, algorithm logic, and performance.
  rust-fix-loop        Iterative 'cargo check' loop to resolve compilation errors.
  rust-high-stakes     Mission-critical mode with Safety vs. Perf duo-critic debate.
  editor-nav           Generate Vim/Ex scripts for automated file editing.
  stream-edit-suite    Apply complex, multi-file transformations.
  editor-nav2          Ctags-driven Vim automation for bulk refactoring.
  bash-refactor        Improve script efficiency, error handling, and POSIX compliance.

Analysis & Optimization:
  rust-analysis        High-level code review and symbol mapping.
  rust-deep-analysis   Macro expansion (cargo-expand) and binary size (cargo-bloat).
  rust-meta-opt        Refine generics and macros to reduce monomorphization bloat.
  profiling-analysis   Analyze execution bottlenecks and flamegraphs.
  rust-clippy          Lint-driven code cleanup and idiomatic fixes.
  rust-explain         Detailed explanation of complex logic or traits.
  rust-analysis2       Focused ownership and unsafe code auditing.

Dependency Management:
  rust-crate-expert    Deep integration with a specific crate ecosystem.
  rust-deps            Suggest and add relevant Cargo dependencies.
  rust-deps2           Comprehensive crate search and dependency management.

Testing & QA:
  rust-test            Standard test suite execution and failure analysis.
  debug-test           Targeted fixing of specific failing test cases.
  debug-core           Post-mortem analysis of GDB/LLDB backtraces and core dumps.
  test-sanitizer       Validate LLM output cleaning and regex robustness.
  chaos-drill          Inject environmental hazards (OOM, Latency) to test resilience.
  debug-core2          Refined core dump analysis with targeted Vim edits.

Git & Lifecycle:
  git-commit-gen       Generate clean, descriptive commits from staged changes.
  git-feature-flow     End-to-end feature branch management.
  git-pr-lifecycle     Review and apply PR diffs with validation.
  git-conflict-solver  Mediate and resolve complex merge conflicts.
  git-bisect-autofix   Automated binary search to find and fix regression culprits.
  git-history-audit    Review commit history for security or logic regressions.
  git-ops              General repository maintenance and plumbing.
  git-feature-flow2    Streamlined feature branch creation and pushing.

Documentation:
  docs                 Generate standard project documentation.
  doc-custom           Context-aware README or Rustdoc generation for specific traits.
  doc-gen              Generate doc comments that compile successfully.  
  doc-custom2         Enhanced context-aware documentation generation.

Options:
  --subject <text>     User goal for Artificial Architect.
  --file <path>        Target source file for analysis or editing.
  --commit <hash>      Specific Git revision to analyze or bisect.
  --crate <name>       Target specific crate in a workspace.
  --doc-type <type>    Format of documentation (md, html, etc.).
  --explain <text>     The topic to explain in documentation tasks.
  -m <role:model>      Override default model for a specific agent role.
  -i <iterations>      Maximum number of loops (default 5).
  -t <temp>            LLM temperature (0.0 for logic, 0.7 for chaos).
  --dry-run            Estimate token cost and plan without executing.
  --save <name>        Persist session state to ~/.ruchat/sessions/<name>.
  --resume <name>      Reload state and history from a previous session.
  --keep-chatter       Disable 'strip_chatter' (useful for debugging agent prose).
  --debug              Enable verbose logging of pipe communication.

Goal:
  A natural language description of the user's objective.
  either specified as the last argument or via --subject.
EOF
  [[ -n "${1:-}" ]] && echo -e "${C_CRIT}ERROR: $1${NC}"
  [[ -n "${2:-}" ]] && exit "$2"
}

# Define default role-to-model mapping
declare -A MODELS=(
    [architect]="qwen2.5:7b"
    [worker]="deepseek-coder-v2"
    [validator]="qwen2.5:7b"
    [critic]="qwen2.5:7b"
    [summarizer]="mistral-nemo"
    [critic_perf]="qwen2.5:7b"
    [chaos]="mistral"
)
# New Global State
TARGET_FILE=""
TARGET_COMMIT=""
TARGET_CRATE=""
DOC_TYPE="README.md"
EXPLAIN_SUBJECT=""
TASK_TYPE=""
USER_GOAL=""
SAVE_MODE=false
RESUME_MODE=false
SESSION_NAME="auto_save_$(date +%s)}"
DEBUG=false
ONBOARD=""

# Function to parse model argument
parse_model_arg() {
    local arg="$1"
    if [[ "$arg" == *":"* ]]; then
        local role="${arg%%:*}"
        local model="${arg#*:}"
        MODELS[$role]="$model"
    else
        MODELS[worker]="$arg"
    fi
}

# --- Argument Parsing ---
[ $# -lt 1 ] && usage "No task specified." 1
TASK_TYPE="$1"
shift

# Main Argument Loop Update
while [[ $# -gt 0 ]]; do
    case "$1" in
        -m|--model) parse_model_arg "$2"; shift 2 ;;
        --file) TARGET_FILE="$2"; shift 2 ;;
        --commit) TARGET_COMMIT="$2"; shift 2 ;;
        --crate) TARGET_CRATE="$2"; shift 2 ;;
        --doc-type) DOC_TYPE="$2"; shift 2 ;;
        --explain) EXPLAIN_SUBJECT="$2"; shift 2 ;;
        --keep-chatter) STRIP_CHATTER=false; shift ;;
        --dry-run) DRY_RUN=true; shift ;;
        -i|--iter) ITERATIONS="$2"; shift 2 ;;
        -t|--temp) TEMP_WORKER="$2"; shift 2 ;;
        --save) SAVE_MODE=true; SESSION_NAME="$2"; shift 2 ;;
        --resume) RESUME_MODE=true; SESSION_NAME="$2"; shift 2 ;;
        --debug) DEBUG=true; set -x; shift ;;
        --subject) USER_GOAL="${2%.}."; echo "User Goal: $USER_GOAL"; shift 2 ;;
        *) [ -z "${USER_GOAL}" ] && USER_GOAL="${1%.}." || 
        TASK_TYPE="$1"; shift ;;
    esac
done

# Cleanup function to close pipes and remove FIFOs
KEEP_ALIVE="${KEEP_ALIVE:-}"
AGENTS=("architect" "worker" "validator" "critic" "summarizer" "critic_perf" "chaos")
cleanup() {
    if [[ -n "$KEEP_ALIVE" ]]; then
        return
    fi
    for a in "${AGENTS[@]}"; do
        # 1. Use indirect expansion to get the FD value safely
        local varname="FD_${a^^}_IN"
        local fd="${!varname:-}"

        # 2. Check if the FD is set and refers to an open file descriptor
        if [[ -n "$fd" ]]; then
            # We use >&"$fd" to specify the exact file descriptor
            # and move the 2>/dev/null to the end of the command
            printf "\n---\n" >&"$fd" || true
        fi

        # 3. Remove the named pipes
        rm -f "/tmp/ruchat_${a}_in" "/tmp/ruchat_${a}_out" 2>/dev/null || true
    done
}
# --- Dynamic Role Engine ---
CHAOS_MODE=false
DUO_MODE=false

case "$TASK_TYPE" in
    prompt-opt)
        ARCHITECT_INIT="Meta-Prompt Engineer. Your job is to transform user goals into highly optimized prompts for a Worker agent. Use Delimited instructions, Few-shot examples, and Chain-of-Thought triggers."
        WORKER_INIT="Executor. Follow the optimized prompt exactly."
        CRITIC_INIT="Prompt Auditor. Ensure the prompt is clear, concise, and unambiguous."
        LOOP_TYPE="STANDARD" ;;
    qa)
        ARCHITECT_INIT="QA Architect. Define edge cases and unit test requirements."
        WORKER_INIT="Test Engineer. Write robust tests and cargo audit patches."
        CRITIC_INIT="Security Auditor. Identify gaps in test coverage."
        LOOP_TYPE="VALIDATED"; VAL_CMD="cargo test --no-run" ;;
    shell)
        ARCHITECT_INIT="Systems Architect. Design POSIX-compliant script logic."
        WORKER_INIT="Bash Expert. Write efficient scripts with error handling."
        CRITIC_INIT="Linux Hardening Expert. Review for quoting and injection flaws."
        LOOP_TYPE="VALIDATED"; VAL_CMD="shellcheck" ;;
    # --- Rust Specialized ---
    rust-analysis)
        ARCHITECT_INIT="Senior Rust Architect. Focus: Ownership, Borrowing, and Unsafe Auditing."
        WORKER_INIT="Rust Expert. Write safe, idiomatic code. Use 'cargo check' compatible syntax."
        CRITIC_INIT="Pedantic Rust Reviewer. Hunt for memory leaks and race conditions."
        LOOP_TYPE="VALIDATED"; VAL_CMD="cargo check" ;;
    rust-analysis2)
        ARCHITECT_INIT="Rust Safety Engineer. Identify ownership, borrowing, and unsafe code issues."
        WORKER_INIT="Code Auditor. Suggest fixes for memory safety and concurrency."
        CRITIC_INIT="Pedantic Reviewer. Ensure idiomatic Rust and no unsafe blocks remain."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;;
    rust-explain)
        ARCHITECT_INIT="Rust Educator. Breakdown ownership, lifetimes, and trait bounds."
        WORKER_INIT="Technical Writer. Explain code using analogies and memory diagrams."
        CRITIC_INIT="Clarity Editor. Ensure explanations are accurate and accessible."
        LOOP_TYPE="STANDARD" ;;
    rust-clippy)
        ARCHITECT_INIT="Rust Lint Specialist. Interpret Clippy output for performance/idiomatic improvements."
        WORKER_INIT="Refactoring Expert. Apply suggested fixes to code blocks."
        CRITIC_INIT="Code Reviewer. Ensure changes align with best practices."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;;
    rust-test)
        ARCHITECT_INIT="SDET. Identify edge cases, panics, and boundary conditions in Rust modules."
        WORKER_INIT="Test Engineer. Write #[cfg(test)] modules and integration tests."
        CRITIC_INIT="QA Lead. Ensure tests cover all critical paths and validate assertions."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;;

    # --- Git & History ---
    git-ops)
        ARCHITECT_INIT="Git Workflow Specialist. Focus: Atomic commits and history legibility."
        WORKER_INIT="Automation Expert. Generate git commands/scripts for complex rebasing."
        CRITIC_INIT="QA Lead. Ensure operations are non-destructive and semantic."
        LOOP_TYPE="STANDARD" ;;
    git-history-audit)
        ARCHITECT_INIT="Repo Historian. Analyze 'git log --graph' and 'git blame' to find technical debt origins."
        WORKER_INIT="Analyst. Summarize development direction and identify 'hot' files with high churn."
        CRITIC_INIT="Senior Reviewer. Spot security regressions or logic flaws in commit history."
        LOOP_TYPE="STANDARD" ;;
    git-commit-gen)
        ARCHITECT_INIT="Semantic Versioning Expert. Group changes into atomic, logical units."
        WORKER_INIT="Git Expert. Write Conventional Commit messages (feat/fix/chore)."
        CRITIC_INIT="Senior Reviewer. Ensure commit messages are clear and follow guidelines."
        LOOP_TYPE="STANDARD" ;;
    git-conflict-solver)
        ARCHITECT_INIT="Conflict Mediator. Analyze HEAD vs Incoming changes in Rust/Bash files."
        WORKER_INIT="Git Surgeon. Resolve conflicts manually while preserving logic from both sides."
        CRITIC_INIT="Logic Evaluator. Ensure merged code compiles and passes existing tests."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;;

    # --- Stream Editing & Automation ---
    stream-edit-suite)
        ARCHITECT_INIT="Automation Architect. Plan multi-stage transformations using Sed, Awk, and Perl."
        WORKER_INIT="RegEx Wizard. Provide optimized one-liners for bulk code changes."
        CRITIC_INIT="Safety Engineer. Verify patterns won't cause accidental data loss."
        LOOP_TYPE="VALIDATED"; VAL_CMD="shellcheck" ;;
    
    # --- Performance ---
    profiling-analysis)
        ARCHITECT_INIT="Performance Engineer. Interpret Flamegraphs, 'perf' output, and 'cargo-expand'."
        WORKER_INIT="Optimization Expert. Identify bottlenecks and suggest 'inline' or 'unroll' strategies."
        CRITIC_INIT="Senior Reviewer. Ensure optimizations don't compromise safety or readability."
        LOOP_TYPE="STANDARD" ;;

    # --- Documentation ---
    docs)
        ARCHITECT_INIT="Technical Writer. Plan documentation structure (README/Rustdoc)."
        WORKER_INIT="Markdown Specialist. Write clear, technical explanations."
        CRITIC_INIT="Editor. Check for clarity and missing technical details."
        LOOP_TYPE="STANDARD" ;;
    doc-gen)
        ARCHITECT_INIT="Technical Documentarian. Plan README.md, CHANGELOG.md, and Rustdoc architecture."
        WORKER_INIT="Writer. Generate doc comments (///) and examples that actually compile."
        CRITIC_INIT="Documentation Reviewer. Ensure examples are accurate and compile without errors."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;; # Ensures doc examples compile

    # --- Advanced Rust Dev ---
    rust-deps)
        ARCHITECT_INIT="Cargo Specialist. Imagine relevant crates for the goal: $USER_GOAL. Suggest features to enable."
        WORKER_INIT="Dependency Manager. Edit Cargo.toml. Maintain MSRV and version compatibility."
        CRITIC_INIT="Dependency Auditor. Check for crate bloat or security advisories."
        LOOP_TYPE="STANDARD" ;;
    rust-deps2)
        ARCHITECT_INIT="Crate Scout. Search for crates relevant to: $USER_GOAL."
        WORKER_INIT="Cargo Specialist. Update Cargo.toml dependencies and features."
        CRITIC_INIT="Dependency Auditor. Check for crate bloat or security advisories."
        LOOP_TYPE="VALIDATED"; VAL_CMD="cargo check" ;;
    rust-refactor)
        ARCHITECT_INIT="Algorithm Expert. Propose more efficient Big-O complexity or cache-friendly patterns."
        WORKER_INIT="Code Simplifier. Refactor logic to reduce cognitive load and remove redundant clones."
        CRITIC_INIT="Logic Evaluator. Identify non-compiler errors: off-by-one, logic inversions, or re-entrancy bugs."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;;
    rust-crate-expert)
        ARCHITECT_INIT="Specialist in the '$TARGET_CRATE' ecosystem. Know the trait patterns and common pitfalls."
        WORKER_INIT="API Integrator. Write idiomatic code using $TARGET_CRATE."
        CRITIC_INIT="Senior Reviewer. Ensure proper usage of $TARGET_CRATE and adherence to best practices."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;;

    rust-fix-loop)
        ARCHITECT_INIT="Senior Troubleshooter. Review the last compilation failure from session context."
        WORKER_INIT="Fix Agent. Apply changes to $TARGET_FILE to resolve the specific error."
        CRITIC_INIT="Logic Evaluator. Ensure the fix doesn't introduce regressions identified in session history."
        LOOP_TYPE="VALIDATED"; VAL_CMD="cargo check" ;;

    # --- Advanced bash Dev ---
    bash-refactor)
        ARCHITECT_INIT="Shell Scripting Expert. Propose POSIX-compliant refactorings for efficiency and safety."
        WORKER_INIT="Bash Specialist. Apply 'set -euo pipefail', quote variables, and remove bashisms."
        CRITIC_INIT="Linux Hardening Expert. Review for injection flaws and unquoted variables."
        LOOP_TYPE="VALIDATED"; VAL_CMD="shellcheck" ;;

    # --- Git & CI/CD Workflow ---
    git-feature-flow)
        ARCHITECT_INIT="Workflow Manager. Design branch naming and atomic commit strategy for: $USER_GOAL."
        WORKER_INIT="Git Agent. Create feature branches and prepare local commits."
        CRITIC_INIT="QA Lead. Ensure branch strategy aligns with team conventions."
        LOOP_TYPE="STANDARD" ;;
    git-feature-flow2)
        ARCHITECT_INIT="Release Engineer. Plan a clean feature-branch strategy."
        WORKER_INIT="Git Agent. Commands: git checkout -b, git push --set-upstream. Ensure branch naming is semantic."
        CRITIC_INIT="QA Lead. Validate branch naming and push commands."
        LOOP_TYPE="STANDARD" ;;
    git-pr-lifecycle)
        ARCHITECT_INIT="PR Strategist. Determine if changes meet repository contribution guidelines."
        WORKER_INIT="PR Agent. Write PR descriptions, evaluate incoming PR diffs, and apply patches from remote contributors."
        CRITIC_INIT="Senior Reviewer. Evaluate PR for breaking changes or security regressions."
        LOOP_TYPE="STANDARD" ;;

    # --- Tooling Integration ---
    editor-nav)
        ARCHITECT_INIT="Index Expert. Use ctags/etags output to map project symbols."
        WORKER_INIT="Vim Automation Agent. Generate Vim scripts or macros for bulk refactoring based on symbol maps."
        CRITIC_INIT="Logic Evaluator. Ensure generated scripts maintain code integrity and structure."
        LOOP_TYPE="STANDARD" ;;
    editor-nav2)
        ARCHITECT_INIT="Vim/Ctags Specialist. Map the project structure using symbol indexes."
        WORKER_INIT="Vim Automation Agent. Generate .vim refactoring scripts using Ex commands."
        CRITIC_INIT="Logic Evaluator. Ensure generated scripts maintain code integrity and structure."
        LOOP_TYPE="STANDARD" ;;


    # --- Dynamic Documentation ---
    doc-custom)
        ARCHITECT_INIT="Technical Writer. Plan the $DOC_TYPE for the subject: $EXPLAIN_SUBJECT."
        WORKER_INIT="Documentarian. Author $DOC_TYPE. Include ctags-referenced code navigation if relevant."
        CRITIC_INIT="Editor. Ensure clarity, accuracy, and completeness of the $DOC_TYPE."
        LOOP_TYPE="STANDARD" ;;
    doc-custom2)
        ARCHITECT_INIT="Technical Writer. Plan the $DOC_TYPE regarding $EXPLAIN_SUBJECT."
        WORKER_INIT="Writer. Generate high-quality $DOC_TYPE using information from $TARGET_FILE."
        CRITIC_INIT="Editor. Ensure clarity, accuracy, and completeness of the $DOC_TYPE."
        LOOP_TYPE="STANDARD" ;;

    # --- PR & Feature Lifecycle ---
    git-pr-apply)
        ARCHITECT_INIT="Integration Lead. Evaluate the incoming patch/PR for architectural fit."
        WORKER_INIT="PR Agent. Use 'git apply' or 'git am' to integrate changes. Resolve logical drift."
        CRITIC_INIT="Logic Evaluator. Ensure the PR doesn't violate safety invariants."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;;


    # --- Deep Code Evolution ---
    rust-algo-optimize)
        ARCHITECT_INIT="Algorithm Expert. Propose Big-O improvements (e.g., HashSets vs Vecs)."
        WORKER_INIT="Code Simplifier. Remove redundant complexity. Use better algorithms."
        CRITIC_INIT="Logic Evaluator. Check for subtle logical errors, off-by-ones, or incorrect edge-case handling."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;;

    # --- Toolchain Integration ---

    # --- Crash Analysis & Debugging ---
    debug-core)
        ARCHITECT_INIT="Debugger Specialist. Interpret GDB/LLDB backtraces. Identify the crashing frame and signal (SIGSEGV, SIGABRT)."
        WORKER_INIT="Fix Agent. Analyze $TARGET_FILE (source) at the reported line. Fix null derefs, OOB access, or unwrap() panics."
        CRITIC_INIT="Logic Evaluator. Ensure the fix addresses the root cause, not just the symptom."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;;
    debug-core2)
        # Refined Architect for GDB
        ARCHITECT_INIT="Post-Mortem Specialist. Interpret stack frames and register state from core dumps."
        WORKER_INIT="Vim Fix Agent. Directly edit source files to prevent the identified crash."
        CRITIC_INIT="Logic Evaluator. Ensure fixes address root causes without introducing new issues."
        LOOP_TYPE="VALIDATED"; VAL_CMD="rustc" ;;

    rust-deep-analysis)
        ARCHITECT_INIT="Rust Internalist. Use 'cargo expand' to check macro hygiene and 'cargo bloat' to identify generic monomorphization costs."
        WORKER_INIT="Optimization Expert. Reduce binary size and compile times by optimizing generic usage and macros."
        CRITIC_INIT="Logic Evaluator. Ensure optimizations maintain public API and safety invariants."
        LOOP_TYPE="VALIDATED"; VAL_CMD="cargo-bloat" ;;
    
    rust-meta-opt)
        ARCHITECT_INIT="Rust Metaprogramming Expert. Use 'cargo expand' to inspect macro expansion for hygiene and 'cargo bloat' for monomorphization bloat."
        WORKER_INIT="Refactor Agent. Optimize generic usage and macro definitions to improve compile times and binary footprint."
        CRITIC_INIT="Logic Evaluator. Ensure optimizations don't break the public API or safety invariants."
        LOOP_TYPE="VALIDATED"; VAL_CMD="cargo bloat --release -n 20" ;;
    rust-high-stakes)
        ARCHITECT_INIT="Lead Mediator. Balance extreme safety requirements with high-performance targets."
        WORKER_INIT="Expert Implementer. Write code that satisfies both a pedantic safety auditor and a performance engineer."
        CRITIC_INIT="Safety Critic. Focus on memory safety, race conditions, and edge-case handling."
        # This task flags the loop to use both FDs
        DUO_MODE=true
        LOOP_TYPE="VALIDATED"; VAL_CMD="cargo test" ;; 
    debug-test)
        ARCHITECT_INIT="QA Lead. Analyze test failures and stack traces to find regression roots."
        WORKER_INIT="Fix Agent. Apply source changes or update test mocks to satisfy the test suite."
        CRITIC_INIT="Logic Evaluator. Ensure fixes address root causes without introducing new issues."
        LOOP_TYPE="VALIDATED"; VAL_CMD="cargo test" ;;

    git-bisect-autofix)
        ARCHITECT_INIT="Bisect Coordinator. Manage the binary search state. Determine the 'good' and 'bad' boundaries."
        WORKER_INIT="Git Agent. Execute 'git bisect good/bad'. On the culprit commit, analyze the diff to find the regression."
        CRITIC_INIT="Logic Evaluator. Verify if the identified commit truly contains the root cause of the failure."
        LOOP_TYPE="VALIDATED"; VAL_CMD="cargo test" ;;

    meta-gen)
        ARCHITECT_INIT="System Dispatcher. Your goal is to map a user request to the correct orchestrator task and arguments."
        WORKER_INIT="CLI Generator. Output ONLY the exact bash command to run the orchestrator. Do not explain."
        CRITIC_INIT="Syntax Validator. Ensure the generated command uses valid tasks and flags from the provided usage text."
        LOOP_TYPE="STANDARD" ;;
    chaos-drill)
        ARCHITECT_INIT="Resilience Architect. Build a system that survives hardware and network failures."
        WORKER_INIT="Implementation Lead. Write robust, fault-tolerant code."
        CRITIC_INIT="Chaos Engineer. Introduce random failures (OOM, Latency) and ensure system stability."
        CHAOS_MODE=true
        DUO_MODE=true # Chaos drills benefit from Duo-Critic debate
        LOOP_TYPE="VALIDATED"; VAL_CMD="cargo test" ;;

    test-sanitizer)
        ARCHITECT_INIT="Test Engineer. Generate 5 diverse examples of 'chatty' LLM responses (prose + code)."
        WORKER_INIT="Automation Specialist. Write a bash script that runs 'strip_chatter' against these examples and checks if the output matches the expected 'pure code'."
        CRITIC_INIT="QA Lead. Ensure the script handles edge cases and various formatting styles."
        LOOP_TYPE="VALIDATED"; VAL_CMD="bash" ;;
    onboard)
        # Gather structural metadata
        REPO_TREE=$(tree --charset=ascii --gitignore -P '*.rs|*.sh|*.md' -I 'target|.git')
        CRATE_DEPS=$(cargo metadata --format-version 1 | jq -r '.packages[0].dependencies[].name' | head -n 20)

        GIT_LOG_SUMMARY=$(git log -n 30 --pretty=short |git shortlog)
        # ctags: the TOML parser is broken.
        TOP_SYMBOLS=$(ctags -x --languages=Rust,sh,-TOML,-Cargo --sort=yes $(git ls-files | grep -E '\.(sh|rs)$') | 
        grep -Pv '(test[s_]|[ \t](field|method)[ \t])' | head -n 50)
        ONBOARD="Structure:\n$REPO_TREE\n\nDependencies:\n$CRATE_DEPS\n\nSymbols:\n$TOP_SYMBOLS"
        ;;
    *) usage "Unknown task type: $TASK_TYPE" 1 ;;
esac
case "$TASK_TYPE" in
    git-ops|git-pr-apply|git-bisect-autofix)
        # Force these tasks to use the filtered context
        if [[ -n "$TARGET_COMMIT" ]]; then
            # Replace the heavy 'git show' in query_agent with this:
            GIT_PAYLOAD=$(query_git_context "$TARGET_COMMIT")
        fi
        ;;
esac
# Append to all _INIT variables
# Append to all system prompts to enforce high-signal communication
DENSE_SIGNAL="Instruction: Use Delimiters (###) for sections. Use 'Chain of Thought' (First, analyze... Then, implement...). Avoid all pleasantries. If providing code, provide ONLY the code."

ARCHITECT_INIT+=" $DENSE_SIGNAL Task: Create a structured prompt for the Worker that includes: 1. Input Context 2. Constraints 3. Expected Output Format."
WORKER_INIT+=" $STRICT_FORMAT"

# --- Infrastructure Setup ---
SESSION_FILE="/tmp/ruchat_session_state.json"
PERSIST_DIR="${HOME}/.ruchat/sessions"
mkdir -p "$PERSIST_DIR"

save_session() {
    local session_name="${1:-session_$(date +%Y%m%d_%H%M%S)}"
    local target_path="${PERSIST_DIR}/${session_name}"
    mkdir -p "$target_path"

    cp "$HISTORY_FILE" "${target_path}/history.log"
    cp "$SESSION_FILE" "${target_path}/state.json"
    
    # Save target files to track drift
    [[ -f "$TARGET_FILE" ]] && cp "$TARGET_FILE" "${target_path}/last_known_file"
    
    echo -e "${C_SUMM}SESSION SAVED: ${session_name}${NC}"
}

resume_session() {
    local session_name="$1"
    local target_path="${PERSIST_DIR}/${session_name}"

    if [[ -d "$target_path" ]]; then
        cp "${target_path}/history.log" "$HISTORY_FILE"
        cp "${target_path}/state.json" "$SESSION_FILE"
        echo -e "${C_VALI}SESSION RESUMED: ${session_name}${NC}"
    else
        echo -e "${C_CRIT}ERROR: Session '${session_name}' not found.${NC}"
        exit 1
    fi
}
# Initialize session if not exists
# Initialize extended session if not exists
if [[ ! -f "$SESSION_FILE" ]]; then
    echo '{"history": [], "last_task": "", "total_tokens": 0, "total_cost": 0.0, "debate_logs": []}' > "$SESSION_FILE"
fi
estimate_costs() {
    echo -e "${C_SUMM}--- PRE-FLIGHT TOKEN ESTIMATE ---${NC}"
    
    # Calculate potential payload size
    local payload_size=0
    if [[ -n "$TARGET_COMMIT" ]]; then
        # Simulate query_git_context
        payload_size=$(git show --abbrev-commit --stat -p -U1 "$TARGET_COMMIT" | wc -c)
    elif [[ -f "$TARGET_FILE" ]]; then
        payload_size=$(wc -c < "$TARGET_FILE")
    fi

    local est_input_tokens=$(( payload_size / 4 ))
    local est_per_round=$(( (est_input_tokens + 500) * 4 )) # (Payload + Prompt) * Agents
    local total_est=$(( est_per_round * ITERATIONS ))

    echo "Mode: $TASK_TYPE"
    echo "Estimated Session Tokens: $total_est"
    echo "Estimated Cost: \$$(awk "BEGIN {print $total_est / 1000 * 0.002}")"
    
    read -p "Begin Orchestration? (y/n) " -n 1 -r; echo
    echo -e "The replay was '$REPLY'"
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo -e "${C_CRIT}Orchestration aborted by user.${NC}"
        exit 0
    fi
}
if [[ ! -f "tags" ]] && command -v ctags &> /dev/null; then
    echo -e "${C_SUMM}SYSTEM: Generating symbol map (ctags)...${NC}"
    # Generate tags for Rust, ignoring target and hidden dirs
    ctags -R --languages=Rust,sh,-TOML,-Cargo --exclude=target --exclude=.git . 2>/dev/null &
fi

# --- Initialization Phase ---
if [[ -z "$TARGET_FILE" && "$TASK_TYPE" != "meta-gen" ]]; then
    echo -e "${C_SUMM}SYSTEM: No target file provided. Discovering context...${NC}"
    DISCOVERED_FILES=$(discover_relevant_files "$SUBJECT")
    
    if [[ -z "$DISCOVERED_FILES" ]]; then
        echo -e "${C_CRIT}WARNING: No relevant files found in repository.${NC}"
    else
        echo -e "${C_VALI}FOUND CONTEXT:${NC}\n$DISCOVERED_FILES"
    fi
fi

# estimate tokens and cost in dry-run mode, optionally exit before execution
if [[ "$DRY_RUN" == "true" ]]; then
    estimate_costs
fi

trap cleanup EXIT

cleanup # Ensure no stale pipes exist

if [[ -n "${ONBOARD:-}" ]]; then
        # Test that target file does not yet exist
        [[ -z "${TARGET_FILE:-}" ]] && usage "Onboard task requires --file <path> to specify an output." 1
        if [[ -f "${TARGET_FILE:-}" ]]; then
            usage "Onboard task requires a non-existent target file to avoid overwriting." 1
        fi

        # Initialize Summarizer Agent FDs
        mkfifo /tmp/ruchat_summarizer_in /tmp/ruchat_summarizer_out
        exec {FD_SUMM_IN}>/tmp/ruchat_summarizer_in {FD_SUMM_OUT}</tmp/ruchat_summarizer_out
        $RUCHAT_BIN pipe -m "${MODELS[summarizer]}" -o "{\"temperature\": $TEMP_STRICT }" < /tmp/ruchat_summarizer_in > /tmp/ruchat_summarizer_out &
        # 3. Query the Summarizer
        ONBOARD_RES=$(query_agent "$FD_SUMM_IN" "$FD_SUMM_OUT" "$C_SUMM" "ONBOARDER" "${USER_GOAL}${ONBOARD}")
        printf "\n---\n" >&"$FD_SUMM_IN" || true
             
        echo -e "$ONBOARD_RES" > "${TARGET_FILE}"
        echo -e "${C_VALI}SUCCESS: Created result file ${TARGET_FILE}${NC}"
        cleanup
        exit 0
fi
for agent in "${AGENTS[@]}"; do
    mkfifo "/tmp/ruchat_${agent}_in" "/tmp/ruchat_${agent}_out"
done

# Start Ruchat Pipes
$RUCHAT_BIN pipe -m "${MODELS[architect]}" -o "{\"temperature\": $TEMP_STRICT}" < /tmp/ruchat_architect_in > /tmp/ruchat_architect_out &
$RUCHAT_BIN pipe -m "${MODELS[worker]}" -o "{\"temperature\": $TEMP_WORKER}" < /tmp/ruchat_worker_in > /tmp/ruchat_worker_out &
[[ "$LOOP_TYPE" = "VALIDATED" ]] && \
  $RUCHAT_BIN pipe -m "${MODELS[validator]}" -o "{\"temperature\": $TEMP_STRICT}" < /tmp/ruchat_validator_in > /tmp/ruchat_validator_out &
$RUCHAT_BIN pipe -m "${MODELS[critic]}" -o "{\"temperature\": $TEMP_STRICT}" < /tmp/ruchat_critic_in > /tmp/ruchat_critic_out &
[[ "$STRIP_CHATTER" = true ]] && \
  $RUCHAT_BIN pipe -m "${MODELS[summarizer]}" -o '{"temperature": 0.0}' < /tmp/ruchat_summarizer_in > /tmp/ruchat_summarizer_out &
[[ "$DUO_MODE" = true ]] && \
  $RUCHAT_BIN pipe -m "${MODELS[critic_perf]}" -o '{"temperature": 0.0}' < /tmp/ruchat_critic_perf_in > /tmp/ruchat_critic_perf_out &
[[ "$CHAOS_MODE" = true ]] && \
  $RUCHAT_BIN pipe -m "${MODELS[chaos]:-mistral}" -o '{"temperature": 1.0}' < /tmp/ruchat_chaos_in > /tmp/ruchat_chaos_out &

# Infrastructure: Add the second Critic pipe

# Role Definitions
CRITIC_SAFETY_INIT="Safety Critic. Focus: Memory safety, race conditions, edge-case handling, and error propagation. Be pedantic."
CRITIC_PERF_INIT="Performance Critic. Focus: Zero-cost abstractions, avoiding heap allocations, Big-O complexity, and cache locality."
exec {FD_ARCH_IN}>/tmp/ruchat_architect_in {FD_ARCH_OUT}</tmp/ruchat_architect_out
exec {FD_WORK_IN}>/tmp/ruchat_worker_in {FD_WORK_OUT}</tmp/ruchat_worker_out
exec {FD_CRIT_IN}>/tmp/ruchat_critic_in {FD_CRIT_OUT}</tmp/ruchat_critic_out

# Conditional opens: Only open if the background process was actually started
if [ "$LOOP_TYPE" = "VALIDATED" ]; then
    exec {FD_VALI_IN}>/tmp/ruchat_validator_in {FD_VALI_OUT}</tmp/ruchat_validator_out
fi

if [ "$STRIP_CHATTER" = true ]; then
    exec {FD_SUMM_IN}>/tmp/ruchat_summarizer_in {FD_SUMM_OUT}</tmp/ruchat_summarizer_out
fi

if [ "$DUO_MODE" = true ]; then
    exec {FD_CRIT_PERF_IN}>/tmp/ruchat_critic_perf_in {FD_CRIT_PERF_OUT}</tmp/ruchat_critic_perf_out
fi

if [ "$CHAOS_MODE" = true ]; then
    exec {FD_CHAOS_IN}>/tmp/ruchat_chaos_in {FD_CHAOS_OUT}</tmp/ruchat_chaos_out
fi

# --- Git Helper Functions ---
query_git_context() {
    local commit="$1"
    git show --abbrev-commit --stat -p -U1 "$commit"
}
list_commit_hashes_for_file() {
    local file="$1"
    git log --pretty="%H" -- "$file"
}
list_files_in_commit() {
    local commit="$1"
    git diff-tree --no-commit-id --name-only "$commit" -r
}
list_associated_files() {
    local target_file="$1"
    local count="${2:-15}"
    local associated_files=""

    # 1. Files changed in the same commits as the target file
    while read -r commit; do
        associated_files+=" "$(list_files_in_commit "$commit")
    done < <(list_commit_hashes_for_file "$target_file")

    # 2. Deduplicate and return
    echo "$associated_files" | tr ' ' '\n' | sort -u | grep -v "^$target_file$" | head -n "$count"
}
sorted_files_on_change() {
    while read -r FILE; do
        git log --pretty="%ad $FILE" --date=iso8601-strict -1 -- "$FILE"
    done < <( git ls-files ) | sort | cut -f 2 -d " "
}
# reads file paths from stdin and outputs a tree structure
treeify() {
    tree --charset=ascii --fromfile /dev/stdin
}
relevant_crates_for_files() {
    local count="${1:-20}"
    # 1. Extract crate names from 'extern crate' and 'use' statements
    set +e
    grep '\.rs$' | xargs head -n "$count" | sed -r -n '/ crate::/b;s/^(extern crate |use )([a-zA-Z0-9_]+).*/\2/p' | sort -u
    set -e
}
top_symbols_for_files() {
    local count="${1:-50}"
    grep -E '\.(rs|sh)$' | xargs ctags -x --languages=Rust,sh,-TOML,-Cargo --sort=yes | \
    grep -Pv '(test[s_]|[ \t](field|method)[ \t])' | head -n "$count"
}
git_log_summary_for_files() {
    local files=("$@")
    local count="${2:-30}"
    git log -n "$count" --pretty=short -- "${files[@]}" | git shortlog
}
get_target_file_metadata() {
    local f="$0"
    [[ ! -f "$f" ]] && return

    echo -e "\n[METADATA: $f]"
    
    # 0. Temporal Coupling (What else changes with this file?)
    local associated; associated=$(list_associated_files "$f" 4)
    [[ -n "$associated" ]] && echo -e "Temporally Associated Files:\n$associated"

    # 1. Crate Dependencies (What external logic does this file rely on?)
    local deps; deps=$(echo "$f" | relevant_crates_for_files 9)
    [[ -n "$deps" ]] && echo -e "Detected Dependencies: $deps"

    # 2. Structural Symbols (High-level API surface)
    if [[ $f =~ \.md$ ]]; then
        # ctags does not support TOML well; skip for markdown files
        echo -e "Skipping Key Symbols for $f (Markdown file)." 1>&2
    else
        echo -e "Key Symbols in $f:"
        echo "$f" | top_symbols_for_files 14
    fi

    # 3. Git History Summary (Recent churn and context)
    echo -e "Recent History Summary:"
    git_log_summary_for_files "$f" 2
}

# --- Helper Functions ---
discover_relevant_files() {
    local subject="${1:-}"
    local files=""

    # 1. Base: Modified & Recent files
    local base_files
    base_files=$(git status --porcelain | awk '{print $2}')
    base_files+=" "$(git log -n 2 --pretty=format: --name-only)

    # 2. Structural: Use ctags to find where symbols in base_files are defined
    if [[ -f "tags" ]]; then
        for f in $base_files; do
            # Extract high-signal keywords (Structs, Enums, Traits) from the base file
            local symbols
            symbols=$(grep -E "(struct|enum|trait) " "$f" 2>/dev/null | awk '{print $2}' | tr -d '[:punct:]')
            
            for sym in $symbols; do
                # Find the file where this symbol is defined
                local def_file
                def_file=$(awk -v s="$sym" '$1 == s {print $2}' tags | head -n 1)
                [[ -n "$def_file" && -f "$def_file" ]] && files+=" $def_file"
            done
        done
    fi

    # 3. Keyword: ripgrep for the subject
    [[ -n "$subject" ]] && files+=" "$(rg -l --max-count 1 "$subject" | head -n 3)

    # Deduplicate and return
    echo "$base_files $files" | tr ' ' '\n' | sort -u | grep -v '^$' | head -n 8
}
maintain_context_efficiency() {
    local history_size
    history_size=$(wc -c < "$HISTORY_FILE")
    
    # 1. Deduplicate File Snapshots (Aggressive Pruning)
    # This removes intermediate [FILE: ...] blocks that haven't changed
    # to stop the "echo effect" in the context window.
    if [[ $history_size -gt 6000 ]]; then
        sed -i -E '/\[FILE: .*\]/,/\[\/FILE\]/ { N; /\[FILE: (.*)\].*\n.*\1/D; }' "$HISTORY_FILE"
    fi

    # 2. Semantic Compression (The Summarizer)
    if [[ $history_size -gt 12000 ]]; then
        echo -e "${C_SUMM}SYSTEM: Context limit approaching. Compressing logic...${NC}"
        local summary
        summary=$(query_agent "$FD_SUMM_IN" "$FD_SUMM_OUT" "$C_SUMM" "SUMMARIZER" \
            "Task: Distill this interaction. 
             Keep: Decisions, specific code fixes, and remaining errors. 
             Discard: General discussion and redundant snapshots. 
             History: $(cat "$HISTORY_FILE")")
        
        echo -e "### SESSION RESUME (Compressed) ###\n$summary" > "$HISTORY_FILE"
    fi
}
strip_chatter() {
    local input="$1"
    # Check if input contains triple backticks
    if [[ "$input" == *"\`\`\`"* ]]; then
        # Extract everything between first and last backticks, remove the markers themselves
        echo "$input" | sed -n '/^```/,/^```/p' | sed '/^```/d'
    else
        # Remove common "AI chatter" prefixes and suffixes
        echo "$input" | sed -E '1,3s/^(Certainly|Sure|Okay|Here is|Based on).*[.:]//gI' | \
                        sed -E '/(If you need|Hope this|Let me know)/d'
    fi
}
run_ripple_validation() {
    local primary_file="$1"
    local val_cmd="$2"
    local report=""

    # 1. Primary Check
    echo -ne "${C_VALI}VALIDATING PRIMARY: $primary_file... "
    set +e
    KEEP_ALIVE=true
    V_RES=$(eval "$val_cmd" 2>&1)
    V_EXIT=$?
    KEEP_ALIVE=false
    set -e
    if [[ $V_EXIT -eq 0 ]]; then
        echo -e "PASSED${NC}" 1>&2
        report+="[Primary: $primary_file Passed]\n"
    else
        # --- POST-MORTEM BRANCH ---
        # Detect Segfaults or Panics (Exit codes 139, 134, or specific strings)
        if [[ $V_EXIT -eq 139 || "$V_LOG" == *"panic"* || "$V_LOG" == *"segfault"* ]]; then
             echo -e "${C_CRIT}CRASH DETECTED. Extracting Post-Mortem...${NC}" 1>&2
             
             # Automatic GDB backtrace for the last binary run
             # (Assumes we are debugging 'target/debug/app')
             gdb -ex "set logging on gdb.log" -ex "run" -ex "bt" -ex "quit" ./target/debug/app &>/dev/null
             
             # Use Smart Validator to explain the crash
             CURRENT_PROMPT=$(query_agent "$FD_VALI_IN" "$FD_VALI_OUT" "$C_VALI" "VALIDATOR" \
                "Binary crashed (Code $V_EXIT). Analysis of log and GDB backtrace: \n$V_LOG\n$(cat gdb.log 2>/dev/null)")
        else
            # Standard compilation error
            CURRENT_PROMPT="Validation Failed ($VAL_CMD):\n$V_LOG"
        fi
        echo "### VALIDATION FAILURE ###"$'\n'"$CURRENT_PROMPT" >> "$HISTORY_FILE"

        echo -e "FAILED${NC}"
        echo "$V_RES"
        return 1
    fi

    # 2. Coupled Check (Check the 'Neighborhood')
    local coupled; coupled=$(list_associated_files "$primary_file" 3)
    if [[ -z "$coupled" ]]; then 
        report+="[Check Passed: $val_cmd]\n"
        return 0; 
    fi

    echo -e "${C_SUMM}CHECKING RIPPLE EFFECTS: $coupled${NC}" 1>&2
    local passed=true
    for f in $coupled; do
        # Check if the coupled file still compiles/tests correctly
        # We target specific tests if it's a Rust project
        if [[ "$val_cmd" == *"cargo"* ]]; then
             local test_cmd="cargo test --file $f"
             if ! eval "$test_cmd" &>/dev/null; then
                 report+="[RIPPLE FAILURE: $f might be broken by changes in $primary_file]\n"
                 echo -e "${C_CRIT}WARNING: Ripple failure in $f${NC}" 1>&2
                 passed=false

             fi
        fi
    done
    if [[ "$passed" = true ]]; then
        report+="[Ripple Check Passed: $val_cmd]\n"
    else
        local broken_file; broken_file=$(echo "$VAL_REPORT" | grep "RIPPLE FAILURE" | cut -d' ' -f3 | tr -d ']')
        
        # Get symbols from the primary and the broken file to find the mismatch
        local context_update="Regression detected in $broken_file.\n"
        [[ $TARGET_FILE =~ \.md$ ]] ||
            context_update+="Symbols in $TARGET_FILE:\n$(echo "$TARGET_FILE" | top_symbols_for_files 10)\n"
        [[ $broken_file =~ \.md$ ]] ||
            context_update+="Symbols in $broken_file:\n$(echo "$broken_file" | top_symbols_for_files 10)\n"

        
        CURRENT_PROMPT+="\n$context_update\nFix the interface mismatch."
    fi
    
    echo -e "$report"
    return 0
}

update_usage_stats() {
    local tokens="$1"
    local cost_per_1k="0.002" # Adjust based on your local/API pricing
    local added_cost
    added_cost=$(awk "BEGIN {print $tokens / 1000 * $cost_per_1k}")

    local updated
    updated=$(jq --argjson t "$tokens" --argjson c "$added_cost" \
        '.total_tokens += $t | .total_cost += $c' "$SESSION_FILE")
    echo "$updated" > "$SESSION_FILE"
}
update_session_debate() {
    local file="$1" s_crit="$2" p_crit="$3"
    local entry
    entry=$(jq -n --arg f "$file" --arg s "$s_crit" --arg p "$p_crit" \
        '{file: $f, safety_issue: $s, perf_issue: $p, timestamp: (now | strftime("%H:%M:%S"))}')
    jq ".debate_logs += [$entry]" "$SESSION_FILE" > "${SESSION_FILE}.tmp" && mv "${SESSION_FILE}.tmp" "$SESSION_FILE"
}
update_session() {
    local task="$1" cmd="$2"
    # Use jq to append the latest command and task to history
    local updated
    updated=$(jq --arg t "$task" --arg c "$cmd" \
        '.last_task = $t | .history += [$c]' "$SESSION_FILE")
    echo "$updated" > "$SESSION_FILE"
}

get_session_context() {
    jq -r '"[SESSION HISTORY]\nLast Task: " + .last_task + 
           "\nTokens Used: " + (.total_tokens|tostring) + 
           "\nEst. Cost: $" + (.total_cost|tostring) + 
           "\nCommand History: " + (.history | join(", "))' "$SESSION_FILE"
}
query_git_context() {
    local target="$1"
    # -w ignores whitespace, -U1 limits context lines to save tokens
    if git rev-parse "$target" >/dev/null 2>&1; then
        echo "--- SKINNY COMMIT ($target) ---"
        git show --abbrev-commit --stat -w -p -U1 "$target" | sed 's/^[[:space:]]*//'
    else
        echo "--- SKINNY DIFF ---"
        git diff -w -U1 "$target"
    fi
}
generate_file_manifest() {
    echo "[FILE MANIFEST]"
    # Provides: filename | short_hash | line_count
    find . -maxdepth 2 -name "*.rs" -exec du -b {} + | awk '{print $2 " | " $1 " bytes"}'
}
compress_context() {
    local long_context="$1"
    if [[ ${#long_context} -gt 4000 ]]; then
        echo -e "${C_VALI}SYSTEM: Context too large. Compressing...${NC}"
        query_agent "$FD_SUMM_IN" "$FD_SUMM_OUT" "$C_SUMM" "SUMMARIZER" \
            "Compress the following technical history into a dense, bulleted list of facts, decisions, and errors. Remove all prose: $long_context"
    else
        echo "$long_context"
    fi
}
run_bisect_session() {
    local good_rev="$1"
    local bad_rev="$2"
    
    echo -e "${C_SYS}SYSTEM: Starting Automated Bisect from $good_rev to $bad_rev${NC}"
    git bisect start "$bad_rev" "$good_rev"

    # Use 'git bisect run' to automate the logic engine as the decider
    git bisect run bash -c "
        # Run the orchestrator in a non-interactive mode for a single iteration
        # If orchestrator returns 0 (APPROVED), git treats commit as 'good'
        ./orchestrator.sh debug-test --iter 1 --file $TARGET_FILE
    "
    
    echo -e "${C_SUMM}SYSTEM: Bisect Complete. Culprit identified.${NC}"
    git bisect reset
}
query_agent() {
    local in_fd=$1 out_fd=$2 color=$3 name=$4 prompt=$5
    local context=""

    # 1. Identity & Meta-Stats
    if [[ "$TASK_TYPE" == "meta-gen" ]]; then
        context+="\n$(get_session_context)\n"
        context+="\n[CAPABILITIES]\n$(usage)\n"
    fi

    # 2. Source Ground-Truth (Mutual Exclusivity Logic)
    if [[ -n "${GIT_PAYLOAD:-}" ]]; then
        context+="\n[SKINNY DIFF]\n$GIT_PAYLOAD\n"
    elif [[ -n "$TARGET_COMMIT" ]]; then
        context+="\n[COMMIT]\n$(git show --abbrev-commit --stat -p -U1 "$TARGET_COMMIT")\n"
    fi

    # --- Target File Enrichment ---
    if [[ -f "$TARGET_FILE" ]]; then
        # Inject the specialized Git/Symbol metadata
        [[ "$TARGET_FILE" == *".md" ]] ||
            context+="$(get_target_file_metadata "$TARGET_FILE")\n"
        
        # Inject full content of the primary file
        context+="\n[SOURCE: $TARGET_FILE]\n$(cat "$TARGET_FILE")\n"
    fi

    # Handle Multi-File Context
    if [[ -n "${DISCOVERED_FILES:-}" ]]; then
        context+="\n[LOGICAL NEIGHBORHOOD TREE]\n"
        context+="$(echo "$DISCOVERED_FILES" | treeify)\n"

        for f in $DISCOVERED_FILES; do
            if [[ "$f" == "$TARGET_FILE" ]]; then
                context+="\n[PRIMARY FILE: $f]\n$(cat "$f")\n"
            else
                context+="\n[REFERENCE FILE: $f]\n$(head -n 50 "$f")\n... (truncated)\n"
            fi
        done
    elif [[ -f "$TARGET_FILE" ]]; then
        context+="\n[FILE: $TARGET_FILE]\n$(cat "$TARGET_FILE")\n"
    fi

    # 3. Dynamic Tooling Context
    case "$TASK_TYPE" in
        editor-nav)
            [[ -f "tags" ]] && context+="\n[CTAGS]\n$(grep "${TARGET_FILE:-.*}" tags | head -n 30)\n" ;;
        rust-meta-opt)
            context+="\n[BLOAT]\n$(cargo bloat --release -n 5 2>/dev/null)\n" ;;
        debug-core)
            # Post-Mortem data from the GDB branch in the main loop
            [[ -f "gdb.log" ]] && context+="\n[GDB BACKTRACE]\n$(cat gdb.log)\n" ;;
    esac

    # 4. Transmission & Reception
    printf "%b\n%b\n---\n" "$context" "$prompt" >&"$in_fd"
    
    local line full_resp=""
    while IFS= read -u "$out_fd" -r line; do
        [[ "$line" == "---" ]] && break
        if [[ "$line" == "USAGE:"* ]]; then
            update_usage_stats "$(echo "$line" | cut -d: -f2 | jq -r .tokens)"
            continue
        fi
        full_resp+="$line"$'\n'
    done

    # 5. Sanitization & Persistence
    local clean_resp
    clean_resp=$(strip_chatter "$full_resp")
    echo "### ${name} ###"$'\n'"${clean_resp}" >> "$HISTORY_FILE"
    
    # Visual Output
    printf "${color}${name}:${NC}\n%s\n" "$clean_resp" >&2
    echo "$clean_resp"
}

validate_output() {
    local text="$1" engine="$2"
    local total_errors=""
    local block_count=0

    # Extract all code blocks using a while-read loop and sed
    while IFS= read -r block; do
        [[ -z "$block" ]] && continue
        ((block_count++))
        
        local tmp_file="/tmp/ruchat_eval_block_${block_count}_$(date +%s)"
        echo "$block" > "$tmp_file"
        
        local err=""
        case "$engine" in
            shellcheck) err=$(shellcheck "$tmp_file" 2>&1 || true) ;;
            rustc)      err=$(rustc --crate-type lib "$tmp_file" -o /dev/null 2>&1 || true) ;;
            clippy)     err=$(cargo clippy --message-format=short 2>&1 || true) ;; # Assumes context is a crate
            awk|sed)    err=$(echo "test" | "$engine" -f "$tmp_file" 2>&1 >/dev/null || true) ;;
            "cargo test")
                echo -e "${C_VALI}SYSTEM: Executing tests...${NC}"
                # Capture stdout/stderr, including colored output for the LLM to see "Expected vs Actual"
                local test_log
                test_log=$(cargo test -- --nocapture 2>&1 || true)
                
                if [[ "$test_log" == *"test result: ok"* ]]; then
                    echo "SUCCESS"
                else
                    # Extract only the failing test failures to save context window tokens
                    local failure_report
                    failure_report=$(echo "$test_log" | sed -n '/----/,/test result:/p')
                    echo -e "FAIL: Test suite failed.\n$failure_report"
                fi
                ;;
        esac

        if [[ -n "$err" && "$err" == *"error"* ]]; then
            total_errors+="Block #$block_count Error: $err\n"
        fi
        rm -f "$tmp_file"
    done < <(echo "$text" | sed -n '/^```/,/^```/ { /^```/d; p; }')

    if [[ $block_count -eq 0 ]]; then
        echo "FAIL: No code blocks found to validate."
    elif [[ -z "$total_errors" ]]; then
        echo "SUCCESS"
    else
        echo -e "FAIL:\n$total_errors"
    fi
}

# --- Main Logic Engine ---
if [ "$USER_GOAL" == "" ]; then
  echo -e "${C_SUMM}TASK:${NC} Enter goal for $TASK_TYPE:"
  read -r USER_GOAL
else
  echo -e "${C_SUMM}TASK:${NC} Using provided goal for $TASK_TYPE."
fi

# --- Meta-Agent Dispatcher (exists early and recurses) ---
if [[ "$TASK_TYPE" == "meta-gen" ]]; then
    echo -e "${C_SUMM}SYSTEM: Meta-Agent analyzing session and request...${NC}"
    for i in $(seq 1 1000); do
        echo -e "${C_SUMM}--- DISPATCHER ITERATION $i ---${NC}" 
        CMD_TO_RUN=$(query_agent "$FD_WORK_IN" "$FD_WORK_OUT" "$C_WORK" "DISPATCHER" \
            "Context: $(get_session_context). User Request: '$USER_GOAL'. Generate the next orchestrator command.")

        CMD_CLEAN=$(echo "$CMD_TO_RUN" | sed -n '/^```/,/^```/ { /^```/d; p; }' | head -n 1)
        [[ -z "$CMD_CLEAN" ]] && CMD_CLEAN="$CMD_TO_RUN"
        [[ -n "$CMD_CLEAN" ]] && {
            [[ "$DEBUG" == "true" ]] && CMD_CLEAN="$CMD_CLEAN --debug"
        }
    
        echo -e "${C_VALI}DISPATCHER SUGGESTS:${NC} $CMD_CLEAN"
        
        read -p "Execute? (y/n/q): " -n 1 -r; echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            update_session "$TASK_TYPE" "$CMD_CLEAN"
            cleanup
            eval "$CMD_CLEAN"
            exit $?
        fi
        if [[ $REPLY =~ ^[Qq]$ ]]; then
            echo "Exiting Meta-Agent Dispatcher."
            break
        fi
    done
    exit 0
fi
CHAOS_INIT="System: You are to inject random system constraints to test resilience."
# --- Consolidated Main Logic Engine ---
if [[ "$RESUME_MODE" == "true" ]]; then
    resume_session "$SESSION_NAME"
    # This function copies persistent logs into /tmp/ruchat_history.log
    # so the Architect sees the previous context in Round 1.
else
    # Standard fresh start
    echo > "$HISTORY_FILE"
    echo '{"history": [], "last_task": "", "total_tokens": 0, "total_cost": 0.0, "debate_logs": []}' > "$SESSION_FILE"
fi
CURRENT_PROMPT="Goal: $USER_GOAL"

# --- Final Main Execution Logic ---
for i in $(seq 1 "$ITERATIONS"); do
    echo -e "\n${C_VALI}--- ROUND $i ---${NC}"

    # 1. HOUSEKEEPING
    # Ensure context is lean before the Architect sees it
    maintain_context_efficiency 

    # 2. CHAOS INJECTION (Environmental constraints)
    if [[ "$CHAOS_MODE" == "true" ]]; then
        CHAOS_EVENT=$(query_agent "$FD_CHAOS_IN" "$FD_CHAOS_OUT" "$C_SUMM" "CHAOS" \
            "$CHAOS_INIT Inject hazard based on files: $TARGET_FILE.")
        CHAOS_INIT="System: Continue chaos injection."
        # Crucial: Prepend to the current goal so the Architect prioritizes the hazard
        CURRENT_PROMPT="[ENVIRONMENTAL HAZARD: $CHAOS_EVENT] Context: $CURRENT_PROMPT"
    fi

    # 3. ARCHITECT: Strategy & Payload Preparation
    # Optimization: Set Git context globally for this round
    if [[ "$TASK_TYPE" == "git-ops" || "$TASK_TYPE" == "git-pr-apply" ]]; then
        export GIT_PAYLOAD=$(query_git_context "$TARGET_COMMIT")
    fi

    # Branch logic for Prompt Optimization vs Standard
    if [[ "$TASK_TYPE" == "prompt-opt" ]]; then
        PLAN=$(query_agent "$FD_ARCH_IN" "$FD_ARCH_OUT" "$C_ARCH" "ARCHITECT" \
            "$ARCHITECT_INIT\nContext:\n$(cat "$HISTORY_FILE")\n\nTask: $CURRENT_PROMPT")
    else
        # Standard: Compress only the immediate prompt, not the system instructions
        CLEAN_PROMPT=$(compress_context "$CURRENT_PROMPT")
        PLAN=$(query_agent "$FD_ARCH_IN" "$FD_ARCH_OUT" "$C_ARCH" "ARCHITECT" \
            "$ARCHITECT_INIT\nHistory:\n$(cat "$HISTORY_FILE")\n\nTask: $CLEAN_PROMPT")
    fi
    # Clear Architect system prompt for Round 2+
    ARCHITECT_INIT="System: Continue technical plan. Keep logic dense."

    # 4. WORKER: Implementation
    WORKER_OUT=$(query_agent "$FD_WORK_IN" "$FD_WORK_OUT" "$C_WORK" "WORKER" "$WORKER_INIT\nPlan: $PLAN")
    [[ -n "$STRIP_CHATTER" ]] && WORKER_OUT=$(strip_chatter "$WORKER_OUT")
    WORKER_INIT="You are the IMPLEMENTATION AGENT. 
CONSTRAINTS:
1. Output RAW CODE BLOCKS or VIM SCRIPTS only.
2. API STABILITY: Your changes will be tested against 'Associated Files' (temporally coupled modules). 
3. If you modify a public Struct, Trait, or Function signature, you MUST ensure all coupled files provided in the context are updated or that the change is backward-compatible.
4. Minimize 'Diff Noise' to keep the validation rounds fast."

    # 5. ACTION & VALIDATION (The "Real World" result)
    VAL_REPORT=""
    
    # Apply Vim Edits if generated
    if [[ "$WORKER_OUT" == *":%s/"* || "$WORKER_OUT" == *'```vim'* ]]; then
        VIM_SCRIPT=$(echo "$WORKER_OUT" | sed -n '/^```\(vim\)\?$/,/^```$/p' | sed '1d;$d')
        [[ -z "$VIM_SCRIPT" ]] && VIM_SCRIPT=$(echo "$WORKER_OUT" | grep -E '^(:|%|s/)')
        
        if [[ -n "$VIM_SCRIPT" && -f "$TARGET_FILE" ]]; then
            TMP_VIM="/tmp/apply_fix.vim"
            { echo "set backup"; echo "$VIM_SCRIPT"; echo "wq"; } > "$TMP_VIM"
            if vim -u NONE -es -S "$TMP_VIM" "$TARGET_FILE"; then
                VAL_REPORT+="[Vim changes applied to $TARGET_FILE] "
            else
                VAL_REPORT+="[Vim execution FAILED] "
            fi
        fi
    fi

    # Smart Validation (Compiler/Check) with Post-Mortem Analysis
    if [[ "$LOOP_TYPE" == "VALIDATED" ]]; then
        # Pass the primary file and the validation command
        if ! VAL_REPORT=$(run_ripple_validation "$TARGET_FILE" "$VAL_CMD"); then
            # If primary fails, extract log and loop back
            CURRENT_PROMPT="Validation Failed:\n$VAL_REPORT\nAdjust implementation."
            continue
        fi
        
        # If primary passed but ripple failed, the Critic gets the report
        if [[ "$VAL_REPORT" == *"RIPPLE FAILURE"* ]]; then
            echo -e "${C_CRIT}SYSTEM: Primary passed, but regressions detected in coupled files.${NC}"
        fi
    fi
    # 6. CRITICISM (Multi-Agent or Single)
    if [[ "${DUO_MODE:-false}" == "true" ]]; then
        echo -e "${C_CRIT}SYSTEM: Initiating Multi-Agent Debate...${NC}"
        
        REVIEW_SAFETY=$(query_agent "$FD_CRIT_IN" "$FD_CRIT_OUT" "$C_CRIT" "SAFETY" "$CRITIC_SAFETY_INIT Review: $WORKER_OUT")
        CRITIC_SAFETY_INIT="System: Continue safety audit."
        
        REVIEW_PERF=$(query_agent "$FD_CRIT_PERF_IN" "$FD_CRIT_PERF_OUT" "$C_PERF" "PERFORMANCE" "$CRITIC_PERF_INIT Review: $WORKER_OUT")
        CRITIC_PERF_INIT="System: Continue performance audit."

        if [[ "$REVIEW_SAFETY" == *"APPROVED"* && "$REVIEW_PERF" == *"APPROVED"* ]]; then
            update_session_debate "${TARGET_FILE}" "APPROVED" "APPROVED"
            break
        else
            update_session_debate "${TARGET_FILE}" "$REVIEW_SAFETY" "$REVIEW_PERF"
            CURRENT_PROMPT="DEBATE CONFLICT:\nSafety: $REVIEW_SAFETY\nPerformance: $REVIEW_PERF"
        fi
    elif [[ -n "${CRITIC_INIT:-}" ]]; then
        # Single Critic
        CRITIC_PROMPT="Goal: $USER_GOAL\nAction Result: $VAL_REPORT\nWorker Output: $WORKER_OUT"
        REVIEW=$(query_agent "$FD_CRIT_IN" "$FD_CRIT_OUT" "$C_CRIT" "CRITIC" "$CRITIC_INIT\n$CRITIC_PROMPT")
        CRITIC_INIT="System: Continue logic review."
        
        [[ "$REVIEW" == *"APPROVED"* ]] && break
        CURRENT_PROMPT="Critic rejected previous attempt. Feedback: $REVIEW"
    else
        # No Critic, assume success
        break
    fi
done
# --- Session Persistence ---
if [[ "$SAVE_MODE" == "true" ]]; then
    # If no name was provided, the function handles timestamping
    save_session "$SESSION_NAME"
fi

# --- Final Summarization ---
$RUCHAT_BIN pipe -m "${MODELS[summarizer]}" -o "{\"temperature\": ${TEMP_STRICT}}" < /tmp/ruchat_summarizer_in > /tmp/ruchat_summarizer_out &
exec {FD_SUMM_IN}>/tmp/ruchat_summarizer_in {FD_SUMM_OUT}</tmp/ruchat_summarizer_out

echo -e "\n${C_SUMM}SYSTEM: Finalizing documentation...${NC}"
query_agent "$FD_SUMM_IN" "$FD_SUMM_OUT" "$C_SUMM" "SUMMARY" "Create README.md from history: $(cat "$HISTORY_FILE")"
cleanup
