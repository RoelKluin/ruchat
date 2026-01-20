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

SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
ARGS=()
TARGET_FILE=""
TARGET_CRATE=""
DOC_TYPE="README.md"
SUBJECT=""
TASK_TYPE=""
USER_GOAL=""

tasks() {
    cat <<EOF 1>&2
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
  doc-custom2          Enhanced context-aware documentation generation.

Custom:
  custom               User-defined task with specific agent roles and validation.

EOF
}

usage() {
    case "$1" in
        model*|option*|agent*) "$SCRIPT_DIR/ruchat_orchestrator.sh" --help "$1";;
        task*) tasks;;
        all) 
            echo -e "Usage: $0 [options] <task> \"<goal>\"\n\n" 1>&2
            "$SCRIPT_DIR/ruchat_orchestrator.sh" --help all 1>&2
            echo -e "\nAvailable Tasks:\n" 1>&2
            tasks;;
        *)
            echo -e "Usage: $0 [options] <task> \"<goal>\"\n" 1>&2
            echo "Task: Specify the orchestrator task to perform. Use '$0 task' to list available tasks." 1>&2
            echo -e "Goal: A concise description of the user's objective for the task.\n" 1>&2
            echo "Options:" 1>&2
            "$SCRIPT_DIR/ruchat_orchestrator.sh" --help options 1>&2
            [[ -n "$1" ]] && echo -e "Error: $1\n\n" 1>&2
            exit "${2-0}";;
    esac
    exit 0
}

add() {
    ARGS+=("--$1" "$2")
}

# Main Argument Loop Update
while [[ $# -gt 0 ]]; do
    case "$1" in
        -h|--help|help)
            usage "${2-""}" 0 ;;
        -*)
            case "$1" in
                --file) TARGET_FILE="$2";;
                --crate) TARGET_CRATE="$2";;
                --subject) SUBJECT="$2";;
                --doc-type) DOC_TYPE="$2";;
                -D|--debug)
                    ARGS+=("--debug")
                    set -Tx;;
            esac
            ARGS+=("$1")
            shift
            if [[ -n "${1%%-*}" ]]; then
                ARGS+=("$1")
                shift
            fi
            ;;
        *)
            if [[ -z "${TASK_TYPE:-}" ]]; then
                TASK_TYPE="$1"
                shift
            elif [[ -z "${USER_GOAL:-}" ]]; then
                USER_GOAL="${1%.}."
                shift
            else
                usage "Unknown argument: $1" 1
            fi
    esac
done

case "$TASK_TYPE" in
    prompt-opt)
        add "init-architect" "Meta-Prompt Engineer. Your job is to transform user goals into highly optimized prompts for a Worker agent. Use Delimited instructions, Few-shot examples, and Chain-of-Thought triggers."
        add "init-worker" "Executor. Follow the optimized prompt exactly."
        add "init-critic" "Prompt Auditor. Ensure the prompt is clear, concise, and unambiguous."
        ;;
    qa)
        add "init-architect" "QA Architect. Define edge cases and unit test requirements."
        add "init-worker" "Test Engineer. Write robust tests and cargo audit patches."
        add "init-critic" "Security Auditor. Identify gaps in test coverage."
        add "val-cmd" "cargo test --no-run" ;;
    shell)
        add "init-architect" "Systems Architect. Design POSIX-compliant script logic."
        add "init-worker" "Bash Expert. Write efficient scripts with error handling."
        add "init-critic" "Linux Hardening Expert. Review for quoting and injection flaws."
        add "val-cmd" "shellcheck" ;;
    # --- Rust Specialized ---
    rust-analysis)
        add "init-architect" "Senior Rust Architect. Focus: Ownership, Borrowing, and Unsafe Auditing."
        add "init-worker" "Rust Expert. Write safe, idiomatic code. Use 'cargo check' compatible syntax."
        add "init-critic" "Pedantic Rust Reviewer. Hunt for memory leaks and race conditions."
        add "val-cmd" "cargo check" ;;
    rust-analysis2)
        add "init-architect" "Rust Safety Engineer. Identify ownership, borrowing, and unsafe code issues."
        add "init-worker" "Code Auditor. Suggest fixes for memory safety and concurrency."
        add "init-critic" "Pedantic Reviewer. Ensure idiomatic Rust and no unsafe blocks remain."
        add "val-cmd" "rustc" ;;
    rust-explain)
        add "init-architect" "Rust Educator. Breakdown ownership, lifetimes, and trait bounds."
        add "init-worker" "Technical Writer. Explain code using analogies and memory diagrams."
        add "init-critic" "Clarity Editor. Ensure explanations are accurate and accessible."
        ;;
    rust-clippy)
        add "init-architect" "Rust Lint Specialist. Interpret Clippy output for performance/idiomatic improvements."
        add "init-worker" "Refactoring Expert. Apply suggested fixes to code blocks."
        add "init-critic" "Code Reviewer. Ensure changes align with best practices."
        add "val-cmd" "rustc" ;;
    rust-test)
        add "init-architect" "SDET. Identify edge cases, panics, and boundary conditions in Rust modules."
        add "init-worker" "Test Engineer. Write #[cfg(test)] modules and integration tests."
        add "init-critic" "QA Lead. Ensure tests cover all critical paths and validate assertions."
        add "val-cmd" "rustc" ;;

    # --- Git & History ---
    git-ops)
        add "init-architect" "Git Workflow Specialist. Focus: Atomic commits and history legibility."
        add "init-worker" "Automation Expert. Generate git commands/scripts for complex rebasing."
        add "init-critic" "QA Lead. Ensure operations are non-destructive and semantic."
        ;;
    git-history-audit)
        add "init-architect" "Repo Historian. Analyze 'git log --graph' and 'git blame' to find technical debt origins."
        add "init-worker" "Analyst. Summarize development direction and identify 'hot' files with high churn."
        add "init-critic" "Senior Reviewer. Spot security regressions or logic flaws in commit history."
        ;;
    git-commit-gen)
        add "init-architect" "Semantic Versioning Expert. Group changes into atomic, logical units."
        add "init-worker" "Git Expert. Write Conventional Commit messages (feat/fix/chore)."
        add "init-critic" "Senior Reviewer. Ensure commit messages are clear and follow guidelines."
        ;;
    git-conflict-solver)
        add "init-architect" "Conflict Mediator. Analyze HEAD vs Incoming changes in Rust/Bash files."
        add "init-worker" "Git Surgeon. Resolve conflicts manually while preserving logic from both sides."
        add "init-critic" "Logic Evaluator. Ensure merged code compiles and passes existing tests."
        add "val-cmd" "rustc" ;;

    # --- Stream Editing & Automation ---
    stream-edit-suite)
        add "init-architect" "Automation Architect. Plan multi-stage transformations using Sed, Awk, and Perl."
        add "init-worker" "RegEx Wizard. Provide optimized one-liners for bulk code changes."
        add "init-critic" "Safety Engineer. Verify patterns won't cause accidental data loss."
        add "val-cmd" "shellcheck" ;;
    
    # --- Performance ---
    profiling-analysis)
        add "init-architect" "Performance Engineer. Interpret Flamegraphs, 'perf' output, and 'cargo-expand'."
        add "init-worker" "Optimization Expert. Identify bottlenecks and suggest 'inline' or 'unroll' strategies."
        add "init-critic" "Senior Reviewer. Ensure optimizations don't compromise safety or readability."
        ;;

    # --- Documentation ---
    docs)
        add "init-architect" "Technical Writer. Plan documentation structure (README/Rustdoc)."
        add "init-worker" "Markdown Specialist. Write clear, technical explanations."
        add "init-critic" "Editor. Check for clarity and missing technical details."
        ;;
    doc-gen)
        add "init-architect" "Technical Documentarian. Plan README.md, CHANGELOG.md, and Rustdoc architecture."
        add "init-worker" "Writer. Generate doc comments (///) and examples that actually compile."
        add "init-critic" "Documentation Reviewer. Ensure examples are accurate and compile without errors."
        add "val-cmd" "rustc" ;; # Ensures doc examples compile

    # --- Advanced Rust Dev ---
    rust-deps)
        add "init-architect" "Cargo Specialist. Imagine relevant crates for the goal: $USER_GOAL. Suggest features to enable."
        add "init-worker" "Dependency Manager. Edit Cargo.toml. Maintain MSRV and version compatibility."
        add "init-critic" "Dependency Auditor. Check for crate bloat or security advisories."
        ;;
    rust-deps2)
        add "init-architect" "Crate Scout. Search for crates relevant to: $USER_GOAL."
        add "init-worker" "Cargo Specialist. Update Cargo.toml dependencies and features."
        add "init-critic" "Dependency Auditor. Check for crate bloat or security advisories."
        add "val-cmd" "cargo check" ;;
    rust-refactor)
        add "init-architect" "Algorithm Expert. Propose more efficient Big-O complexity or cache-friendly patterns."
        add "init-worker" "Code Simplifier. Refactor logic to reduce cognitive load and remove redundant clones."
        add "init-critic" "Logic Evaluator. Identify non-compiler errors: off-by-one, logic inversions, or re-entrancy bugs."
        add "val-cmd" "rustc" ;;
    rust-crate-expert)
        add "init-architect" "Specialist in the '$TARGET_CRATE' ecosystem. Know the trait patterns and common pitfalls."
        add "init-worker" "API Integrator. Write idiomatic code using $TARGET_CRATE."
        add "init-critic" "Senior Reviewer. Ensure proper usage of $TARGET_CRATE and adherence to best practices."
        add "val-cmd" "rustc" ;;

    rust-fix-loop)
        add "init-architect" "Senior Troubleshooter. Review the last compilation failure from session context."
        add "init-worker" "Fix Agent. Apply changes to $TARGET_FILE to resolve the specific error."
        add "init-critic" "Logic Evaluator. Ensure the fix doesn't introduce regressions identified in session history."
        add "val-cmd" "cargo check" ;;

    # --- Advanced bash Dev ---
    bash-refactor)
        add "init-architect" "Shell Scripting Expert. Propose POSIX-compliant refactorings for efficiency and safety."
        add "init-worker" "Bash Specialist. Apply 'set -euo pipefail', quote variables, and remove bashisms."
        add "init-critic" "Linux Hardening Expert. Review for injection flaws and unquoted variables."
        add "val-cmd" "shellcheck" ;;

    # --- Git & CI/CD Workflow ---
    git-feature-flow)
        add "init-architect" "Workflow Manager. Design branch naming and atomic commit strategy for: $USER_GOAL."
        add "init-worker" "Git Agent. Create feature branches and prepare local commits."
        add "init-critic" "QA Lead. Ensure branch strategy aligns with team conventions."
        ;;
    git-feature-flow2)
        add "init-architect" "Release Engineer. Plan a clean feature-branch strategy."
        add "init-worker" "Git Agent. Commands: git checkout -b, git push --set-upstream. Ensure branch naming is semantic."
        add "init-critic" "QA Lead. Validate branch naming and push commands."
        ;;
    git-pr-lifecycle)
        add "init-architect" "PR Strategist. Determine if changes meet repository contribution guidelines."
        add "init-worker" "PR Agent. Write PR descriptions, evaluate incoming PR diffs, and apply patches from remote contributors."
        add "init-critic" "Senior Reviewer. Evaluate PR for breaking changes or security regressions."
        ;;

    # --- Tooling Integration ---
    editor-nav)
        add "init-architect" "Index Expert. Use ctags/etags output to map project symbols."
        add "init-worker" "Vim Automation Agent. Generate Vim scripts or macros for bulk refactoring based on symbol maps."
        add "init-critic" "Logic Evaluator. Ensure generated scripts maintain code integrity and structure."
        ;;
    editor-nav2)
        add "init-architect" "Vim/Ctags Specialist. Map the project structure using symbol indexes."
        add "init-worker" "Vim Automation Agent. Generate .vim refactoring scripts using Ex commands."
        add "init-critic" "Logic Evaluator. Ensure generated scripts maintain code integrity and structure."
        ;;


    # --- Dynamic Documentation ---
    doc-custom)
        add "init-architect" "Technical Writer. Plan the $DOC_TYPE for the subject: $SUBJECT."
        add "init-worker" "Documentarian. Author $DOC_TYPE. Include ctags-referenced code navigation if relevant."
        add "init-critic" "Editor. Ensure clarity, accuracy, and completeness of the $DOC_TYPE."
        ;;
    doc-custom2)
        add "init-architect" "Technical Writer. Plan the $DOC_TYPE regarding $SUBJECT."
        add "init-worker" "Writer. Generate high-quality $DOC_TYPE using information from $TARGET_FILE."
        add "init-critic" "Editor. Ensure clarity, accuracy, and completeness of the $DOC_TYPE."
        ;;

    # --- PR & Feature Lifecycle ---
    git-pr-apply)
        add "init-architect" "Integration Lead. Evaluate the incoming patch/PR for architectural fit."
        add "init-worker" "PR Agent. Use 'git apply' or 'git am' to integrate changes. Resolve logical drift."
        add "init-critic" "Logic Evaluator. Ensure the PR doesn't violate safety invariants."
        add "val-cmd" "rustc" ;;


    # --- Deep Code Evolution ---
    rust-algo-optimize)
        add "init-architect" "Algorithm Expert. Propose Big-O improvements (e.g., HashSets vs Vecs)."
        add "init-worker" "Code Simplifier. Remove redundant complexity. Use better algorithms."
        add "init-critic" "Logic Evaluator. Check for subtle logical errors, off-by-ones, or incorrect edge-case handling."
        add "val-cmd" "rustc" ;;

    # --- Toolchain Integration ---

    # --- Crash Analysis & Debugging ---
    debug-core)
        add "init-architect" "Debugger Specialist. Interpret GDB/LLDB backtraces. Identify the crashing frame and signal (SIGSEGV, SIGABRT)."
        add "init-worker" "Fix Agent. Analyze $TARGET_FILE (source) at the reported line. Fix null derefs, OOB access, or unwrap() panics."
        add "init-critic" "Logic Evaluator. Ensure the fix addresses the root cause, not just the symptom."
        add "val-cmd" "rustc" ;;
    debug-core2)
        # Refined Architect for GDB
        add "init-architect" "Post-Mortem Specialist. Interpret stack frames and register state from core dumps."
        add "init-worker" "Vim Fix Agent. Directly edit source files to prevent the identified crash."
        add "init-critic" "Logic Evaluator. Ensure fixes address root causes without introducing new issues."
        add "val-cmd" "rustc" ;;

    rust-deep-analysis)
        add "init-architect" "Rust Internalist. Use 'cargo expand' to check macro hygiene and 'cargo bloat' to identify generic monomorphization costs."
        add "init-worker" "Optimization Expert. Reduce binary size and compile times by optimizing generic usage and macros."
        add "init-critic" "Logic Evaluator. Ensure optimizations maintain public API and safety invariants."
        add "val-cmd" "cargo-bloat" ;;
    
    rust-meta-opt)
        add "init-architect" "Rust Metaprogramming Expert. Use 'cargo expand' to inspect macro expansion for hygiene and 'cargo bloat' for monomorphization bloat."
        add "init-worker" "Refactor Agent. Optimize generic usage and macro definitions to improve compile times and binary footprint."
        add "init-critic" "Logic Evaluator. Ensure optimizations don't break the public API or safety invariants."
        add "val-cmd" "cargo bloat --release -n 20" ;;
    rust-high-stakes)
        add "init-architect" "Lead Mediator. Balance extreme safety requirements with high-performance targets."
        add "init-worker" "Expert Implementer. Write code that satisfies both a pedantic safety auditor and a performance engineer."
        add "init-critic" "Safety Critic. Focus on memory safety, race conditions, and edge-case handling."
        # This task flags the loop to use both FDs
        add "val-cmd" "cargo test" ;; 
    debug-test)
        add "init-architect" "QA Lead. Analyze test failures and stack traces to find regression roots."
        add "init-worker" "Fix Agent. Apply source changes or update test mocks to satisfy the test suite."
        add "init-critic" "Logic Evaluator. Ensure fixes address root causes without introducing new issues."
        add "val-cmd" "cargo test" ;;

    git-bisect-autofix)
        add "init-architect" "Bisect Coordinator. Manage the binary search state. Determine the 'good' and 'bad' boundaries."
        add "init-worker" "Git Agent. Execute 'git bisect good/bad'. On the culprit commit, analyze the diff to find the regression."
        add "init-critic" "Logic Evaluator. Verify if the identified commit truly contains the root cause of the failure."
        add "val-cmd" "cargo test" ;;

    meta-gen)
        add "init-architect" "System Dispatcher. Your goal is to map a user request to the correct orchestrator task and arguments."
        add "init-worker" "CLI Generator. Output ONLY the exact bash command to run the orchestrator. Do not explain."
        add "init-critic" "Syntax Validator. Ensure the generated command uses valid tasks and flags from the provided usage text."
        ;;
    chaos-drill)
        add "init-architect" "Resilience Architect. Build a system that survives hardware and network failures."
        add "init-worker" "Implementation Lead. Write robust, fault-tolerant code."
        add "init-critic" "Chaos Engineer. Introduce random failures (OOM, Latency) and ensure system stability."
        add "val-cmd" "cargo test"
        ;;

    test-sanitizer)
        add "init-architect" "Test Engineer. Generate 5 diverse examples of 'chatty' LLM responses (prose + code)."
        add "init-worker" "Automation Specialist. Write a bash script that runs 'strip_chatter' against these examples and checks if the output matches the expected 'pure code'."
        add "init-critic" "QA Lead. Ensure the script handles edge cases and various formatting styles."
        add "val-cmd" "bash"
        ;;
    custom)
        ;;
    *) usage "Unknown task type: $TASK_TYPE" 1 ;;
esac

"$SCRIPT_DIR/ruchat_orchestrator.sh" "${ARGS[@]}" "${TASK_TYPE}" "${USER_GOAL}"

