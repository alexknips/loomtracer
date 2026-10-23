use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use loomtracer::ledger::{EventLedger, SqliteEventLedger};
use serde_json::Value;

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn codex_capture_plugin_bundle_is_installable_and_points_at_cli_capture() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let marketplace = read_json(root.join(".agents/plugins/marketplace.json"))?;
    assert_eq!(marketplace["name"], "loomtracer-plugins");
    assert_eq!(
        marketplace["interface"]["displayName"],
        "Loomtracer Plugins"
    );

    let plugin = marketplace["plugins"]
        .as_array()
        .and_then(|plugins| {
            plugins
                .iter()
                .find(|plugin| plugin["name"] == "loomtracer-capture")
        })
        .expect("loomtracer-capture appears in marketplace");
    assert_eq!(plugin["source"]["source"], "local");
    assert_eq!(plugin["source"]["path"], "./plugins/loomtracer-capture");
    assert_eq!(plugin["policy"]["installation"], "AVAILABLE");
    assert_eq!(plugin["policy"]["authentication"], "ON_INSTALL");

    let manifest = read_json(root.join("plugins/loomtracer-capture/.codex-plugin/plugin.json"))?;
    assert_eq!(manifest["name"], "loomtracer-capture");
    assert_eq!(manifest["skills"], "./skills/");
    // hivemind-cxqd: the bundle starts its own server config, which names codex. Sharing the
    // Claude plugin's `.mcp.json` filed every Codex MCP capture as agent:claude.
    assert_eq!(manifest["mcpServers"], "./.codex-plugin/mcp.json");
    assert!(manifest["interface"]["capabilities"]
        .as_array()
        .expect("capabilities")
        .iter()
        .any(|capability| capability == "MCP"));
    assert_no_todos("plugin manifest", &manifest.to_string());

    let mcp = read_json(root.join("plugins/loomtracer-capture/.codex-plugin/mcp.json"))?;
    assert_mcp_pins_shared_ledger(&mcp);
    let codex_server = mcp_command_line(&mcp);
    assert!(
        codex_server.contains("--agent-tool codex")
            && !codex_server.contains("--agent-tool claude"),
        "the Codex bundle's MCP server names codex: {codex_server}"
    );
    assert_mcp_probes_project_flag(&codex_server);

    let skill = fs::read_to_string(
        root.join("plugins/loomtracer-capture/skills/loomtracer-capture/SKILL.md"),
    )?;
    assert_no_todos("skill", &skill);
    assert!(skill.contains("decision.capture"));
    // hivemind-cxqd: the skill is shared by Claude Code and Codex, so its direct CLI form names
    // whichever tool is running instead of hard-coding one.
    assert!(skill.contains(r#"--agent-tool "$LOOMTRACER_AGENT_TOOL""#));
    assert!(skill.contains("echo codex || echo claude"));
    assert!(skill.contains("unexpected argument '--project-from-context'"));
    // hivemind-zdsh.9: the skill now documents actor construction generically
    // (agent:<tool>:<name>, a stable identity, not a raw session id) rather
    // than repeating a per-tool "agent:codex:<session>" example.
    assert!(skill.contains("agent:<tool>:<name>"));
    assert!(skill.contains("Automatic Capture Triggers"));
    assert!(skill.contains("CODEX_THREAD_ID"));
    assert!(skill.contains("Capture immediately after the decision is made"));
    assert!(skill.contains("LOOMTRACER_DIR"));
    // hivemind-s15q.15: decision captures work their project out from where the agent runs, and
    // the skill tells the agent what it will see and to relay the reminder.
    assert!(skill.contains("--project-from-context"));
    assert!(skill.contains("Which Project A Capture Lands In"));
    assert!(skill.contains(".loomtracer-project"));
    assert!(skill.contains("saved to your personal project"));
    Ok(())
}

#[test]
fn claude_code_capture_command_writes_human_decision() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let settings = read_json(root.join(".claude/settings.json"))?;
    assert_eq!(settings["env"]["LOOMTRACER_DIR"], "./loomtracer/");
    assert_eq!(
        settings["extraKnownMarketplaces"]["loomtracer"]["source"]["source"],
        "github"
    );
    assert_eq!(
        settings["extraKnownMarketplaces"]["loomtracer"]["source"]["repo"],
        "alexknips/loomtracer"
    );
    assert_eq!(
        settings["enabledPlugins"]["loomtracer-capture@loomtracer"],
        true
    );
    assert!(settings["permissions"]["allow"]
        .as_array()
        .expect("allow list")
        .iter()
        .any(|permission| permission == "Bash(.claude/scripts/capture-decision.sh:*)"));

    let command = fs::read_to_string(root.join(".claude/commands/capture-decision.md"))?;
    assert!(command.contains("/capture-decision"));
    assert!(command.contains("--source human"));
    assert!(command.contains("--source agent"));

    let script = root.join(".claude/scripts/capture-decision.sh");
    let unique = uuid::Uuid::new_v4().to_string();
    let loomtracer_dir = std::env::temp_dir().join(format!("loomtracer-claude-capture-{unique}"));

    let output = Command::new(&script)
        .current_dir(root)
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .args([
            "--source",
            "human",
            "--actor-id",
            "human:test-user",
            "--title",
            "Capture Claude Code slash command decisions",
            "--rationale",
            "Project-local Claude Code commands should write manual decisions with human provenance",
            "--topic-keys",
            "claude,capture",
            "--options",
            "repo-command,manual-shell",
            "--chose",
            "repo-command",
            "--bet",
            "--binds-nothing",
            "--cheap-to-undo",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "capture script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let query = Command::new(env!("CARGO_BIN_EXE_loomtracer"))
        .arg("--loomtracer-dir")
        .arg(&loomtracer_dir)
        .args([
            "query",
            "search_decisions",
            "--q",
            "slash command",
            "--actor-id",
            "human:test-user",
            "--source",
            "human",
            "--limit",
            "5",
        ])
        .output()?;
    assert!(
        query.status.success(),
        "query failed: {}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query: Value = serde_json::from_slice(&query.stdout)?;
    assert_eq!(query["result_count"], 1);
    assert_eq!(
        query["data"]["items"][0]["graph_context"]["actor_ids"][0],
        "human:test-user"
    );

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

#[test]
fn claude_code_plugin_bundle_is_installable_and_wires_cli_mcp() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    let repo_mcp = read_json(root.join(".mcp.json"))?;
    assert_mcp_pins_shared_ledger(&repo_mcp);

    let marketplace = read_json(root.join(".claude-plugin/marketplace.json"))?;
    assert_eq!(marketplace["name"], "loomtracer");
    assert_eq!(marketplace["owner"]["name"], "Loomtracer contributors");

    let plugin = marketplace["plugins"]
        .as_array()
        .and_then(|plugins| {
            plugins
                .iter()
                .find(|plugin| plugin["name"] == "loomtracer-capture")
        })
        .expect("loomtracer-capture appears in Claude marketplace");
    assert_eq!(plugin["source"], "./plugins/loomtracer-capture");
    assert_eq!(plugin["displayName"], "Loomtracer Capture");
    assert!(plugin["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .any(|tag| tag == "mcp"));

    let manifest = read_json(root.join("plugins/loomtracer-capture/.claude-plugin/plugin.json"))?;
    assert_eq!(manifest["name"], "loomtracer-capture");
    assert_eq!(manifest["commands"], "./commands/");
    assert_eq!(manifest["skills"], "./skills/");
    assert_eq!(manifest["mcpServers"], "./.mcp.json");
    assert_eq!(
        manifest["userConfig"]["loomtracer_dir"]["default"],
        "./loomtracer/"
    );
    assert_no_todos("Claude plugin manifest", &manifest.to_string());

    let mcp = read_json(root.join("plugins/loomtracer-capture/.mcp.json"))?;
    let claude_server = mcp_command_line(&mcp);
    assert!(
        claude_server.contains("--agent-tool claude")
            && !claude_server.contains("--agent-tool codex"),
        "the Claude plugin's MCP server names claude: {claude_server}"
    );
    // hivemind-s15q.15: the bundled stdio server works a capture's project out from its own
    // working directory when the call names none, but only on a CLI that has the flag
    // (hivemind-cxqd): the marketplace serves this file from master while people install the
    // latest release binary.
    assert_mcp_probes_project_flag(&claude_server);
    assert_eq!(
        mcp["mcpServers"]["loomtracer"]["env"]["LOOMTRACER_DIR"],
        "./loomtracer/"
    );
    assert_mcp_pins_shared_ledger(&mcp);

    let capture_command =
        fs::read_to_string(root.join("plugins/loomtracer-capture/commands/capture.md"))?;
    assert_no_todos("Claude unified capture command", &capture_command);
    require_contains(
        &capture_command,
        "${CLAUDE_PLUGIN_ROOT}/scripts/capture.sh",
        "unified capture command invokes capture.sh",
    )?;
    require_contains(
        &capture_command,
        "--kind evidence",
        "unified capture command documents evidence kind",
    )?;
    // With --kind omitted capture.sh classifies the text itself (see run_classifier); no
    // loomtracer-classifier subagent ships, so the command must name what runs, not that.
    for needle in [
        "LOOMTRACER_CAPTURE_CLASSIFIER_JSON",
        "LOOMTRACER_CAPTURE_CLASSIFIER_CMD",
        "claude --print",
        "No Loomtracer capture emitted: --kind was omitted",
    ] {
        require_contains(
            &capture_command,
            needle,
            "unified capture command documents the classifier capture.sh runs",
        )?;
    }
    if capture_command.contains("`loomtracer-classifier` subagent if") {
        return Err("unified capture command claims a loomtracer-classifier subagent ships".into());
    }
    // capture.sh refuses blocker and notification (exit 2, nothing recorded), so the
    // hint must not offer a kind that records nothing.
    let argument_hint = capture_command
        .lines()
        .find(|line| line.starts_with("argument-hint:"))
        .ok_or("unified capture command has an argument-hint")?;
    require_contains(
        argument_hint,
        "[--kind decision|evidence|hypothesis]",
        "unified capture command hint offers only capturable kinds",
    )?;
    if argument_hint.contains("blocker") || argument_hint.contains("notification") {
        return Err("unified capture command hint offers a kind capture.sh refuses".into());
    }

    let capture_decision_command =
        fs::read_to_string(root.join("plugins/loomtracer-capture/commands/capture-decision.md"))?;
    assert_no_todos("Claude capture decision command", &capture_decision_command);
    require_contains(
        &capture_decision_command,
        "/loomtracer-capture:capture",
        "decision command points to primary capture command",
    )?;
    require_contains(
        &capture_decision_command,
        "/loomtracer-capture:query-decisions",
        "decision command prints query follow-up",
    )?;
    require_contains(
        &capture_decision_command,
        "agent:claude:<name>",
        "decision command documents Claude actor provenance",
    )?;
    require_contains(
        &capture_decision_command,
        "${CLAUDE_PLUGIN_ROOT}/scripts/capture-decision.sh",
        "decision command invokes compatibility wrapper",
    )?;

    let query_command =
        fs::read_to_string(root.join("plugins/loomtracer-capture/commands/query-decisions.md"))?;
    assert_no_todos("Claude query command", &query_command);
    assert!(query_command.contains("query recall"));
    assert!(query_command.contains("truncated"));

    let readme = fs::read_to_string(root.join("plugins/loomtracer-capture/README.md"))?;
    assert_no_todos("Claude plugin README", &readme);
    assert!(readme.contains("/plugin install loomtracer-capture@loomtracer"));
    assert!(readme.contains("/plugin uninstall loomtracer-capture@loomtracer"));
    require_contains(
        &readme,
        "/loomtracer-capture:capture",
        "README documents primary capture command",
    )?;
    require_contains(
        &readme,
        "/loomtracer-capture:capture-decision",
        "README documents compatibility decision command",
    )?;
    require_contains(
        &readme,
        "install-active-capture-skill.sh",
        "README documents active capture skill installer",
    )?;

    let active_skill = fs::read_to_string(
        root.join("plugins/loomtracer-capture/.claude-plugin/skills/active-capture.md"),
    )?;
    verify_active_capture_skill("Claude active capture skill", &active_skill)?;

    let rig_skill = fs::read_to_string(root.join(".claude/skills/active-capture.md"))?;
    if rig_skill != active_skill {
        return Err("rig-local active capture skill differs from plugin source".into());
    }

    assert_executable(root.join("plugins/loomtracer-capture/scripts/capture.sh"))?;
    assert_executable(root.join("plugins/loomtracer-capture/scripts/capture-decision.sh"))?;
    assert_executable(root.join("plugins/loomtracer-capture/scripts/query-decisions.sh"))?;
    assert_executable(
        root.join("plugins/loomtracer-capture/scripts/install-active-capture-skill.sh"),
    )?;
    Ok(())
}

#[test]
fn claude_active_capture_skill_installer_copies_to_project_skills() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let project_dir = unique_temp_dir("loomtracer-active-capture-project")?;
    let installer = root.join("plugins/loomtracer-capture/scripts/install-active-capture-skill.sh");

    let output = Command::new(&installer)
        .current_dir(root)
        .arg("--project-dir")
        .arg(&project_dir)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "active capture skill installer failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let source = fs::read_to_string(
        root.join("plugins/loomtracer-capture/.claude-plugin/skills/active-capture.md"),
    )?;
    let installed = fs::read_to_string(project_dir.join(".claude/skills/active-capture.md"))?;
    if installed != source {
        return Err("installed active capture skill differs from plugin source".into());
    }
    verify_active_capture_skill("installed active capture skill", &installed)?;

    let _ = fs::remove_dir_all(project_dir);
    Ok(())
}

#[test]
fn claude_code_plugin_capture_and_query_scripts_write_agent_decision() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let unique = uuid::Uuid::new_v4().to_string();
    let loomtracer_dir = std::env::temp_dir().join(format!("loomtracer-claude-plugin-{unique}"));
    let actor_id = "agent:claude:plugin-test-session";

    let capture_script = root.join("plugins/loomtracer-capture/scripts/capture-decision.sh");
    let output = Command::new(&capture_script)
        .current_dir(markerless_cwd())
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("CLAUDE_SESSION_ID", "plugin-test-session")
        // A Gas City-hosted test run (or this very repo's own dev polecats) may have
        // GC_AGENT/GC_ALIAS ambient; this test isolates CLAUDE_SESSION_ID-derived
        // identity specifically, so the stable-identity-first resolution
        // (identity.rs::stable_agent_identity, hivemind-zdsh.9) must not see them.
        .env_remove("GC_AGENT")
        .env_remove("GC_ALIAS")
        .env_remove("GC_RIG")
        .args([
            "--title",
            "Capture Claude plugin decisions",
            "--rationale",
            "The Claude plugin should write agent decisions with session provenance",
            "--topic-keys",
            "claude,plugin,capture",
            "--options",
            "plugin-command,manual-shell",
            "--chose",
            "plugin-command",
            "--bet",
            "--binds-nothing",
            "--cheap-to-undo",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "plugin capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Captured Loomtracer decision decision-"));
    assert!(stdout.contains("/loomtracer-capture:query-decisions"));
    assert!(stdout.contains(actor_id));

    let query_script = root.join("plugins/loomtracer-capture/scripts/query-decisions.sh");
    let query = Command::new(&query_script)
        .current_dir(root)
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("CLAUDE_SESSION_ID", "plugin-test-session")
        .args(["--q", "plugin decisions", "--limit", "5"])
        .output()?;
    assert!(
        query.status.success(),
        "plugin query failed: {}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query: Value = serde_json::from_slice(&query.stdout)?;
    assert_eq!(query["result_count"], 1);
    assert_eq!(
        query["data"]["ranked"]["items"][0]["graph_context"]["actor_ids"][0],
        actor_id
    );

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

#[test]
fn query_decisions_script_finds_decisions_captured_by_a_different_session() -> TestResult<()> {
    // Regression test for hivemind-tenv.4's reopen finding: the query
    // script used to silently inject --actor-id/--source for the
    // QUERYING session's own identity, so "what did we decide about X"
    // from a fresh session found nothing a prior session had captured.
    // Capture and query here run as two distinct sessions/actors on
    // purpose — the bug was invisible when both used the same session
    // (see claude_code_plugin_capture_and_query_scripts_write_agent_decision).
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let unique = uuid::Uuid::new_v4().to_string();
    let loomtracer_dir =
        std::env::temp_dir().join(format!("loomtracer-cross-session-query-{unique}"));
    let capturer_actor_id = "agent:claude:capturer-session";

    let capture_script = root.join("plugins/loomtracer-capture/scripts/capture-decision.sh");
    let output = Command::new(&capture_script)
        .current_dir(markerless_cwd())
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("CLAUDE_SESSION_ID", "capturer-session")
        .env_remove("GC_AGENT")
        .env_remove("GC_ALIAS")
        .env_remove("GC_RIG")
        .args([
            "--title",
            "Recall smoke test fixture across sessions",
            "--rationale",
            "A fresh querying session must find decisions captured by other sessions",
            "--topic-keys",
            "claude,plugin,recall",
            "--options",
            "cross-session,same-session",
            "--chose",
            "cross-session",
            "--bet",
            "--binds-nothing",
            "--cheap-to-undo",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "plugin capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let query_script = root.join("plugins/loomtracer-capture/scripts/query-decisions.sh");
    let query = Command::new(&query_script)
        .current_dir(root)
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("CLAUDE_SESSION_ID", "querier-session")
        .args(["--q", "recall smoke test fixture", "--limit", "5"])
        .output()?;
    assert!(
        query.status.success(),
        "plugin query failed: {}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query: Value = serde_json::from_slice(&query.stdout)?;
    assert_eq!(
        query["result_count"], 1,
        "a query from a different session with no --actor-id/--source must still find the decision"
    );
    assert_eq!(
        query["data"]["ranked"]["items"][0]["graph_context"]["actor_ids"][0],
        capturer_actor_id
    );

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

/// Captures one decision through the plugin's own capture script so the
/// free-text query tests below have exactly one thing to find.
fn capture_joined_query_fixture(root: &Path, loomtracer_dir: &Path) -> TestResult<()> {
    let output = Command::new(root.join("plugins/loomtracer-capture/scripts/capture-decision.sh"))
        .current_dir(markerless_cwd())
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", loomtracer_dir)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("CLAUDE_SESSION_ID", "join-fixture-session")
        .env_remove("GC_AGENT")
        .env_remove("GC_ALIAS")
        .env_remove("GC_RIG")
        .args([
            "--title",
            "Recall smoke test fixture for joined free text",
            "--rationale",
            "A free-text query typed with or without quotes must reach recall as one description",
            "--topic-keys",
            "plugin,recall",
            "--options",
            "join-words,require-quotes",
            "--chose",
            "join-words",
            "--bet",
            "--binds-nothing",
            "--cheap-to-undo",
        ])
        .output()?;
    require(
        output.status.success(),
        format!(
            "plugin capture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

fn require_one_recall_hit(output: &std::process::Output, what: &str) -> TestResult<()> {
    require(
        output.status.success(),
        format!(
            "{what}: query failed ({:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    let result: Value = serde_json::from_slice(&output.stdout)?;
    require_eq(&result["result_count"], &Value::from(1), what)
}

#[test]
fn query_decisions_script_joins_unquoted_free_text_into_one_query() -> TestResult<()> {
    // hivemind-f4ng: the CLI's positional query is ONE shell token, so an unquoted
    // `recall smoke test fixture` used to reach it as four and fail with
    // "unexpected argument 'smoke' found". The script now joins the leading
    // non-flag words itself; a query already quoted into one word is unchanged.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let loomtracer_dir = unique_temp_dir("loomtracer-query-join")?;
    capture_joined_query_fixture(root, &loomtracer_dir)?;

    let query_script = root.join("plugins/loomtracer-capture/scripts/query-decisions.sh");
    let typed_forms: [&[&str]; 4] = [
        &["recall smoke test fixture", "--limit", "5"],
        &["recall", "smoke", "test", "fixture", "--limit", "5"],
        &["recall", "smoke", "test", "fixture"],
        &["--q", "recall smoke test fixture", "--limit", "5"],
    ];
    for args in typed_forms {
        let output = Command::new(&query_script)
            .current_dir(root)
            .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
            .env("LOOMTRACER_DIR", &loomtracer_dir)
            .env("CLAUDE_PROJECT_DIR", root)
            .args(args)
            .output()?;
        require_one_recall_hit(&output, &format!("query-decisions.sh {args:?}"))?;
    }

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

#[test]
fn query_decisions_command_runs_both_typed_forms_of_its_own_argument_hint() -> TestResult<()> {
    // hivemind-f4ng: the command markdown wrapped `$ARGUMENTS` in one more pair of
    // quotes, so the quoted form its argument-hint documents expanded to `""a b""`
    // and split into two words. Run the very line the markdown hands the model,
    // with the typed arguments substituted the way Claude Code does.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let plugin_root = root.join("plugins/loomtracer-capture");
    let command_md = fs::read_to_string(plugin_root.join("commands/query-decisions.md"))?;
    let invocation = command_md
        .lines()
        .find(|line| line.starts_with("${CLAUDE_PLUGIN_ROOT}/scripts/query-decisions.sh"))
        .ok_or("query-decisions.md should show the helper invocation")?;
    require_eq(
        invocation,
        "${CLAUDE_PLUGIN_ROOT}/scripts/query-decisions.sh $ARGUMENTS",
        "the helper takes the typed arguments bare, never wrapped in more quotes",
    )?;

    let loomtracer_dir = unique_temp_dir("loomtracer-query-command")?;
    capture_joined_query_fixture(root, &loomtracer_dir)?;

    for typed in [
        r#""recall smoke test fixture" --limit 5"#,
        "recall smoke test fixture --limit 5",
        r#"--q "recall smoke test fixture" --limit 5"#,
    ] {
        let line = invocation
            .replace("${CLAUDE_PLUGIN_ROOT}", &plugin_root.to_string_lossy())
            .replace("$ARGUMENTS", typed);
        let output = Command::new("bash")
            .args(["-c", &line])
            .current_dir(root)
            .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
            .env("LOOMTRACER_DIR", &loomtracer_dir)
            .env("CLAUDE_PROJECT_DIR", root)
            .output()?;
        require_one_recall_hit(
            &output,
            &format!("/loomtracer-capture:query-decisions {typed}"),
        )?;
    }

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

#[test]
fn context_verbs_print_usage_when_called_with_no_arguments() -> TestResult<()> {
    // macOS bash 3.2 died on the empty-array expansion under `set -u` when these
    // scripts got no arguments at all. No argument is a usage error, answered
    // before the CLI is even resolved: point the binary override at a path that
    // cannot run, so reaching the CLI would show up as exit 127, not usage.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for verb in [
        "recall",
        "why",
        "verify",
        "disagree",
        "supersede",
        "reaffirm",
    ] {
        let output =
            Command::new(root.join(format!("plugins/loomtracer-context/scripts/{verb}.sh")))
                .current_dir(root)
                .env("LOOMTRACER_CAPTURE_BIN", "/nonexistent/loomtracer")
                .output()?;
        require_eq(
            output.status.code(),
            Some(2),
            &format!("{verb}.sh exit code"),
        )?;
        require_contains(
            &String::from_utf8_lossy(&output.stderr),
            "Usage:",
            &format!("{verb}.sh stderr"),
        )?;
    }
    Ok(())
}

#[test]
fn codex_capture_defaults_actor_from_session_environment() -> TestResult<()> {
    let loomtracer_dir = unique_temp_dir("loomtracer-codex-default-capture")?;

    let output = Command::new(env!("CARGO_BIN_EXE_loomtracer"))
        .env_remove("LOOMTRACER_ACTOR")
        .env_remove("LOOMTRACER_AGENT_TOOL")
        .env_remove("LOOMTRACER_AGENT_SESSION")
        .env_remove("LOOMTRACER_CODEX_SESSION")
        .env_remove("LOOMTRACER_CLAUDE_SESSION")
        .env_remove("CLAUDE_SESSION_ID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("GC_AGENT")
        .env_remove("GC_ALIAS")
        .env_remove("GC_RIG")
        .env("CODEX_SESSION_ID", "plugin-test-session")
        .arg("--json")
        .arg("--loomtracer-dir")
        .arg(&loomtracer_dir)
        .args([
            "emit",
            "decision.capture",
            "--binds-nothing",
            "--cheap-to-undo",
            "--bet",
            "--title",
            "Capture Codex plugin decisions without setup",
            "--rationale",
            "The Codex capture path should derive actor provenance from the session",
            "--topic-keys",
            "codex,plugin,capture",
            "--options",
            "default-actor,manual-actor",
            "--chose",
            "default-actor",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "codex capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output: Value = serde_json::from_slice(&output.stdout)?;
    let event = proposal_event(
        &loomtracer_dir,
        output["value"].as_str().expect("decision id"),
    )?;
    assert_eq!(event.actor_id, "agent:codex:plugin-test-session");
    assert_eq!(event.source.as_str(), "agent");
    // source_ref carries the raw session id (provenance), not a repeat of actor_id --
    // ambient derivation with no --agent-session/--actor-id pin (hivemind-zdsh.9).
    assert_eq!(event.source_ref.as_deref(), Some("plugin-test-session"));

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

#[test]
fn capture_plugin_scripts_derive_codex_session_context() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let unique = uuid::Uuid::new_v4().to_string();
    let loomtracer_dir = std::env::temp_dir().join(format!("loomtracer-codex-plugin-{unique}"));
    let actor_id = "agent:codex:codex-thread-test";

    let capture_script = root.join("plugins/loomtracer-capture/scripts/capture-decision.sh");
    let output = Command::new(&capture_script)
        .current_dir(markerless_cwd())
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .env("CODEX_THREAD_ID", "codex-thread-test")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("CLAUDE_SESSION_ID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("GC_AGENT")
        .env_remove("GC_ALIAS")
        .env_remove("GC_RIG")
        .args([
            "--title",
            "Derive Codex plugin session defaults",
            "--rationale",
            "The Codex path should capture decisions without manual actor or source setup",
            "--topic-keys",
            "codex,plugin,capture",
            "--options",
            "manual-provenance,session-context",
            "--chose",
            "session-context",
            "--bet",
            "--binds-nothing",
            "--cheap-to-undo",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "codex plugin capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Captured Loomtracer decision decision-"));
    assert!(stdout.contains(actor_id));

    let query_script = root.join("plugins/loomtracer-capture/scripts/query-decisions.sh");
    let query = Command::new(&query_script)
        .current_dir(root)
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .env("CODEX_THREAD_ID", "codex-thread-test")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("CLAUDE_SESSION_ID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .args(["--q", "session defaults", "--limit", "5"])
        .output()?;
    assert!(
        query.status.success(),
        "codex plugin query failed: {}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query: Value = serde_json::from_slice(&query.stdout)?;
    assert_eq!(query["result_count"], 1);
    assert_eq!(
        query["data"]["ranked"]["items"][0]["graph_context"]["actor_ids"][0],
        actor_id
    );

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

#[test]
fn capture_plugin_defaults_to_rig_ledger_from_linked_worktree() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = unique_temp_dir("loomtracer-plugin-linked-worktree")?;
    let rig = scratch.join("rig");
    let worktree = scratch.join("worktree");
    fs::create_dir_all(&rig)?;
    fs::write(rig.join("README.md"), "fixture repository\n")?;
    run_git(&rig, ["init"])?;
    run_git(&rig, ["add", "README.md"])?;
    run_git(
        &rig,
        [
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test User",
            "commit",
            "-m",
            "init",
        ],
    )?;
    let worktree_arg = worktree.to_str().ok_or("worktree path should be UTF-8")?;
    run_git(&rig, ["worktree", "add", "--detach", worktree_arg])?;

    let capture_script = root.join("plugins/loomtracer-capture/scripts/capture-decision.sh");
    let output = Command::new(&capture_script)
        .current_dir(&worktree)
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("CODEX_THREAD_ID", "linked-worktree-test")
        .env_remove("LOOMTRACER_DIR")
        .env_remove("CLAUDE_PLUGIN_OPTION_LOOMTRACER_DIR")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("CLAUDE_SESSION_ID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("GC_AGENT")
        .env_remove("GC_ALIAS")
        .env_remove("GC_RIG")
        .args([
            "--title",
            "Capture from linked worktree to rig ledger",
            "--rationale",
            "The plugin default should use the original repository root, not the transient worktree root",
            "--topic-keys",
            "codex,plugin,worktree",
            "--options",
            "rig-ledger,worktree-ledger",
            "--chose",
            "rig-ledger",
            "--bet",
            "--binds-nothing",
            "--cheap-to-undo",
        ])
        .output()?;
    require(
        output.status.success(),
        format!(
            "linked worktree capture failed: {}\nstdout: {}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ),
    )?;

    let rig_loomtracer_dir = rig.join("loomtracer");
    let worktree_loomtracer_dir = worktree.join("loomtracer");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let rig_loomtracer_dir_text = rig_loomtracer_dir.to_string_lossy();
    let rig_text = rig.to_string_lossy();
    let worktree_text = worktree.to_string_lossy();
    require_contains(&stderr, "loomtracer-dir resolved to", "capture stderr")?;
    require_contains(&stderr, rig_loomtracer_dir_text.as_ref(), "capture stderr")?;
    require_contains(&stderr, rig_text.as_ref(), "capture stderr")?;
    require_contains(&stderr, worktree_text.as_ref(), "capture stderr")?;
    require(
        !worktree_loomtracer_dir.join("ledger.sqlite").exists(),
        format!(
            "capture should not create a worktree-local ledger at {}",
            worktree_loomtracer_dir.display()
        ),
    )?;

    let ledger = SqliteEventLedger::open(&rig_loomtracer_dir)?;
    let events = ledger.read(0, 100)?;
    let captured_event = events
        .iter()
        .find(|event| {
            event.event_type == loomtracer::events::EventType::DecisionProposed
                && event.payload.get("title").and_then(Value::as_str)
                    == Some("Capture from linked worktree to rig ledger")
        })
        .ok_or("rig ledger should contain the linked-worktree capture event")?;
    require_eq(
        captured_event.actor_id.as_str(),
        "agent:codex:linked-worktree-test",
        "rig ledger actor",
    )?;

    let query_script = root.join("plugins/loomtracer-capture/scripts/query-decisions.sh");
    let query = Command::new(&query_script)
        .current_dir(&worktree)
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("CODEX_THREAD_ID", "linked-worktree-test")
        .env_remove("LOOMTRACER_DIR")
        .env_remove("CLAUDE_PLUGIN_OPTION_LOOMTRACER_DIR")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("CLAUDE_SESSION_ID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .args(["--q", "linked worktree", "--limit", "5"])
        .output()?;
    require(
        query.status.success(),
        format!(
            "linked worktree query failed: {}\nstdout: {}",
            String::from_utf8_lossy(&query.stderr),
            String::from_utf8_lossy(&query.stdout)
        ),
    )?;
    let query_stderr = String::from_utf8_lossy(&query.stderr);
    require_contains(&query_stderr, "loomtracer-dir resolved to", "query stderr")?;
    require_contains(
        &query_stderr,
        rig_loomtracer_dir_text.as_ref(),
        "query stderr",
    )?;
    let query: Value = serde_json::from_slice(&query.stdout)?;
    let result_count = query
        .get("result_count")
        .and_then(Value::as_u64)
        .ok_or("query result_count should be an integer")?;
    require(
        result_count == 1,
        format!("query result_count should be 1, got {result_count}"),
    )?;
    let query_actor = query
        .get("data")
        .and_then(|data| data.get("ranked"))
        .and_then(|ranked| ranked.get("items"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("graph_context"))
        .and_then(|graph_context| graph_context.get("actor_ids"))
        .and_then(Value::as_array)
        .and_then(|actor_ids| actor_ids.first())
        .and_then(Value::as_str)
        .ok_or("query actor id should exist")?;
    require_eq(
        query_actor,
        "agent:codex:linked-worktree-test",
        "query actor",
    )?;

    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

#[test]
fn unified_capture_script_records_evidence_with_agent_provenance() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let unique = uuid::Uuid::new_v4().to_string();
    let loomtracer_dir = std::env::temp_dir().join(format!("loomtracer-evidence-capture-{unique}"));
    let actor_id = "agent:claude:evidence-test-session";

    let capture_script = root.join("plugins/loomtracer-capture/scripts/capture.sh");
    let output = Command::new(&capture_script)
        .current_dir(root)
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("CLAUDE_SESSION_ID", "evidence-test-session")
        .env_remove("GC_AGENT")
        .env_remove("GC_ALIAS")
        .env_remove("GC_RIG")
        .args([
            "The plugin smoke test wrote a decision and queried it back",
            "--kind",
            "evidence",
        ])
        .output()?;
    require(
        output.status.success(),
        format!(
            "unified evidence capture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    require_contains(
        stdout.as_ref(),
        "Captured Loomtracer evidence evidence-",
        "unified evidence capture prints evidence id",
    )?;
    require_contains(
        stdout.as_ref(),
        actor_id,
        "unified evidence capture prints actor id",
    )?;

    let event = event_with_type(
        &loomtracer_dir,
        loomtracer::events::EventType::EvidenceRecorded,
    )?;
    require_eq(event.actor_id.as_str(), actor_id, "evidence actor id")?;
    require_eq(event.source.as_str(), "agent", "evidence source")?;
    // source_ref carries the raw session id (provenance), not a repeat of actor_id --
    // ambient derivation with no explicit session/actor pin (hivemind-zdsh.9).
    require_eq(
        event.source_ref.as_deref(),
        Some("evidence-test-session"),
        "evidence source_ref",
    )?;
    require_eq(
        event.payload.get("content").and_then(Value::as_str),
        Some("The plugin smoke test wrote a decision and queried it back"),
        "evidence content",
    )?;

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

#[test]
fn unified_capture_script_uses_classifier_when_kind_is_omitted() -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let unique = uuid::Uuid::new_v4().to_string();
    let loomtracer_dir =
        std::env::temp_dir().join(format!("loomtracer-classified-capture-{unique}"));

    let capture_script = root.join("plugins/loomtracer-capture/scripts/capture.sh");
    let output = Command::new(&capture_script)
        .current_dir(root)
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .env("CODEX_THREAD_ID", "classifier-thread-test")
        .env(
            "LOOMTRACER_CAPTURE_CLASSIFIER_JSON",
            r#"{"kind":"evidence"}"#,
        )
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("CLAUDE_SESSION_ID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("GC_AGENT")
        .env_remove("GC_ALIAS")
        .env_remove("GC_RIG")
        .arg("The test failed with error E")
        .output()?;
    require(
        output.status.success(),
        format!(
            "classified capture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    require_contains(
        stdout.as_ref(),
        "Captured Loomtracer evidence evidence-",
        "classified capture prints evidence id",
    )?;

    let event = event_with_type(
        &loomtracer_dir,
        loomtracer::events::EventType::EvidenceRecorded,
    )?;
    require_eq(
        event.actor_id.as_str(),
        "agent:codex:classifier-thread-test",
        "classified actor id",
    )?;
    require_eq(event.source.as_str(), "agent", "classified source")?;
    require_eq(
        event.payload.get("content").and_then(Value::as_str),
        Some("The test failed with error E"),
        "classified content",
    )?;

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

/// `capture.sh` with the agent/classifier environment cleared, writing to `loomtracer_dir`.
fn capture_script_command(loomtracer_dir: &Path) -> Command {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut command = Command::new(root.join("plugins/loomtracer-capture/scripts/capture.sh"));
    command
        .current_dir(root)
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", loomtracer_dir)
        .env("CODEX_THREAD_ID", "classifier-refusal-test")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .env_remove("LOOMTRACER_CAPTURE_CLASSIFIER_JSON")
        .env_remove("LOOMTRACER_CAPTURE_CLASSIFIER_CMD");
    without_agent_env(&mut command);
    command
}

fn require_empty_ledger(loomtracer_dir: &Path, message: &str) -> TestResult<()> {
    let events = SqliteEventLedger::open(loomtracer_dir)?.read(0, 100)?;
    require(events.is_empty(), format!("{message}: {events:?}"))
}

#[test]
fn capture_script_without_a_kind_records_nothing_when_the_classifier_picks_one_it_cannot_record(
) -> TestResult<()> {
    // hivemind-uwslm: the classifier was offered blocker and notification, and capture.sh then
    // refused its own classifier's answer (exit 2, "known but does not yet have a canonical CLI
    // capture path") for a kind the caller never chose. A kind capture.sh cannot record, from
    // whatever the classifier answers, now ends as "no capture emitted" (exit 0, nothing written).
    for kind in ["blocker", "notification", "request"] {
        let scratch = unique_temp_dir("loomtracer-classifier-unrecordable")?;
        let loomtracer_dir = scratch.join("ledger");
        let output = capture_script_command(&loomtracer_dir)
            .env(
                "LOOMTRACER_CAPTURE_CLASSIFIER_JSON",
                format!(r#"{{"kind":"{kind}"}}"#),
            )
            .arg("Merge is blocked until the schema review lands")
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        require(
            output.status.success(),
            format!("classifier kind {kind} should not fail the capture: {stdout} {stderr}"),
        )?;
        require_contains(
            stdout.as_ref(),
            &format!("No Loomtracer capture emitted: classifier returned kind={kind}"),
            "the line says what the classifier answered",
        )?;
        require(
            !stderr.contains("known but does not yet"),
            format!("classifier kind {kind} hit the explicit-kind refusal: {stderr}"),
        )?;
        require_empty_ledger(
            &loomtracer_dir,
            "an unrecordable classifier kind wrote events",
        )?;
        let _ = fs::remove_dir_all(scratch);
    }
    Ok(())
}

#[test]
fn capture_script_offers_its_classifier_only_the_kinds_it_can_record() -> TestResult<()> {
    // hivemind-uwslm: the schema and the prompt handed to the classifier name decision,
    // evidence, hypothesis and none, and no kind capture.sh refuses. A stand-in `claude` on PATH
    // records its arguments and answers blocker, so the reply path runs end to end as well.
    let scratch = unique_temp_dir("loomtracer-classifier-offered-kinds")?;
    let loomtracer_dir = scratch.join("ledger");
    let bin_dir = scratch.join("bin");
    let args_file = scratch.join("claude-args");
    fs::create_dir_all(&bin_dir)?;
    let stub = bin_dir.join("claude");
    fs::write(
        &stub,
        "#!/usr/bin/env bash\nprintf '%s\\0' \"$@\" > \"$STUB_ARGS_FILE\"\nprintf '%s\\n' \"$STUB_REPLY\"\n",
    )?;
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755))?;

    let output = capture_script_command(&loomtracer_dir)
        .env(
            "PATH",
            format!("{}:{}", bin_dir.display(), std::env::var("PATH")?),
        )
        .env("STUB_ARGS_FILE", &args_file)
        .env("STUB_REPLY", r#"{"kind":"blocker"}"#)
        .arg("Merge is blocked until the schema review lands")
        .output()?;
    require(
        output.status.success(),
        format!(
            "a blocker reply should not fail the capture: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    require_contains(
        String::from_utf8_lossy(&output.stdout).as_ref(),
        "No Loomtracer capture emitted: classifier returned kind=blocker",
        "the stand-in classifier's blocker reply",
    )?;

    let args_raw = fs::read_to_string(&args_file)?;
    let args: Vec<&str> = args_raw.split('\0').collect();
    let schema_at = args
        .iter()
        .position(|arg| *arg == "--json-schema")
        .ok_or("the classifier is called with --json-schema")?;
    let schema: Value = serde_json::from_str(args[schema_at + 1])?;
    let offered: Vec<&str> = schema["properties"]["kind"]["enum"]
        .as_array()
        .ok_or("the schema lists the kinds as an enum")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    require_eq(
        offered,
        vec!["decision", "evidence", "hypothesis", "none"],
        "kinds in the classifier schema",
    )?;
    let prompt = args
        .iter()
        .find(|arg| arg.starts_with("Classify this Loomtracer capture text"))
        .ok_or("the classifier is handed the capture prompt")?
        .to_lowercase();
    for refused in ["blocker", "notification"] {
        require(
            !prompt.contains(refused),
            format!("the classifier prompt offers {refused}, which capture.sh cannot record"),
        )?;
    }
    require_empty_ledger(&loomtracer_dir, "a blocker reply wrote events")?;
    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

#[test]
fn capture_script_explicit_blocker_and_notification_kinds_still_refuse() -> TestResult<()> {
    // hivemind-uwslm leaves what hivemind-udvza taught alone: a caller who names the kind is told
    // it has no capture path (exit 2), nothing recorded. Only the classifier's pick is softened.
    for kind in ["blocker", "notification"] {
        let scratch = unique_temp_dir("loomtracer-explicit-unrecordable-kind")?;
        let loomtracer_dir = scratch.join("ledger");
        let output = capture_script_command(&loomtracer_dir)
            .args([
                "--kind",
                kind,
                "Merge is blocked until the schema review lands",
            ])
            .output()?;
        require_eq(output.status.code(), Some(2), "explicit kind exit code")?;
        require_contains(
            String::from_utf8_lossy(&output.stderr).as_ref(),
            &format!("kind {kind} is known but does not yet have a canonical CLI capture path"),
            "explicit kind refusal",
        )?;
        require_empty_ledger(&loomtracer_dir, "an explicit refused kind wrote events")?;
        let _ = fs::remove_dir_all(scratch);
    }
    Ok(())
}

#[test]
fn human_cli_emit_defaults_actor_and_source_from_git_email() -> TestResult<()> {
    let scratch = unique_temp_dir("loomtracer-human-cli-default")?;
    let repo = scratch.join("repo");
    let loomtracer_dir = scratch.join("ledger");
    fs::create_dir_all(&repo)?;
    run_git(&repo, ["init"])?;
    run_git(
        &repo,
        ["config", "user.email", "Ada.Example+Decisions@Example.COM"],
    )?;

    let output =
        without_agent_env(Command::new(env!("CARGO_BIN_EXE_loomtracer")).current_dir(&repo))
            .arg("--json")
            .arg("--loomtracer-dir")
            .arg(&loomtracer_dir)
            .args([
                "emit",
                "decision.proposed",
                "--title",
                "Use git identity for human CLI writes",
                "--rationale",
                "Bare terminal writes should still carry human provenance",
                "--topic-keys",
                "cli,provenance",
                "--options",
                "git-email,manual-actor",
                "--chose",
                "git-email",
            ])
            .output()?;
    assert!(
        output.status.success(),
        "human CLI emit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output: Value = serde_json::from_slice(&output.stdout)?;
    let event = proposal_event(
        &loomtracer_dir,
        output["value"].as_str().expect("decision id"),
    )?;
    assert_eq!(event.actor_id, "human:ada.example-decisions@example.com");
    assert_eq!(event.source.as_str(), "human");
    assert_eq!(
        event.source_ref.as_deref(),
        Some("human:ada.example-decisions@example.com")
    );

    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

/// The server's command and arguments as one line: a plain `loomtracer ... mcp` entry or a
/// `sh -c "<script>"` entry that starts it, whichever the config uses.
fn mcp_command_line(mcp: &Value) -> String {
    let server = &mcp["mcpServers"]["loomtracer"];
    let command = server["command"].as_str().expect("mcp command");
    let args = server["args"].as_array().expect("mcp args");
    std::iter::once(command)
        .chain(args.iter().map(|arg| arg.as_str().expect("mcp arg")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn assert_mcp_pins_shared_ledger(mcp: &Value) {
    let server = &mcp["mcpServers"]["loomtracer"];
    assert_eq!(server["env"]["LOOMTRACER_DIR"], "./loomtracer/");

    let line = mcp_command_line(mcp);
    assert!(
        line.contains("--loomtracer-dir ./loomtracer/"),
        "mcp command should pin --loomtracer-dir ./loomtracer/: {line}"
    );
    assert!(
        line.split_whitespace().any(|word| word == "mcp"),
        "mcp command should run the mcp subcommand: {line}"
    );
}

/// A plugin config the marketplace serves from master must not hand the release binary a flag it
/// lacks: `--project-from-context` goes on the server only when `mcp --help` lists it.
fn assert_mcp_probes_project_flag(line: &str) {
    assert!(
        line.contains("loomtracer mcp --help")
            && line.contains("grep -q -e --project-from-context"),
        "the MCP server asks the CLI whether it has --project-from-context: {line}"
    );
    assert!(
        line.contains("exec loomtracer --loomtracer-dir ./loomtracer/ mcp")
            && line.ends_with("$flag"),
        "the flag reaches the server only through the probe: {line}"
    );
}

fn read_json(path: impl AsRef<Path>) -> TestResult<Value> {
    let path = path.as_ref();
    let input = fs::read_to_string(path).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("{} is readable: {error}", path.display()),
        )
    })?;
    let value = serde_json::from_str(&input).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{} is valid json: {error}", path.display()),
        )
    })?;
    Ok(value)
}

#[test]
fn capture_script_forwards_grounding_flags_and_refuses_an_ungrounded_capture() -> TestResult<()> {
    // hivemind-gwhr.2: every decision capture must say what it rests on. The plugin script has to
    // carry the value-taking grounding flags (and `--bet`'s optional statement) through to
    // `emit decision.capture`, and a capture that names nothing must fail loudly (exit 2) with
    // nothing written.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let loomtracer_dir = unique_temp_dir("loomtracer-capture-script-grounding")?;
    let script = root.join("plugins/loomtracer-capture/scripts/capture-decision.sh");
    let base = [
        "--title",
        "Adopt the grounded capture script",
        "--rationale",
        "The plugin script should forward every grounding flag to the CLI",
        "--topic-keys",
        "capture,grounding",
        "--options",
        "forward,drop",
        "--chose",
        "forward",
    ];
    let run_script = |extra: &[&str]| -> TestResult<std::process::Output> {
        Ok(Command::new(&script)
            .current_dir(markerless_cwd())
            .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
            .env("LOOMTRACER_DIR", &loomtracer_dir)
            .env("CLAUDE_PROJECT_DIR", root)
            .env("CLAUDE_SESSION_ID", "grounding-script-session")
            .env_remove("GC_AGENT")
            .env_remove("GC_ALIAS")
            .env_remove("GC_RIG")
            .args(base)
            .args(extra)
            .output()?)
    };

    let refused = run_script(&[])?;
    assert_eq!(
        refused.status.code(),
        Some(2),
        "an ungrounded capture must exit 2: {}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(String::from_utf8_lossy(&refused.stderr).contains("must say what it rests on"));
    assert_eq!(
        SqliteEventLedger::open(&loomtracer_dir)?
            .read(0, 100)?
            .len(),
        0,
        "a refused capture writes nothing"
    );

    let captured = run_script(&[
        "--rests-on-evidence",
        "the forwarded flags reached the ledger",
        "--evidence-source",
        "ci run 9",
        "--rests-on-assumption",
        "The script keeps forwarding unknown flags",
        "--binds-nothing",
        "--cheap-to-undo",
        "--bet",
        "The CLI keeps its grounding flag names",
        "--would-change-if",
        "the CLI renames a flag",
        "--check-by",
        "2026-12-01",
        "--confidence",
        "high",
    ])?;
    assert!(
        captured.status.success(),
        "grounded plugin capture failed: {}",
        String::from_utf8_lossy(&captured.stderr)
    );
    let stdout = String::from_utf8_lossy(&captured.stdout);
    assert!(
        stdout.contains("Captured Loomtracer decision decision-"),
        "{stdout}"
    );

    let evidence = event_with_type(
        &loomtracer_dir,
        loomtracer::events::EventType::EvidenceRecorded,
    )?;
    assert_eq!(evidence.payload["source"], "ci run 9");
    let bet = SqliteEventLedger::open(&loomtracer_dir)?
        .read(0, 100)?
        .into_iter()
        .find(|event| {
            event.event_type == loomtracer::events::EventType::HypothesisRecorded
                && event.payload.get("kind").and_then(Value::as_str) == Some("bet")
        })
        .ok_or("the bet should have been recorded")?;
    assert_eq!(
        bet.payload["statement"],
        "The CLI keeps its grounding flag names"
    );
    assert_eq!(bet.payload["would_change_if"], "the CLI renames a flag");
    let proposal = event_with_type(
        &loomtracer_dir,
        loomtracer::events::EventType::DecisionProposed,
    )?;
    assert_eq!(proposal.payload["expressed_confidence"], "high");

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

#[test]
fn capture_script_answers_a_question_that_was_asked_and_the_request_stops_waiting() -> TestResult<()>
{
    // hivemind-ig9i: `--answers <request id>` takes a value. The script once let it fall through
    // to its generic unknown-flag branch, which forwarded the flag alone and glued the request id
    // onto the capture text: the CLI refused ("a value is required"), nothing was written and the
    // request stayed waiting. The plugin has to answer an ask the way the CLI does.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let loomtracer_dir = unique_temp_dir("loomtracer-capture-script-answers")?;
    let script = root.join("plugins/loomtracer-capture/scripts/capture.sh");
    let cli = |args: &[&str]| -> TestResult<Value> {
        let output = Command::new(env!("CARGO_BIN_EXE_loomtracer"))
            .args(["--json", "--actor", "human:checker", "--loomtracer-dir"])
            .arg(&loomtracer_dir)
            .args(args)
            .output()?;
        require(
            output.status.success(),
            format!(
                "loomtracer {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )?;
        Ok(serde_json::from_slice(&output.stdout)?)
    };

    let asked = cli(&["ask", "Which currency do invoices use?"])?;
    let request_id = asked["request_id"]
        .as_str()
        .ok_or("ask returns the request id")?
        .to_owned();
    let waiting = cli(&["query", "get_waiting_requests"])?;
    require_eq(
        waiting["data"]["items"].as_array().map(Vec::len),
        Some(1),
        "the ask is waiting before anything answers it",
    )?;

    let captured = Command::new(&script)
        .current_dir(markerless_cwd())
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("CLAUDE_SESSION_ID", "answers-script-session")
        .env_remove("GC_AGENT")
        .env_remove("GC_ALIAS")
        .env_remove("GC_RIG")
        .args([
            "Invoices are issued in euro",
            "--kind",
            "decision",
            "--title",
            "Invoices are issued in euro",
            "--rationale",
            "Customers are in the euro area, so one currency keeps invoicing simple",
            "--topic-keys",
            "billing",
            "--options",
            "Euro,Multi-currency",
            "--chose",
            "Euro",
            "--answers",
            &request_id,
            "--rests-on-assumption",
            "Customers are in the euro area",
            "--binds-nothing",
            "--cheap-to-undo",
        ])
        .output()?;
    require(
        captured.status.success(),
        format!(
            "answering a request through the plugin failed: {}",
            String::from_utf8_lossy(&captured.stderr)
        ),
    )?;

    let waiting = cli(&["query", "get_waiting_requests"])?;
    require_eq(
        waiting["data"]["items"].as_array().map(Vec::len),
        Some(0),
        "the answered request leaves the waiting list",
    )?;
    let proposal = event_with_type(
        &loomtracer_dir,
        loomtracer::events::EventType::DecisionProposed,
    )?;
    let decision_id = proposal.payload["decision_id"]
        .as_str()
        .ok_or("the proposal names its decision")?;
    require(
        !proposal.payload.to_string().contains(&request_id),
        "the request id is a link, not part of the decision's text",
    )?;
    let why = cli(&["query", "why", "--id", decision_id])?;
    require(
        why["data"]["root"]["asked_at"].is_string(),
        format!("why shows when the answered question was asked: {why}"),
    )?;

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

#[test]
fn capture_script_keeps_an_option_label_with_a_comma_whole() -> TestResult<()> {
    // hivemind-ndtp: `--options` splits on every comma, and the CLI's refusal for a space after a
    // comma points at `--option`. The script once had no case for it: the flag fell through to the
    // generic unknown-flag branch, which forwarded it alone and glued its value onto the capture
    // text, so through the plugin a label with a comma could not be recorded at all.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let loomtracer_dir = unique_temp_dir("loomtracer-capture-script-option")?;
    let script = root.join("plugins/loomtracer-capture/scripts/capture.sh");
    let label = "Rename after the comparison, before the first listing";

    let captured = Command::new(&script)
        .current_dir(markerless_cwd())
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", &loomtracer_dir)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("CLAUDE_SESSION_ID", "option-script-session")
        .env_remove("GC_AGENT")
        .env_remove("GC_ALIAS")
        .env_remove("GC_RIG")
        .args([
            "Rename timing for the plugin path",
            "--kind",
            "decision",
            "--title",
            "Rename timing for the plugin path",
            "--rationale",
            "One label holds a comma, so it has to travel on its own flag",
            "--topic-keys",
            "naming",
            "--options",
            "Rename now,Pause",
            "--option",
            label,
            "--chose",
            label,
            "--rests-on-assumption",
            "Option labels are phrases and phrases hold commas",
            "--binds-nothing",
            "--cheap-to-undo",
        ])
        .output()?;
    require(
        captured.status.success(),
        format!(
            "capturing a label with a comma through the plugin failed: {}",
            String::from_utf8_lossy(&captured.stderr)
        ),
    )?;

    let proposal = event_with_type(
        &loomtracer_dir,
        loomtracer::events::EventType::DecisionProposed,
    )?;
    let strings = |field: &str| -> TestResult<Vec<String>> {
        Ok(proposal.payload[field]
            .as_array()
            .ok_or(format!("the proposal lists its {field}"))?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect())
    };
    // `--option` labels come after the `--options` ones, and the comma label stays one option.
    require_eq(
        strings("option_labels")?,
        vec![
            "Rename now".to_owned(),
            "Pause".to_owned(),
            label.to_owned(),
        ],
        "the labels as recorded",
    )?;
    require_eq(
        proposal.payload["chosen_option_id"].as_str(),
        strings("option_ids")?.get(2).map(String::as_str),
        "--chose picked the label that holds the comma",
    )?;

    let _ = fs::remove_dir_all(loomtracer_dir);
    Ok(())
}

#[cfg(unix)]
#[test]
fn capture_script_refuses_option_on_a_cli_without_it_and_writes_nothing() -> TestResult<()> {
    // hivemind-ndtp: the v0.7.0 release binary has no `--option`, and its own answer ("a similar
    // argument exists: --options") would send the caller to the flag that splits the label. The
    // script says what is true, exits 2 and never reaches a write. A stand-in binary whose
    // `--help` lists only `--options` plays the release.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = unique_temp_dir("loomtracer-capture-script-old-cli")?;
    let wrote = scratch.join("wrote");
    let old_cli = scratch.join("old-loomtracer");
    fs::write(
        &old_cli,
        format!(
            "#!/usr/bin/env bash\n\
             if [[ \" $* \" == *\" --help \"* ]]; then\n\
             echo '      --options <LABELS>  Comma-separated labels'\n\
             exit 0\n\
             fi\n\
             touch '{}'\n",
            wrote.display()
        ),
    )?;
    fs::set_permissions(&old_cli, fs::Permissions::from_mode(0o755))?;
    let script = root.join("plugins/loomtracer-capture/scripts/capture.sh");
    let run = |option_flags: &[&str]| -> TestResult<std::process::Output> {
        Ok(Command::new(&script)
            .current_dir(markerless_cwd())
            .env("LOOMTRACER_CAPTURE_BIN", &old_cli)
            .env("LOOMTRACER_DIR", scratch.join("ledger"))
            .env("CLAUDE_PROJECT_DIR", root)
            .env("CLAUDE_SESSION_ID", "old-cli-session")
            .env_remove("GC_AGENT")
            .env_remove("GC_ALIAS")
            .env_remove("GC_RIG")
            .args([
                "Rename timing",
                "--kind",
                "decision",
                "--title",
                "Rename timing",
                "--rationale",
                "Why",
                "--topic-keys",
                "naming",
                "--rests-on-assumption",
                "Because",
                "--binds-nothing",
                "--cheap-to-undo",
            ])
            .args(option_flags)
            .output()?)
    };

    let refused = run(&[
        "--option",
        "Rename later, after the comparison",
        "--option",
        "Pause",
    ])?;
    require_eq(refused.status.code(), Some(2), "exit code of the refusal")?;
    let stderr = String::from_utf8_lossy(&refused.stderr);
    require_contains(&stderr, "no `--option`", "the refusal says what is missing")?;
    require_contains(
        &stderr,
        "nothing was written",
        "the refusal says nothing was written",
    )?;
    require(!wrote.exists(), "the old CLI was never asked to write")?;

    // Plain `--options` is unaffected: the old CLI is asked to write.
    let plain = run(&["--options", "Rename now,Pause"])?;
    require(
        plain.status.success(),
        format!(
            "--options on a CLI without --option failed: {}",
            String::from_utf8_lossy(&plain.stderr)
        ),
    )?;
    require(wrote.exists(), "--options reaches the old CLI's write")?;

    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

#[cfg(unix)]
#[test]
fn capture_script_leaves_the_answers_out_on_a_cli_without_them_and_says_so() -> TestResult<()> {
    // hivemind-bbnw.12: the v0.7.0 release binary has no `--binds` / `--binds-nothing` /
    // `--undo-cost` / `--cheap-to-undo` and refuses the whole capture over the first one
    // ("unexpected argument '--binds'"), while the marketplace serves this plugin from master. The
    // script sends the capture without the four answers and says so on stderr; a CLI that lists
    // them gets them as given. Stand-ins whose `--help` lists the flags (or not) record their argv.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = unique_temp_dir("loomtracer-capture-script-answers-old-cli")?;
    let argv_file = scratch.join("argv");
    let stand_in = |name: &str, help: &str| -> TestResult<std::path::PathBuf> {
        let path = scratch.join(name);
        fs::write(
            &path,
            format!(
                "#!/usr/bin/env bash\n\
                 if [[ \" $* \" == *\" --help \"* ]]; then\n\
                 echo '{help}'\n\
                 exit 0\n\
                 fi\n\
                 printf '%s\\n' \"$@\" > '{}'\n",
                argv_file.display()
            ),
        )?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        Ok(path)
    };
    let old_cli = stand_in(
        "old-loomtracer",
        "      --options <LABELS>  Comma-separated labels",
    )?;
    let new_cli = stand_in(
        "new-loomtracer",
        "      --binds <WHAT>  What it binds\n      --undo-cost <WHAT>  What undoing it costs",
    )?;
    let script = root.join("plugins/loomtracer-capture/scripts/capture.sh");
    let run = |cli: &Path, answers: &[&str]| -> TestResult<(std::process::Output, Vec<String>)> {
        let _ = fs::remove_file(&argv_file);
        let output = Command::new(&script)
            .current_dir(markerless_cwd())
            .env("LOOMTRACER_CAPTURE_BIN", cli)
            .env("LOOMTRACER_DIR", scratch.join("ledger"))
            .env("CLAUDE_PROJECT_DIR", root)
            .env("CLAUDE_SESSION_ID", "answers-old-cli-session")
            .env_remove("GC_AGENT")
            .env_remove("GC_ALIAS")
            .env_remove("GC_RIG")
            .args([
                "Rename timing",
                "--kind",
                "decision",
                "--title",
                "Rename timing",
                "--rationale",
                "Why",
                "--topic-keys",
                "naming",
                "--options",
                "Rename now,Pause",
                "--rests-on-assumption",
                "Because",
            ])
            .args(answers)
            .output()?;
        let argv = fs::read_to_string(&argv_file)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect();
        Ok((output, argv))
    };
    let answers: [&[&str]; 2] = [
        &["--binds", "Users call the new name", "--cheap-to-undo"],
        &["--binds-nothing", "--undo-cost", "A second release"],
    ];

    for given in answers {
        let (output, argv) = run(&old_cli, given)?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        require(
            output.status.success(),
            format!("a capture with answers on an old CLI failed: {stderr}"),
        )?;
        require(
            argv.iter().any(|a| a == "Rename timing"),
            format!("the old CLI was asked to write the capture: {argv:?}"),
        )?;
        for sent in &argv {
            require(
                ![
                    "--binds",
                    "--binds-nothing",
                    "--undo-cost",
                    "--cheap-to-undo",
                ]
                .contains(&sent.as_str())
                    && !["Users call the new name", "A second release"].contains(&sent.as_str()),
                format!("an answer reached the old CLI: {argv:?}"),
            )?;
        }
        require_contains(
            &stderr,
            "left out",
            "the helper says the answers were left out",
        )?;
    }

    for given in answers {
        let (output, argv) = run(&new_cli, given)?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        require(output.status.success(), format!("capture failed: {stderr}"))?;
        for answer in given {
            require(
                argv.iter().any(|a| a == *answer),
                format!("{answer} reaches a CLI that lists it: {argv:?}"),
            )?;
        }
        require(
            !stderr.contains("left out"),
            "nothing is left out when the CLI lists the flags",
        )?;
    }

    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

/// hivemind-s15q.15: the capture scripts ask the CLI to work a decision's project out from the
/// folder they run in (the nearest `.loomtracer-project` walking up). Tests that are not about
/// projects run from here instead of the repository root, where this rig's own marker sits: the
/// fresh test ledger has never registered that project, so a capture from the root would be
/// refused.
fn markerless_cwd() -> std::path::PathBuf {
    std::env::temp_dir()
}

#[test]
fn supersede_script_works_out_the_project_from_the_folder_it_runs_in() -> TestResult<()> {
    // hivemind-s15q.15: supersede records a new decision, so the context plugin's script hands the
    // CLI the same "work the project out from where this runs" switch the capture script does. It
    // is a flag of the `supersede` subcommand: it must land after the subcommand name.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = unique_temp_dir("loomtracer-supersede-project")?;
    let loomtracer_dir = scratch.join("loomtracer");
    let loomtracer = |args: &[&str]| -> TestResult<std::process::Output> {
        Ok(Command::new(env!("CARGO_BIN_EXE_loomtracer"))
            .arg("--loomtracer-dir")
            .arg(&loomtracer_dir)
            .args(["--actor", "human:test-registrar"])
            .args(args)
            .output()?)
    };
    for handle in ["billing", "payments"] {
        let registered = loomtracer(&["project", "register", handle])?;
        require(
            registered.status.success(),
            format!(
                "register {handle} failed: {}",
                String::from_utf8_lossy(&registered.stderr)
            ),
        )?;
        // A registered project's topic keys are declared (hivemind-zywz): declare the ones the
        // fixture captures below use.
        let declared = loomtracer(&["project", "declare-topic", handle, "billing", "cadence"])?;
        require(
            declared.status.success(),
            format!(
                "declare topics for {handle} failed: {}",
                String::from_utf8_lossy(&declared.stderr)
            ),
        )?;
    }

    let script_env = |command: &mut Command| {
        command
            .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
            .env("LOOMTRACER_DIR", &loomtracer_dir)
            .env("CLAUDE_SESSION_ID", "supersede-project-session")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("GC_AGENT")
            .env_remove("GC_ALIAS")
            .env_remove("GC_RIG");
    };
    let capture_script = root.join("plugins/loomtracer-capture/scripts/capture-decision.sh");
    for title in ["Ship invoices weekly", "Ship refunds daily"] {
        let mut capture = Command::new(&capture_script);
        script_env(&mut capture);
        let output = capture
            .current_dir(markerless_cwd())
            .args([
                "--title",
                title,
                "--rationale",
                "The fixture decision the supersede script will replace under a project",
                "--topic-keys",
                "billing,cadence",
                "--options",
                "weekly,daily",
                "--chose",
                "weekly",
                "--bet",
                "--binds-nothing",
                "--cheap-to-undo",
                "--project",
                "billing",
            ])
            .output()?;
        require(
            output.status.success(),
            format!(
                "fixture capture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )?;
    }

    let supersede_script = root.join("plugins/loomtracer-context/scripts/supersede.sh");
    let supersede = |old: &str, title: &str, cwd: &Path| -> TestResult<String> {
        let mut command = Command::new(&supersede_script);
        script_env(&mut command);
        let output = command
            .current_dir(cwd)
            .args([
                old,
                "--title",
                title,
                "--rationale",
                "Replacing the fixture decision to see which project the replacement lands in",
                "--topic-keys",
                "billing,cadence",
                "--options",
                "monthly,daily",
                "--chose",
                "monthly",
                "--bet",
                "--binds-nothing",
                "--cheap-to-undo",
            ])
            .output()?;
        require(
            output.status.success(),
            format!(
                "supersede failed: {}\nstdout: {}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            ),
        )?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };

    // From a folder attached to another project, the replacement goes there.
    let attached = scratch.join("attached");
    fs::create_dir_all(&attached)?;
    fs::write(attached.join(".loomtracer-project"), "payments\n")?;
    let marked = supersede("Ship invoices weekly", "Ship invoices monthly", &attached)?;
    require_contains(
        &marked,
        "project=payments project_source=folder_marker",
        "supersede from a marker folder",
    )?;

    // From an unattached folder nothing is found, so the replacement stays where the old one
    // was, and says it inherited that project.
    let bare = scratch.join("bare");
    fs::create_dir_all(&bare)?;
    let inherited = supersede("Ship refunds daily", "Ship refunds monthly", &bare)?;
    require_contains(
        &inherited,
        "project=billing project_source=inherited",
        "supersede from a bare folder",
    )?;

    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

/// One grounded `emit decision.capture --chose` through the real binary, in an environment
/// scrubbed down to `extra_env` plus the HOME/PATH git needs, so the ambient agent (if any) is
/// exactly what the caller says. Returns who the ledger recorded as proposing it and as
/// accepting it.
fn capture_through_binary(
    repo: &Path,
    loomtracer_dir: &Path,
    actor_flag: Option<&str>,
    extra_env: &[(&str, &str)],
    title: &str,
) -> TestResult<(loomtracer::events::Event, loomtracer::events::Event)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_loomtracer"));
    command
        .current_dir(repo)
        .env_clear()
        .env("HOME", repo)
        .env("PATH", std::env::var("PATH")?)
        .envs(extra_env.iter().copied());
    if let Some(actor) = actor_flag {
        command.args(["--actor", actor]);
    }
    let output = command
        .arg("--json")
        .arg("--loomtracer-dir")
        .arg(loomtracer_dir)
        .args([
            "emit",
            "decision.capture",
            "--binds-nothing",
            "--cheap-to-undo",
            "--title",
            title,
            "--rationale",
            "Which build do testers get? Nightly, because they report bugs on fresh code",
            "--topic-keys",
            "releases",
            "--options",
            "nightly,tagged",
            "--chose",
            "nightly",
            "--rests-on-assumption",
            "Testers tolerate occasional breakage",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output: Value = serde_json::from_slice(&output.stdout)?;
    let decision_id = output["value"].as_str().expect("decision id").to_owned();
    let proposal = proposal_event(loomtracer_dir, &decision_id)?;
    let acceptance = SqliteEventLedger::open(loomtracer_dir)?
        .read(0, 100)?
        .into_iter()
        .find(|event| {
            event.event_type == loomtracer::events::EventType::DecisionAccepted
                && event.payload.get("decision_id").and_then(Value::as_str)
                    == Some(decision_id.as_str())
        })
        .ok_or("--chose self-accepts the decision")?;
    Ok((proposal, acceptance))
}

#[test]
fn human_cli_capture_is_recorded_as_the_human_and_an_agent_stays_an_agent() -> TestResult<()> {
    // hivemind-6ait: `--actor human:alice emit decision.capture` recorded the person's
    // decision as agent:codex:manual-session (or agent:claude:<alias> under Claude Code).
    let scratch = unique_temp_dir("loomtracer-human-cli-capture")?;
    let repo = scratch.join("repo");
    let loomtracer_dir = scratch.join("ledger");
    fs::create_dir_all(&repo)?;
    run_git(&repo, ["init"])?;
    run_git(&repo, ["config", "user.email", "Ada.Example@Example.COM"])?;

    struct Case {
        name: &'static str,
        actor_flag: Option<&'static str>,
        env: &'static [(&'static str, &'static str)],
        expected_actor: &'static str,
        expected_source: &'static str,
    }
    let cases = [
        Case {
            name: "a typed --actor at a plain terminal",
            actor_flag: Some("human:alice"),
            env: &[],
            expected_actor: "human:alice",
            expected_source: "human",
        },
        Case {
            name: "a typed --actor under Claude Code",
            actor_flag: Some("human:alice"),
            env: &[("CLAUDE_CODE_SESSION_ID", "sess-1")],
            expected_actor: "human:alice",
            expected_source: "human",
        },
        Case {
            name: "no --actor at a plain terminal",
            actor_flag: None,
            env: &[],
            expected_actor: "human:ada.example@example.com",
            expected_source: "human",
        },
        Case {
            name: "no --actor under Claude Code",
            actor_flag: None,
            env: &[("CLAUDE_CODE_SESSION_ID", "sess-1")],
            expected_actor: "agent:claude:sess-1",
            expected_source: "agent",
        },
    ];
    for (index, case) in cases.iter().enumerate() {
        let title = format!("Use nightly builds for testers ({index})");
        let (proposal, acceptance) =
            capture_through_binary(&repo, &loomtracer_dir, case.actor_flag, case.env, &title)?;
        for (what, event) in [("proposal", &proposal), ("acceptance", &acceptance)] {
            require_eq(
                event.actor_id.as_str(),
                case.expected_actor,
                &format!("{}: {what} actor", case.name),
            )?;
            require_eq(
                event.source.as_str(),
                case.expected_source,
                &format!("{}: {what} source", case.name),
            )?;
        }
    }

    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

/// The step-3 `bash` block of the skill's direct form: the one an agent runs when the helper is
/// unavailable.
fn skill_direct_capture_block(skill: &str) -> TestResult<String> {
    let after = skill
        .split("3. Capture a new proposed decision directly when the helper is unavailable:")
        .nth(1)
        .ok_or("the skill should show the direct capture form as step 3")?;
    let block = after
        .split("```bash\n")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .ok_or("step 3 should carry a bash block")?;
    Ok(block.to_owned())
}

/// A spawned process that sees no agent: the variables `without_agent_env` clears, plus the
/// ones the capture helpers and the skill's direct form read on top of them.
fn without_ambient_agent_session(command: &mut Command) -> &mut Command {
    without_agent_env(command);
    for key in [
        "CODEX_THREAD_ID",
        "CLAUDE_PROJECT_DIR",
        "CLAUDE_PLUGIN_ROOT",
        "GC_SESSION_ID",
        "GC_SESSION_NAME",
    ] {
        command.env_remove(key);
    }
    command
}

/// Runs one capture path with `LOOMTRACER_AGENT_SESSION` pinned to `alice-laptop` and the client's
/// own per-run session variable set to `session_id`, and returns the proposal it recorded.
#[cfg(unix)]
fn capture_pinned(
    label: &str,
    mut command: Command,
    bin_dir: &Path,
    ledger: &Path,
    (session_var, session_id): (&str, &str),
) -> TestResult<loomtracer::events::Event> {
    without_ambient_agent_session(&mut command);
    let output = command
        .current_dir(markerless_cwd())
        .env(
            "PATH",
            format!("{}:{}", bin_dir.display(), std::env::var("PATH")?),
        )
        .env("LOOMTRACER_CAPTURE_BIN", env!("CARGO_BIN_EXE_loomtracer"))
        .env("LOOMTRACER_DIR", ledger)
        .env("LOOMTRACER_AGENT_SESSION", "alice-laptop")
        .env(session_var, session_id)
        .output()?;
    require(
        output.status.success(),
        format!(
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    event_with_type(ledger, loomtracer::events::EventType::DecisionProposed)
}

/// hivemind-9v7vu: the docs tell a person outside Gas City to set `LOOMTRACER_AGENT_SESSION` to
/// keep one actor, and the MCP server and bare `emit` honour it. The plugin helper, the
/// repo-local helper and the skill's direct form each named the session from their own list of
/// variables and never looked at it, so every session still landed under a new actor. Each path
/// runs twice with one pin and two different per-run session ids.
#[cfg(unix)]
#[test]
fn a_pinned_agent_session_keeps_one_actor_through_the_helpers_and_the_skills_direct_form(
) -> TestResult<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = unique_temp_dir("loomtracer-pinned-session")?;
    // The skill's direct form calls `loomtracer` off `PATH`.
    let bin_dir = scratch.join("bin");
    fs::create_dir_all(&bin_dir)?;
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_loomtracer"), bin_dir.join("loomtracer"))?;
    let skill_block = skill_direct_capture_block(&fs::read_to_string(
        root.join("plugins/loomtracer-capture/skills/loomtracer-capture/SKILL.md"),
    )?)?;

    let capture_args = [
        "--title",
        "Keep one actor through the pinned session",
        "--rationale",
        "A pinned LOOMTRACER_AGENT_SESSION must name the actor whichever helper records the capture",
        "--topic-keys",
        "identity,capture",
        "--options",
        "pin,per-run",
        "--chose",
        "pin",
        "--rests-on-assumption",
        "The pin is the one name a person gave their agent",
        "--binds-nothing",
        "--cheap-to-undo",
    ];
    let plugin_helper = || {
        let mut command =
            Command::new(root.join("plugins/loomtracer-capture/scripts/capture-decision.sh"));
        command.args(capture_args);
        command
    };
    let repo_local_helper = || {
        let mut command = Command::new(root.join(".claude/scripts/capture-decision.sh"));
        command.args(["--source", "agent"]).args(capture_args);
        command
    };
    let skill_form = || {
        let mut command = Command::new("bash");
        command.args(["-c", &skill_block]);
        command
    };

    const CLAUDE: &str = "CLAUDE_CODE_SESSION_ID";
    const CODEX: &str = "CODEX_THREAD_ID";
    // (label, how to run it, the client's per-run session variable, the actor it must record)
    let cases: [(&str, &dyn Fn() -> Command, &str, &str); 5] = [
        (
            "plugin-claude",
            &plugin_helper,
            CLAUDE,
            "agent:claude:alice-laptop",
        ),
        (
            "plugin-codex",
            &plugin_helper,
            CODEX,
            "agent:codex:alice-laptop",
        ),
        (
            "repo-local",
            &repo_local_helper,
            CLAUDE,
            "agent:claude:alice-laptop",
        ),
        (
            "skill-claude",
            &skill_form,
            CLAUDE,
            "agent:claude:alice-laptop",
        ),
        (
            "skill-codex",
            &skill_form,
            CODEX,
            "agent:codex:alice-laptop",
        ),
    ];
    for (label, command, session_var, expected_actor) in cases {
        for session_id in ["88888888-bbbb", "99999999-cccc"] {
            let run = format!("{label} ({session_var}={session_id})");
            let proposal = capture_pinned(
                &run,
                command(),
                &bin_dir,
                &scratch.join(format!("{label}-{session_id}")),
                (session_var, session_id),
            )?;
            require_eq(proposal.actor_id.as_str(), expected_actor, &run)?;
            // The pin names the actor; the run it came from stays in provenance where the
            // helper can see it (hivemind-zdsh.9).
            if label.starts_with("plugin-") {
                require_eq(proposal.source_ref.as_deref(), Some(session_id), &run)?;
            }
        }
    }

    // The context plugin's write verbs (`disagree`, `supersede`, `ground`, `reaffirm`) pass
    // `--actor agent:<tool>:<name>` and name the session the same way the capture helper does, so
    // a pinned person is one actor across a capture and the verbs that follow it.
    let context_lib = root.join("plugins/loomtracer-context/scripts/lib.sh");
    for (tool, session_var) in [("claude", CLAUDE), ("codex", CODEX)] {
        let mut command = Command::new("bash");
        command.args([
            "-c",
            &format!(
                "source '{}' && detect_agent_session {tool}",
                context_lib.display()
            ),
        ]);
        let output = without_ambient_agent_session(&mut command)
            .env("LOOMTRACER_AGENT_SESSION", "alice-laptop")
            .env(session_var, "88888888-bbbb")
            .output()?;
        require_eq(
            String::from_utf8_lossy(&output.stdout).trim(),
            "alice-laptop",
            &format!("loomtracer-context under {tool}"),
        )?;
    }

    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

/// A spawned CLI that sees no agent session: every variable `identity` reads to name an agent
/// (hivemind-jglb7) is removed, so a person-at-a-terminal assertion holds when `cargo test` runs
/// inside a Claude Code, Codex or Gas City session too (hivemind-2qg60).
fn without_agent_env(command: &mut Command) -> &mut Command {
    for key in [
        "LOOMTRACER_ACTOR",
        "LOOMTRACER_AGENT_TOOL",
        "LOOMTRACER_TOOL",
        "LOOMTRACER_AGENT_SESSION",
        "LOOMTRACER_CODEX_SESSION",
        "LOOMTRACER_CLAUDE_SESSION",
        "CLAUDE_SESSION_ID",
        "CLAUDE_CODE_SESSION_ID",
        "CODEX_SESSION_ID",
        "CODEX_TASK_ID",
        "GC_AGENT",
        "GC_ALIAS",
        "GC_RIG",
    ] {
        command.env_remove(key);
    }
    command
}

fn unique_temp_dir(label: &str) -> TestResult<std::path::PathBuf> {
    let unique = uuid::Uuid::new_v4().to_string();
    let path = std::env::temp_dir().join(format!("{label}-{unique}"));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn run_git<const N: usize>(cwd: &Path, args: [&str; N]) -> TestResult<()> {
    let output = Command::new("git").current_dir(cwd).args(args).output()?;
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn proposal_event(
    loomtracer_dir: &Path,
    decision_id: &str,
) -> TestResult<loomtracer::events::Event> {
    let ledger = SqliteEventLedger::open(loomtracer_dir)?;
    let events = ledger.read(0, 100)?;
    events
        .into_iter()
        .find(|event| {
            event.event_type == loomtracer::events::EventType::DecisionProposed
                && event.payload.get("decision_id").and_then(Value::as_str) == Some(decision_id)
        })
        .ok_or_else(|| format!("proposal event for {decision_id} should exist").into())
}

fn event_with_type(
    loomtracer_dir: &Path,
    event_type: loomtracer::events::EventType,
) -> TestResult<loomtracer::events::Event> {
    let ledger = SqliteEventLedger::open(loomtracer_dir)?;
    let events = ledger.read(0, 100)?;
    events
        .into_iter()
        .find(|event| event.event_type == event_type)
        .ok_or_else(|| format!("{event_type:?} event should exist").into())
}

fn require(condition: bool, message: impl Into<String>) -> TestResult<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into().into())
    }
}

fn require_contains(haystack: &str, needle: &str, message: &str) -> TestResult<()> {
    require(
        haystack.contains(needle),
        format!("{message}: missing {needle:?}"),
    )
}

fn require_eq<T>(actual: T, expected: T, message: &str) -> TestResult<()>
where
    T: PartialEq + std::fmt::Debug,
{
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{message}: expected {expected:?}, got {actual:?}").into())
    }
}

fn assert_no_todos(name: &str, body: &str) {
    assert!(
        !body.contains("[TODO"),
        "{name} should not contain scaffold TODO placeholders"
    );
}

fn verify_active_capture_skill(name: &str, body: &str) -> TestResult<()> {
    assert_no_todos(name, body);
    require_contains(body, "name: active-capture", name)?;
    require_contains(body, "comparing alternatives", name)?;
    require_contains(
        body,
        "/capture <text> [--kind decision|evidence|hypothesis]",
        name,
    )?;
    require_contains(body, "`decision`", name)?;
    require_contains(body, "`evidence`", name)?;
    require_contains(body, "`hypothesis`", name)?;
    // capture.sh refuses these two kinds and records nothing, so the skill must not teach them.
    for refused in ["--kind blocker", "--kind notification"] {
        if body.contains(refused) {
            return Err(format!("{name} teaches `{refused}`, which capture.sh refuses").into());
        }
    }
    require_contains(
        body,
        "Do NOT call this for synthetic test data or routing chatter",
        name,
    )?;
    require_contains(body, "gc sling", name)?;
    require_contains(body, "br update", name)?;
    require_contains(body, "do not write", name)?;
    require_contains(body, "directly to the ledger from this skill", name)?;
    Ok(())
}

fn assert_executable(path: impl AsRef<Path>) -> TestResult<()> {
    let path = path.as_ref();
    let metadata = fs::metadata(path).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("{} has metadata: {error}", path.display()),
        )
    })?;
    #[cfg(unix)]
    {
        assert_ne!(
            metadata.permissions().mode() & 0o111,
            0,
            "{} should be executable",
            path.display()
        );
    }
    #[cfg(not(unix))]
    {
        assert!(
            metadata.is_file(),
            "{} should be a regular file",
            path.display()
        );
    }
    Ok(())
}
