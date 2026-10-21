//! W02/W03 happy-path check for `OmpHarness` through the engine's driver
//! trait: catalog discovery, one real turn with a tool call (persistent
//! native session in a disposable temp workspace), then a second run
//! resuming that session by its session-file token.
//!   cargo run -p zeron-harness --example omp_harness_smoke
use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};
use zeron_harness::{CancellationToken, Harness, OmpHarness, RunControls, TurnControl};
use zeron_proto::{AgentEvent, HarnessId, RunRequest, SandboxLevel};

/// The probe never steers, but the steering channel must stay open for
/// the whole run — the caller owns the sender until the turn ends.
fn controls(approve_confirms: bool) -> (RunControls, mpsc::Sender<zeron_harness::SteerMessage>) {
    let (steer_tx, steering) = mpsc::channel(16);
    let controls = RunControls {
        realtime: None,
        execution_lease: None,
        // Dialogs: confirms answer per `approve_confirms` (the probe's
        // deny/approve turns) — default turns never approve. Other
        // questions get their first option (ask round-trip).
        request_input: Box::new(move |questions| {
            eprintln!("  [probe dialog] {questions:?}");
            let (tx, rx) = oneshot::channel();
            let answers = questions
                .iter()
                .map(|question| {
                    // Approval arrives as a select (Approve / Approve …
                    // for this session / Deny): grant exactly the one-time
                    // "Approve", never the session-wide variant.
                    let labels = if question.options.iter().any(|o| o == "Deny") {
                        vec![if approve_confirms { "Approve" } else { "Deny" }.to_string()]
                    } else if question.options.iter().any(|o| o == "Yes") {
                        vec![if approve_confirms { "Yes" } else { "No" }.to_string()]
                    } else if let Some(first) = question.options.first() {
                        vec![first.clone()]
                    } else {
                        Vec::new()
                    };
                    zeron_proto::UserInputAnswer {
                        question_id: question.id.clone(),
                        labels,
                    }
                })
                .collect();
            let _ = tx.send(answers);
            rx
        }),
        steering,
        interrupt: CancellationToken::new(),
        turn: TurnControl::default(),
    };
    (controls, steer_tx)
}

fn request(prompt: &str, cwd: &str, resume: Option<String>) -> RunRequest {
    omp: None,
    RunRequest {
        omp: None,
        prompt: prompt.into(),
        harness: Some(HarnessId::Omp),
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: cwd.into(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: false,
        resume,
        attachments: Vec::new(),
        worktree: None,
        mcp: None,
    }
}

async fn run_turn(
    harness: &OmpHarness,
    request: RunRequest,
    approve_confirms: bool,
) -> Result<(String, Option<String>, Vec<String>, usize, Vec<String>, Option<String>), Box<dyn std::error::Error>> {
    let (controls, _steer_tx) = controls(approve_confirms);
    let mut stream = harness.run(request, controls).await?;
    let mut text = String::new();
    let mut subagent_events = 0usize;
    let mut session = None;
    let mut tools: Vec<String> = Vec::new();
    let mut todo_texts: Vec<String> = Vec::new();
    let mut leaf: Option<String> = None;
    let mut done_status = None;
    while let Some(item) = stream.next().await {
        match item? {
            AgentEvent::SessionStarted { session_id, model, .. } => {
                println!("  session started: model={model} id={session_id}");
                if !session_id.is_empty() {
                    session = Some(session_id);
                }
            }
            AgentEvent::OmpSessionBound { binding } => {
                leaf = binding.leaf_id.clone().or(leaf);
                println!(
                    "  bound: session={} file={:?} leaf={:?} generation={}",
                    binding.session_id, binding.session_file, binding.leaf_id, binding.generation
                );
                assert!(
                    binding
                        .session_file
                        .as_deref()
                        .is_some_and(|file| file.ends_with(".jsonl")),
                    "binding carries the native session file"
                );
            }
            AgentEvent::TextDelta { text: delta } => text.push_str(&delta),
            AgentEvent::ToolCall { id, call } => {
                if let zeron_proto::ToolCall::Todo { items } = &call {
                    todo_texts = items.iter().map(|item| item.text.clone()).collect();
                }
                tools.push(id);
                println!("  tool call: {call:?}");
            }
            AgentEvent::ToolResult { is_error, .. } => {
                println!("  tool result: error={is_error}");
            }
            AgentEvent::Error { message } => println!("  error event: {message}"),
            AgentEvent::Done { status, error, session_id, .. } => {
                println!("  done: {status:?} error={error:?}");
                done_status = Some(status);
                if session_id.is_some() {
                    session = session_id;
                }
            }
            AgentEvent::Subagent { parent_tool_use_id, event } => {
                subagent_events += 1;
                println!("  subagent[{parent_tool_use_id}]: {event:?}");
            }
            _ => {}
        }
    }
    match done_status {
        Some(zeron_proto::DoneStatus::Completed) => {
            Ok((text, session, tools, subagent_events, todo_texts, leaf))
        }
        other => Err(format!("turn did not complete: {other:?}").into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let harness = OmpHarness::new();
    println!("installed: {}", harness.installed());
    let models = harness.models().await?;
    println!("models discovered: {}", models.len());
    for model in models.iter().take(3) {
        println!("  {} ({})", model.id, model.label);
    }
    let with_levels = models.iter().filter(|m| !m.reasoning_levels.is_empty()).count();
    println!("models with thinking levels: {with_levels}");
    if let Some(model) = models.iter().find(|m| !m.reasoning_levels.is_empty()) {
        println!("  e.g. {} levels: {:?}", model.id, model.reasoning_levels);
    }
    // Login discovery (read-only): providers + auth state as OMP reports.
    let providers = harness.login_providers().await?;
    let discovered = harness.discover_sessions().await;
    println!("existing native sessions discovered: {}", discovered.len());
    if let Some(newest) = discovered.first() {
        println!("  newest: cwd={:?} title={:?}", newest.cwd, newest.title);
    }
    let authenticated = providers.iter().filter(|p| p.authenticated).count();
    println!("login providers: {} ({authenticated} authenticated)", providers.len());

    let cwd = std::env::temp_dir().join(format!("zeron-omp-harness-{}", std::process::id()));
    std::fs::create_dir_all(&cwd)?;
    std::fs::write(cwd.join("seed.txt"), "zeron-omp-seed-42\n")?;
    let cwd = cwd.to_string_lossy().into_owned();

    println!("turn 1 (tool call, new session):");
    let (text, session, tools, _, _, leaf) = run_turn(
        &harness,
        request(
            "Use the read tool to read seed.txt in the current directory, then reply with exactly its contents and nothing else.",
            &cwd,
            None,
        ),
        false,
    )
    .await?;
    println!("  text: {text:?} tools: {tools:?}");
    if !text.contains("zeron-omp-seed-42") || tools.is_empty() {
        return Err("turn 1 did not read the seed file through a tool".into());
    }
    let session = session.ok_or("no native session token returned")?;
    println!("  resume token: {session}");
    if leaf.is_none() {
        return Err("binding never carried a native leaf id".into());
    }

    println!("turn 2 (resume):");
    let (text, _, _, _, _, _) = run_turn(
        &harness,
        request(
            "What exact token was in the file you just read? Reply with only the token.",
            &cwd,
            Some(session.clone()),
        ),
        false,
    )
    .await?;
    println!("  text: {text:?}");
    if !text.contains("zeron-omp-seed-42") {
        return Err("resumed session did not recall the token".into());
    }
    // Crash-recovery gate against this session's native history: the
    // turn-2 prompt was admitted (must not resend); a fresh prompt was
    // not (safe to dispatch).
    let cwd_path = std::path::Path::new(&cwd);
    let admitted = harness
        .safe_to_redispatch(
            &session,
            cwd_path,
            "What exact token was in the file you just read? Reply with only the token.",
        )
        .await;
    let fresh = harness
        .safe_to_redispatch(&session, cwd_path, "A prompt that was never sent in this session.")
        .await;
    println!("redispatch gate: admitted-prompt safe={admitted} fresh-prompt safe={fresh}");
    if admitted || !fresh {
        return Err("redispatch gate misjudged native history".into());
    }
    // Owner-side detail fetch: turn 1's read, straight from native
    // history (original arguments + result payload intact).
    let detail = harness.tool_detail(&session, cwd_path, &tools[0]).await;
    println!("tool detail: {detail:?}");
    match detail {
        zeron_proto::omp::OmpToolDetail::Available { native, .. } => {
            if native["call"]["name"] != "read"
                || !native["result"].to_string().contains("zeron-omp-seed-42")
            {
                return Err("tool detail missing the read call/result".into());
            }
        }
        other => return Err(format!("expected available tool detail, got {other:?}").into()),
    }
    let missing = harness.tool_detail(&session, cwd_path, "no-such-tool-call").await;
    if !matches!(missing, zeron_proto::omp::OmpToolDetail::Expired) {
        return Err(format!("expected expired for unknown tool call, got {missing:?}").into());
    }
    // Rich ask round-trip: the model must ask, receive "Red" (first
    // option from the probe's responder), and echo it back.
    println!("turn 3 (ask):");
    let (text, _, _, _, _, _) = run_turn(
        &harness,
        request(
            "You must use the ask tool now. Ask exactly one question: 'Pick a color' with options Red and Blue (in that order). After I answer, reply with exactly the chosen label and nothing else.",
            &cwd,
            Some(session.clone()),
        ),
        false,
    )
    .await?;
    println!("  text: {text:?}");
    if text.trim() != "Red" {
        return Err(format!("ask round-trip failed: expected Red, got {text:?}").into());
    }
    // Approval gate: with OMP's always-ask mode (session-scoped), a
    // write must be denied without a Yes and executed with one.
    let strict = OmpHarness::new().with_approval_mode(zeron_harness::OmpApprovalMode::AlwaysAsk);
    println!("turn 4 (approval denied):");
    let _ = run_turn(
        &strict,
        request(
            "Use the write tool to create a file named denied.txt in the current directory containing exactly: hello",
            &cwd,
            None,
        ),
        false,
    )
    .await?;
    if std::path::Path::new(&cwd).join("denied.txt").exists() {
        return Err("denied write created the file anyway".into());
    }
    println!("  denied.txt absent, as required");
    println!("turn 5 (approval granted):");
    let _ = run_turn(
        &strict,
        request(
            "Use the write tool to create a file named approved.txt in the current directory containing exactly: hello",
            &cwd,
            None,
        ),
        true,
    )
    .await?;
    let approved = std::fs::read_to_string(std::path::Path::new(&cwd).join("approved.txt"))
        .unwrap_or_default();
    if !approved.contains("hello") {
        return Err(format!("approved write did not land: {approved:?}").into());
    }
    println!("  approved.txt written");
    println!("turn 6 (subagent):");
    let (text, _, _, subagent_events, _, _) = run_turn(
        &harness,
        request(
            "Use the task tool to spawn exactly one subagent. Its assignment: read seed.txt in the current directory and report its exact contents. Wait for it to finish, then reply with exactly the contents it reports and nothing else.",
            &cwd,
            Some(session.clone()),
        ),
        false,
    )
    .await?;
    println!("  text: {text:?} subagent events: {subagent_events}");
    if !text.contains("zeron-omp-seed-42") || subagent_events == 0 {
        return Err("subagent turn produced no routed subagent events or wrong text".into());
    }
    println!("turn 7 (phased todos):");
    let (_, _, _, _, todo_texts, _) = run_turn(
        &harness,
        request(
            "Use the todo tool to set up a todo list with a phase named Probe containing two tasks: alpha and beta. Mark alpha as in progress. Then reply with exactly: ok",
            &cwd,
            Some(session.clone()),
        ),
        false,
    )
    .await?;
    println!("  todo snapshot: {todo_texts:?}");
    if !todo_texts.iter().any(|t| t.contains("alpha"))
        || !todo_texts.iter().any(|t| t.contains("beta"))
    {
        return Err("todo snapshot did not flow through the projection".into());
    }
    println!("turn 8 (fork):");
    let forked = harness
        .fork_session(&session, std::path::Path::new(&cwd), None)
        .await?
        .ok_or("fork was cancelled")?;
    println!("  forked: {session} -> {forked}");
    if forked == session {
        return Err("fork returned the original session id".into());
    }
    let (text, _, _, _, _, _) = run_turn(
        &harness,
        request(
            "What exact token was in the seed file from earlier in this conversation? Reply with only the token.",
            &cwd,
            Some(forked),
        ),
        false,
    )
    .await?;
    println!("  fork recall: {text:?}");
    if !text.contains("zeron-omp-seed-42") {
        return Err("forked session lost pre-fork history".into());
    }
    let (text, _, _, _, _, _) = run_turn(
        &harness,
        request(
            "Reply with exactly: original-alive",
            &cwd,
            Some(session.clone()),
        ),
        false,
    )
    .await?;
    if !text.contains("original-alive") {
        return Err("original session no longer usable after fork".into());
    }
    println!("  original session still alive");
    println!("OMP harness smoke passed");
    Ok(())
}
