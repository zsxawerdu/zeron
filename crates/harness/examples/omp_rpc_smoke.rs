//! W01 happy-path check against the real `omp` binary:
//! spawn `omp --mode rpc-ui` through the vendored client and Zeron's
//! supervisor, run one real model turn, abort a second, shut down cleanly.
//!
//! Uses the caller's default OMP profile (its configured model and
//! credentials) in a disposable temp workspace with an ephemeral
//! (`--no-session`) session — two tiny prompts, nothing persisted.
//!   cargo run -p zeron-harness --example omp_rpc_smoke
use std::time::{Duration, Instant};

use omp_rpc::wire::{
    GetAvailableCommandsCommand, PromptCommand, SetAskDialogCommand,
    SetSubagentSubscriptionCommand, SubagentSubscriptionLevel,
};
use omp_rpc::{Event, RpcNotification};
use zeron_harness::omp::{OmpExecutable, OmpLaunch, OmpRuntime, OmpSession};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let exe = OmpExecutable::resolve(None)?;
    println!(
        "exe: {} version={:?} qualified={}",
        exe.path.display(),
        exe.version,
        exe.qualified()
    );

    let cwd = std::env::temp_dir().join(format!("zeron-omp-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&cwd)?;
    let mut launch = OmpLaunch::new(&cwd);
    launch.session = OmpSession::Ephemeral;

    let mut runtime = OmpRuntime::spawn(exe, &launch, 1)?;
    let caps = runtime.capabilities();
    println!(
        "spawned: pid={:?} protocol={} max_frame={:?}",
        runtime.pid(),
        caps.protocol_version,
        caps.max_frame_bytes
    );

    // Drain events on their own thread; report session_settled arrivals.
    let events = runtime.take_events().expect("events present");
    let (settled_tx, settled_rx) = std::sync::mpsc::channel::<()>();
    let lifecycle_log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let lifecycle_in_drain = lifecycle_log.clone();
    let drain = std::thread::spawn(move || {
        let mut count = 0usize;
        for event in events {
            count += 1;
            match &event {
                Event::Notification(RpcNotification::SessionSettled(_)) => {
                    let _ = settled_tx.send(());
                }
                Event::Notification(RpcNotification::SubagentLifecycle(lifecycle)) => {
                    lifecycle_in_drain.lock().unwrap().push(format!(
                        "{}:{:?}",
                        lifecycle.payload.id, lifecycle.payload.status
                    ));
                }
                _ => {}
            }
        }
        count
    });

    let state = runtime.state()?;
    println!(
        "state: session={} model={:?} streaming={}",
        state.session_id,
        state.model.as_ref().map(|m| format!("{}/{}", m.provider, m.id)),
        state.is_streaming
    );
    let commands = runtime.call(&GetAvailableCommandsCommand {})?;
    println!("commands discovered: {}", commands.len());
    let ask = runtime.call(&SetAskDialogCommand { enabled: true })?;
    println!("rich ask enabled: {ask}");

    // One real turn.
    let turn = runtime.prompt_and_wait(
        &PromptCommand {
            message: "Reply with exactly this word: ok".into(),
            images: None,
            streaming_behavior: None,
        },
        Duration::from_secs(180),
    )?;
    println!(
        "turn: status={:?} settled={:?} text={:?}",
        turn.result.as_ref().map(|r| r.status),
        turn.result.as_ref().map(|r| r.session_settled),
        turn.assistant_text.as_deref().map(|t| t.trim().to_owned())
    );

    // Abort a second, long turn and wait for the session to go quiet.
    runtime.call(&PromptCommand {
        message: "Count from 1 to 100000, one number per line, no commentary.".into(),
        images: None,
        streaming_behavior: None,
    })?;
    std::thread::sleep(Duration::from_millis(750));
    runtime.abort()?;
    println!("abort sent");
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut settled = false;
    while Instant::now() < deadline {
        if settled_rx.recv_timeout(Duration::from_millis(250)).is_ok() {
            settled = true;
            break;
        }
        if !runtime.state()?.is_streaming {
            settled = true;
            break;
        }
    }
    println!("settled after abort: {settled}");
    if !settled {
        return Err("session did not settle after abort".into());
    }

    // Subagent snapshot + cancel against a live worker.
    while settled_rx.try_recv().is_ok() {}
    runtime.call(&SetSubagentSubscriptionCommand {
        level: SubagentSubscriptionLevel::Events,
    })?;
    runtime.call(&PromptCommand {
        message: "Use the task tool to spawn exactly one subagent. Its assignment: write a detailed essay of at least 3000 words about the history of computing, section by section. Do not write the essay yourself; wait for the subagent to finish, then confirm completion in one short sentence.".into(),
        images: None,
        streaming_behavior: None,
    })?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut target = None;
    while Instant::now() < deadline {
        let snapshot = runtime.subagents()?;
        if let Some(sub) = snapshot.first() {
            println!("subagent snapshot: id={} agent={} status={:?}", sub.id, sub.agent, sub.status);
            target = Some(sub.id.clone());
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let Some(subagent_id) = target else {
        return Err("no subagent appeared in get_subagents snapshot".into());
    };
    let cancelled = runtime.cancel_subagent(&subagent_id)?;
    println!("cancel_subagent({subagent_id}) accepted: {cancelled}");
    // The parent turn settles once the cancelled worker resolves; abort
    // as a backstop so the probe can never hang on a stubborn model.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut settled = false;
    while Instant::now() < deadline {
        if settled_rx.recv_timeout(Duration::from_millis(250)).is_ok() {
            settled = true;
            break;
        }
    }
    if !settled {
        runtime.abort()?;
        let _ = settled_rx.recv_timeout(Duration::from_secs(30));
    }
    // The terminal lifecycle frame can trail session settlement; give it
    // a bounded moment before judging (closing first would race it).
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut log = lifecycle_log.lock().unwrap().clone();
    while Instant::now() < deadline
        && !log.iter().any(|entry| {
            entry.starts_with(&subagent_id)
                && (entry.contains("Aborted")
                    || entry.contains("Completed")
                    || entry.contains("Failed"))
        })
    {
        std::thread::sleep(Duration::from_millis(200));
        log = lifecycle_log.lock().unwrap().clone();
    }
    println!("subagent lifecycle: {log:?}");
    if !log.iter().any(|entry| entry.starts_with(&subagent_id)) {
        return Err("no lifecycle frames observed for the subagent".into());
    }

    runtime.close()?;
    let drained = drain.join().expect("drain thread");
    println!("closed cleanly; events drained: {drained}");
    let _ = std::fs::remove_dir_all(&cwd);
    Ok(())
}
