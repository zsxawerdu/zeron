//! OMP wire events → the engine's `AgentEvent` display projection.
//!
//! This is a presentation mapping (handoff C4): OMP native history stays
//! authoritative and full tool detail is fetched from it owner-side, so
//! truncating display output here never destroys the original. Unknown
//! tools keep their full input (`ToolCall::Unknown`) — the run journal
//! retains it exactly as emitted.

use std::collections::HashMap;

use omp_rpc::wire::{
    AgentMessage, AssistantContent, AssistantMessageEvent, ExtensionUiRequest, RpcAgentEvent,
    RpcNotification,
};
use omp_rpc::Event;
use serde_json::Value;
use zeron_proto::omp::{OmpSlashCommand, OmpSlashSubcommand};
use zeron_proto::{AgentEvent, DoneStatus, ToolCall, ToolDiff};

/// Full-fidelity slash-command projection (W02): aliases, subcommands
/// and source are preserved here; the legacy `SlashCommand` event/cache
/// projection derived from it is the documented lossy boundary (W07).
pub(crate) fn omp_slash_command(
    command: &omp_rpc::wire::AvailableSlashCommand,
) -> OmpSlashCommand {
    OmpSlashCommand {
        name: command.name.clone(),
        description: command.description.clone().unwrap_or_default(),
        input_hint: command.input.as_ref().and_then(|input| input.hint.clone()),
        aliases: command.aliases.clone().unwrap_or_default(),
        subcommands: command
            .subcommands
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|sub| OmpSlashSubcommand {
                name: sub.name,
                description: sub.description,
                usage: sub.usage,
            })
            .collect(),
        source: command.source.as_str().to_owned(),
    }
}

/// Stateful mapper: tool diffs are reconstructed from start-args at
/// end-time, and final message content is reconciled against streamed
/// deltas so a coalesced/lost delta cannot truncate the rendered text —
/// the same invariants as the Pi driver.
/// Routing + projection state for one live subagent.
struct SubagentRoute {
    parent_tool_call_id: String,
    normalizer: Box<Normalizer>,
}

#[derive(Default)]
pub struct Normalizer {
    tools: HashMap<String, (String, Value)>,
    text: HashMap<usize, String>,
    thinking: HashMap<usize, String>,
    window: Option<u64>,
    /// Live subagents by native id: routing to the spawn tool call plus
    /// each child's own projection state (nested tools/text reconcile
    /// independently of the parent's).
    subagents: HashMap<String, SubagentRoute>,
}

impl Normalizer {
    /// Context window of the session's model (from `get_state`), stamped
    /// on `ContextUsage` events.
    pub fn set_window(&mut self, window: Option<i64>) {
        self.window = window.filter(|tokens| *tokens > 0).map(|tokens| tokens as u64);
    }

    /// Map one client event to zero or more projection events. Extension
    /// UI requests are NOT handled here (the runner's dialog bridge owns
    /// them); the caller filters first via [`extension_ui_request`].
    pub fn map(&mut self, event: &Event) -> Vec<AgentEvent> {
        let Event::Notification(notification) = event else {
            return Vec::new();
        };
        match notification {
            RpcNotification::RpcAgentEvent(agent) => self.map_agent(agent),
            RpcNotification::ExtensionError(error) => vec![AgentEvent::Error {
                message: format!("OMP extension: {}", error.error),
            }],
            RpcNotification::AvailableCommandsUpdate(update) => {
                vec![AgentEvent::AvailableCommands {
                    commands: update
                        .commands
                        .iter()
                        .map(|command| omp_slash_command(command).to_legacy())
                        .collect(),
                }]
            }
            RpcNotification::SubagentLifecycle(lifecycle) => {
                self.map_subagent_lifecycle(&lifecycle.payload)
            }
            RpcNotification::SubagentEvent(sub) => {
                let Some(route) = self.subagents.get_mut(&sub.payload.id) else {
                    // No lifecycle seen (subscription started mid-flight):
                    // unroutable, never misattributed to the parent.
                    return Vec::new();
                };
                let omp_rpc::wire::OrUnknown::Known(agent) = &sub.payload.event else {
                    return Vec::new();
                };
                let parent = route.parent_tool_call_id.clone();
                route
                    .normalizer
                    .map_agent(agent)
                    .into_iter()
                    .map(|event| AgentEvent::Subagent {
                        parent_tool_use_id: parent.clone(),
                        event: Box::new(event),
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    fn map_agent(&mut self, event: &RpcAgentEvent) -> Vec<AgentEvent> {
        match event {
            RpcAgentEvent::MessageStart(start) => {
                if matches!(start.message, AgentMessage::Assistant(_)) {
                    self.text.clear();
                    self.thinking.clear();
                }
                Vec::new()
            }
            RpcAgentEvent::MessageUpdate(update) => match &update.assistant_message_event {
                AssistantMessageEvent::TextDelta(delta) => {
                    let Some(text) = &delta.delta else { return Vec::new() };
                    self.text
                        .entry(index(delta.content_index))
                        .or_default()
                        .push_str(text);
                    vec![AgentEvent::TextDelta { text: text.clone() }]
                }
                AssistantMessageEvent::ThinkingDelta(delta) => {
                    let Some(text) = &delta.delta else { return Vec::new() };
                    self.thinking
                        .entry(index(delta.content_index))
                        .or_default()
                        .push_str(text);
                    vec![AgentEvent::ReasoningDelta { text: text.clone() }]
                }
                _ => Vec::new(),
            },
            RpcAgentEvent::MessageEnd(end) => {
                let AgentMessage::Assistant(message) = &end.message else {
                    return Vec::new();
                };
                let mut events = Vec::new();
                if let Some(content) = &message.content {
                    for (index, part) in content.iter().enumerate() {
                        let (cache, value, reasoning) = match part {
                            AssistantContent::Text(text) => {
                                (&mut self.text, text.text.as_deref().unwrap_or(""), false)
                            }
                            AssistantContent::Thinking(thinking) => (
                                &mut self.thinking,
                                thinking.thinking.as_deref().unwrap_or(""),
                                true,
                            ),
                            _ => continue,
                        };
                        let previous = cache.entry(index).or_default();
                        if let Some(suffix) = value
                            .strip_prefix(previous.as_str())
                            .filter(|suffix| !suffix.is_empty())
                        {
                            events.push(if reasoning {
                                AgentEvent::ReasoningDelta { text: suffix.into() }
                            } else {
                                AgentEvent::TextDelta { text: suffix.into() }
                            });
                        }
                        *previous = value.into();
                    }
                }
                if let Some(usage) = &message.usage {
                    let input = count(usage.input) + count(usage.cache_read) + count(usage.cache_write);
                    let output = count(usage.output);
                    events.push(AgentEvent::Usage {
                        input_tokens: input,
                        output_tokens: output,
                    });
                    events.push(AgentEvent::ContextUsage {
                        tokens: Some(count(usage.context_tokens).max(input + output)),
                        window: self.window,
                    });
                }
                events
            }
            RpcAgentEvent::ToolExecutionStart(start) => {
                if self.tools.contains_key(&start.tool_call_id) {
                    return Vec::new();
                }
                let args = start.args.clone().unwrap_or(Value::Null);
                let call = tool_call(&start.tool_name, &args);
                self.tools
                    .insert(start.tool_call_id.clone(), (start.tool_name.clone(), args));
                vec![AgentEvent::ToolCall {
                    id: start.tool_call_id.clone(),
                    call,
                }]
            }
            RpcAgentEvent::ToolExecutionEnd(end) => {
                let info = self.tools.remove(&end.tool_call_id);
                let is_error = end.is_error.unwrap_or(false);
                let output = end.result.as_ref().and_then(result_text);
                let diff = info.as_ref().and_then(|(name, args)| {
                    if is_error {
                        return None;
                    }
                    match name.as_str() {
                        "edit" => Some(ToolDiff {
                            path: string(args, "path").into(),
                            old_text: Some(cap(string(args, "oldText"), 65_536)),
                            new_text: cap(string(args, "newText"), 65_536),
                        }),
                        "write" => Some(ToolDiff {
                            path: string(args, "path").into(),
                            old_text: None,
                            new_text: cap(string(args, "content"), 65_536),
                        }),
                        _ => None,
                    }
                });
                let mut events = vec![AgentEvent::ToolResult {
                    id: end.tool_call_id.clone(),
                    is_error,
                    output: output.map(|text| cap(&text, 16_384)),
                    diff,
                }];
                // OMP todo ops are incremental; the authoritative phased
                // snapshot rides each result's details. Re-emitting the
                // call with it upgrades the folded part in place (the
                // fold replaces a same-id call), so the panel always
                // renders OMP's own effective state — never Zeron's
                // reconstruction from ops.
                if info.as_ref().is_some_and(|(name, _)| name == "todo")
                    && let Some(items) = end.result.as_ref().and_then(todo_items)
                {
                    events.push(AgentEvent::ToolCall {
                        id: end.tool_call_id.clone(),
                        call: ToolCall::Todo { items },
                    });
                }
                events
            }
            // Structured session events (W02/C4): compaction, retry,
            // notices, cache warming, model/thinking/goal/queue changes
            // are preserved with their native payload instead of being
            // dropped. Presentation picks fields; unknown fields survive.
            RpcAgentEvent::AutoCompactionStart(e) => structured("auto_compaction_start", e),
            RpcAgentEvent::AutoCompactionEnd(e) => structured("auto_compaction_end", e),
            RpcAgentEvent::AutoRetryStart(e) => structured("auto_retry_start", e),
            RpcAgentEvent::AutoRetryEnd(e) => structured("auto_retry_end", e),
            RpcAgentEvent::CacheWarmingStart(e) => structured("cache_warming_start", e),
            RpcAgentEvent::CacheWarmingEnd(e) => structured("cache_warming_end", e),
            RpcAgentEvent::RetryFallbackApplied(e) => structured("retry_fallback_applied", e),
            RpcAgentEvent::RetryFallbackSucceeded(e) => {
                structured("retry_fallback_succeeded", e)
            }
            RpcAgentEvent::Notice(e) => structured("notice", e),
            RpcAgentEvent::ModelChanged(e) => structured("model_changed", e),
            RpcAgentEvent::ThinkingLevelChanged(e) => {
                structured("thinking_level_changed", e)
            }
            RpcAgentEvent::GoalUpdated(e) => structured("goal_updated", e),
            RpcAgentEvent::QueueUpdate(e) => structured("queue_update", e),
            RpcAgentEvent::TodoReminder(e) => structured("todo_reminder", e),
            RpcAgentEvent::TodoAutoClear(e) => structured("todo_auto_clear", e),
            RpcAgentEvent::IrcMessage(e) => structured("irc_message", e),
            RpcAgentEvent::AdvisorCostChanged(e) => structured("advisor_cost_changed", e),
            RpcAgentEvent::AdvisorYielded(e) => structured("advisor_yielded", e),
            RpcAgentEvent::TtsrTriggered(e) => structured("ttsr_triggered", e),
            RpcAgentEvent::ConfigWarningsChanged(e) => {
                structured("config_warnings_changed", e)
            }
            _ => Vec::new(),
        }
    }

    fn map_subagent_lifecycle(
        &mut self,
        payload: &omp_rpc::wire::SubagentLifecyclePayload,
    ) -> Vec<AgentEvent> {
        use omp_rpc::wire::SubagentLifecycleStatus as Status;
        match payload.status {
            Status::Started => {
                let Some(parent) = payload.parent_tool_call_id.clone() else {
                    return Vec::new();
                };
                self.subagents.insert(
                    payload.id.clone(),
                    SubagentRoute {
                        parent_tool_call_id: parent.clone(),
                        normalizer: Box::default(),
                    },
                );
                // Seed the child's opening entry with its assignment,
                // like other drivers seed the spawn prompt.
                match payload.description.clone() {
                    Some(text) if !text.is_empty() => vec![AgentEvent::Subagent {
                        parent_tool_use_id: parent,
                        event: Box::new(AgentEvent::UserMessage { text }),
                    }],
                    _ => Vec::new(),
                }
            }
            Status::Completed | Status::Failed | Status::Aborted => {
                let Some(route) = self.subagents.remove(&payload.id) else {
                    return Vec::new();
                };
                let (status, error) = match payload.status {
                    Status::Completed => (DoneStatus::Completed, None),
                    Status::Failed => (
                        DoneStatus::Errored,
                        Some(format!("subagent {} failed", payload.agent)),
                    ),
                    _ => (DoneStatus::Interrupted, None),
                };
                vec![AgentEvent::Subagent {
                    parent_tool_use_id: route.parent_tool_call_id,
                    event: Box::new(AgentEvent::Done {
                        status,
                        result: None,
                        error,
                        session_id: Some(payload.id.clone()),
                    }),
                }]
            }
        }
    }
}

fn structured(kind: &str, payload: &impl serde::Serialize) -> Vec<AgentEvent> {
    vec![AgentEvent::OmpStructured {
        kind: kind.to_owned(),
        native: serde_json::to_value(payload).unwrap_or(Value::Null),
    }]
}

/// Extract an extension-UI request from a client event, if it is one.
pub fn extension_ui_request(event: &Event) -> Option<&ExtensionUiRequest> {
    match event {
        Event::Notification(RpcNotification::ExtensionUiRequest(request)) => Some(request),
        _ => None,
    }
}

fn index(content_index: Option<i64>) -> usize {
    content_index.filter(|index| *index >= 0).map_or(0, |index| index as usize)
}

fn count(tokens: Option<i64>) -> u64 {
    tokens.filter(|tokens| *tokens > 0).map_or(0, |tokens| tokens as u64)
}

/// Flatten a todo result's phased snapshot (`details.phases`) into
/// projection items, preserving phase name, status, and blocker.
fn todo_items(result: &Value) -> Option<Vec<zeron_proto::TodoItem>> {
    let phases = result["details"]["phases"].as_array()?;
    let mut items = Vec::new();
    for phase in phases {
        let phase_name = phase["name"].as_str().unwrap_or_default();
        for task in phase["tasks"].as_array().into_iter().flatten() {
            let status = zeron_proto::TodoStatus::parse(string(task, "status"));
            let mut item = zeron_proto::TodoItem::new(string(task, "content"), status);
            item.phase = (!phase_name.is_empty()).then(|| phase_name.to_owned());
            item.blocker = task["blocker"].as_str().map(str::to_owned);
            items.push(item);
        }
    }
    Some(items)
}

/// Join the text blocks of a tool result value (`{content:[{text}…]}`),
/// falling back to a plain string result.
fn result_text(result: &Value) -> Option<String> {
    if let Some(text) = result.as_str() {
        return Some(text.to_owned());
    }
    let items = result["content"].as_array()?;
    let text: Vec<&str> = items.iter().filter_map(|item| item["text"].as_str()).collect();
    if text.is_empty() {
        None
    } else {
        Some(text.join("\n"))
    }
}

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

fn cap(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    let mut at = limit;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    format!("{}\n… [truncated]", &text[..at])
}

/// Known OMP tool names reduced to renderable calls; everything else keeps
/// its full input as `Unknown`. MCP tools ride OMP's own MCP naming, which
/// is not second-guessed here.
fn tool_call(name: &str, args: &Value) -> ToolCall {
    match name {
        "bash" => ToolCall::Exec {
            command: string(args, "command").into(),
        },
        "read" => ToolCall::ReadFile {
            path: string(args, "path").into(),
        },
        "write" => ToolCall::WriteFile {
            path: string(args, "path").into(),
            content: args["content"].as_str().map(str::to_owned),
        },
        "edit" => ToolCall::EditFile {
            path: string(args, "path").into(),
            old_string: args["oldText"].as_str().map(str::to_owned),
            new_string: args["newText"].as_str().map(str::to_owned),
        },
        "grep" => ToolCall::Search {
            pattern: string(args, "pattern").into(),
            path: args["path"].as_str().map(str::to_owned),
        },
        "find" | "glob" => ToolCall::Glob {
            pattern: string(args, "pattern").into(),
        },
        "web_search" => ToolCall::WebSearch {
            query: string(args, "query").into(),
        },
        "fetch" | "web_fetch" => ToolCall::WebFetch {
            url: string(args, "url").into(),
            prompt: args["prompt"].as_str().map(str::to_owned),
        },
        _ => ToolCall::Unknown {
            name: name.into(),
            input: Some(args.clone()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omp_rpc::wire::{ToolExecutionEndEvent, ToolExecutionStartEvent};
    use serde_json::json;

    fn start(id: &str, name: &str, args: Value) -> Event {
        Event::Notification(RpcNotification::RpcAgentEvent(
            RpcAgentEvent::ToolExecutionStart(ToolExecutionStartEvent {
                tool_call_id: id.into(),
                tool_name: name.into(),
                args: Some(args),
                intent: None,
            }),
        ))
    }

    #[test]
    fn edit_tool_round_trips_identity_and_diff() {
        let mut normalizer = Normalizer::default();
        let events = normalizer.map(&start(
            "t1",
            "edit",
            json!({"path": "a.rs", "oldText": "old", "newText": "new"}),
        ));
        assert!(matches!(
            &events[0],
            AgentEvent::ToolCall { id, call: ToolCall::EditFile { .. } } if id == "t1"
        ));
        let end = Event::Notification(RpcNotification::RpcAgentEvent(
            RpcAgentEvent::ToolExecutionEnd(ToolExecutionEndEvent {
                tool_call_id: "t1".into(),
                tool_name: "edit".into(),
                result: Some(json!({"content": [{"type": "text", "text": "done"}]})),
                is_error: Some(false),
            }),
        ));
        let events = normalizer.map(&end);
        match &events[0] {
            AgentEvent::ToolResult { output, diff, .. } => {
                assert_eq!(output.as_deref(), Some("done"));
                let diff = diff.as_ref().expect("edit diff");
                assert_eq!(diff.path, "a.rs");
                assert_eq!(diff.old_text.as_deref(), Some("old"));
                assert_eq!(diff.new_text, "new");
            }
            other => panic!("expected tool result, got {other:?}"),
        }
    }

    #[test]
    fn slash_command_projection_preserves_aliases_subcommands_source() {
        let wire: omp_rpc::wire::AvailableSlashCommand = serde_json::from_value(json!({
            "name": "review", "source": "mcp_prompt", "aliases": ["rv"],
            "description": "Review", "input": {"hint": "<pr>"},
            "subcommands": [{"name": "show", "usage": "[id]"}]
        }))
        .unwrap();
        let cmd = omp_slash_command(&wire);
        assert_eq!(cmd.aliases, vec!["rv".to_string()]);
        assert_eq!(cmd.source, "mcp_prompt");
        assert_eq!(cmd.subcommands[0].name, "show");
        assert_eq!(cmd.subcommands[0].usage.as_deref(), Some("[id]"));
        assert_eq!(cmd.to_legacy().name, "review");
    }

    #[test]
    fn unknown_tools_keep_full_input_and_duplicate_starts_are_dropped() {
        let mut normalizer = Normalizer::default();
        let first = start("t2", "custom_tool", json!({"secret_shape": [1, 2, 3]}));
        let events = normalizer.map(&first);
        match &events[0] {
            AgentEvent::ToolCall { call: ToolCall::Unknown { input, .. }, .. } => {
                assert_eq!(input.as_ref().unwrap()["secret_shape"], json!([1, 2, 3]));
            }
            other => panic!("expected unknown tool call, got {other:?}"),
        }
        assert!(normalizer.map(&first).is_empty(), "duplicate start dropped");
    }

    #[test]
    fn final_message_reconciles_missing_delta_suffix_and_reports_usage() {
        use omp_rpc::wire::{
            AssistantMessage, AssistantTextDeltaEvent, MessageEndEvent, MessageUpdateEvent,
            TextContent, Usage,
        };
        let mut normalizer = Normalizer::default();
        normalizer.set_window(Some(200_000));
        let delta = Event::Notification(RpcNotification::RpcAgentEvent(
            RpcAgentEvent::MessageUpdate(MessageUpdateEvent {
                message: AgentMessage::Assistant(AssistantMessage::default()),
                assistant_message_event: AssistantMessageEvent::TextDelta(
                    AssistantTextDeltaEvent {
                        content_index: Some(0),
                        delta: Some("hel".into()),
                        partial: None,
                        extra: Default::default(),
                    },
                ),
                message_id: None,
            }),
        ));
        normalizer.map(&delta);
        let end = Event::Notification(RpcNotification::RpcAgentEvent(
            RpcAgentEvent::MessageEnd(MessageEndEvent {
                message: AgentMessage::Assistant(AssistantMessage {
                    content: Some(vec![AssistantContent::Text(TextContent {
                        text: Some("hello".into()),
                        ..Default::default()
                    })]),
                    usage: Some(Usage {
                        input: Some(10),
                        cache_read: Some(5),
                        output: Some(3),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                message_id: None,
            }),
        ));
        let events = normalizer.map(&end);
        assert!(matches!(&events[0], AgentEvent::TextDelta { text } if text == "lo"));
        assert!(matches!(
            &events[1],
            AgentEvent::Usage { input_tokens: 15, output_tokens: 3 }
        ));
        assert!(matches!(
            &events[2],
            AgentEvent::ContextUsage { tokens: Some(18), window: Some(200_000) }
        ));
    }

    #[test]
    fn subagent_events_route_by_lifecycle_and_wrap_nested() {
        use omp_rpc::wire::{
            AgentSource, AssistantMessage, AssistantTextDeltaEvent, MessageUpdateEvent,
            OrUnknown, SubagentEvent, SubagentEventPayload, SubagentLifecycleEvent,
            SubagentLifecyclePayload, SubagentLifecycleStatus,
        };
        let mut normalizer = Normalizer::default();
        let lifecycle = |status| {
            Event::Notification(RpcNotification::SubagentLifecycle(SubagentLifecycleEvent {
                payload: SubagentLifecyclePayload {
                    id: "sub1".into(),
                    agent: "scout".into(),
                    agent_source: AgentSource::Bundled,
                    status,
                    index: 0,
                    description: Some("scan the repo".into()),
                    session_file: None,
                    parent_tool_call_id: Some("spawn1".into()),
                    detached: None,
                },
            }))
        };
        // Nested traffic before lifecycle is unroutable and dropped.
        let nested = |text: &str| {
            Event::Notification(RpcNotification::SubagentEvent(SubagentEvent {
                payload: SubagentEventPayload {
                    id: "sub1".into(),
                    event: OrUnknown::Known(RpcAgentEvent::MessageUpdate(MessageUpdateEvent {
                        message: AgentMessage::Assistant(AssistantMessage::default()),
                        assistant_message_event: AssistantMessageEvent::TextDelta(
                            AssistantTextDeltaEvent {
                                content_index: Some(0),
                                delta: Some(text.into()),
                                partial: None,
                                extra: Default::default(),
                            },
                        ),
                        message_id: None,
                    })),
                },
            }))
        };
        assert!(normalizer.map(&nested("early")).is_empty());
        let started = normalizer.map(&lifecycle(SubagentLifecycleStatus::Started));
        assert!(matches!(
            &started[0],
            AgentEvent::Subagent { parent_tool_use_id, event }
                if parent_tool_use_id == "spawn1"
                    && matches!(event.as_ref(), AgentEvent::UserMessage { text } if text == "scan the repo")
        ));
        let events = normalizer.map(&nested("working"));
        assert!(matches!(
            &events[0],
            AgentEvent::Subagent { parent_tool_use_id, event }
                if parent_tool_use_id == "spawn1"
                    && matches!(event.as_ref(), AgentEvent::TextDelta { text } if text == "working")
        ));
        let done = normalizer.map(&lifecycle(SubagentLifecycleStatus::Completed));
        assert!(matches!(
            &done[0],
            AgentEvent::Subagent { event, .. }
                if matches!(event.as_ref(), AgentEvent::Done { status: DoneStatus::Completed, .. })
        ));
        // Route removed at completion.
        assert!(normalizer.map(&nested("late")).is_empty());
    }

    #[test]
    fn todo_result_upgrades_call_with_phased_snapshot() {
        let mut normalizer = Normalizer::default();
        normalizer.map(&start("td1", "todo", json!({"op": "init"})));
        let end = Event::Notification(RpcNotification::RpcAgentEvent(
            RpcAgentEvent::ToolExecutionEnd(ToolExecutionEndEvent {
                tool_call_id: "td1".into(),
                tool_name: "todo".into(),
                result: Some(json!({
                    "content": [{"type": "text", "text": "2 phases"}],
                    "details": {"phases": [
                        {"name": "Build", "tasks": [
                            {"content": "compile", "status": "completed"},
                            {"content": "test", "status": "blocked", "blocker": "CI down"},
                        ]},
                        {"name": "Ship", "tasks": [
                            {"content": "release", "status": "abandoned"},
                        ]},
                    ]},
                })),
                is_error: Some(false),
            }),
        ));
        let events = normalizer.map(&end);
        assert_eq!(events.len(), 2);
        let AgentEvent::ToolCall { id, call: ToolCall::Todo { items } } = &events[1] else {
            panic!("expected todo upgrade, got {:?}", events[1]);
        };
        assert_eq!(id, "td1");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].status(), zeron_proto::TodoStatus::Completed);
        assert_eq!(items[0].phase.as_deref(), Some("Build"));
        assert_eq!(items[1].status(), zeron_proto::TodoStatus::Blocked);
        assert_eq!(items[1].blocker.as_deref(), Some("CI down"));
        assert_eq!(items[2].status(), zeron_proto::TodoStatus::Abandoned);
        assert_eq!(items[2].phase.as_deref(), Some("Ship"));
    }
}
