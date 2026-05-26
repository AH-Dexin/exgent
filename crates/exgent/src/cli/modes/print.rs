use std::io::{self, IsTerminal, Read, Write};

use exgent_core::{AgentEvent, AgentSessionEvent, AppRuntimeHost};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrintMode {
    prompt: Option<String>,
    output: PrintOutputMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PrintOutputMode {
    #[default]
    Text,
    Json,
}

impl PrintMode {
    pub fn new(prompt: Option<String>) -> Self {
        Self {
            prompt,
            output: PrintOutputMode::Text,
        }
    }

    pub fn json(prompt: Option<String>) -> Self {
        Self {
            prompt,
            output: PrintOutputMode::Json,
        }
    }

    pub fn run(self, runtime: &mut AppRuntimeHost) -> io::Result<()> {
        let prompt = prompt_or_stdin(self.prompt)?;
        let mut renderer = PrintRenderer::new(self.output);

        runtime
            .run_prompt_events(&prompt, &mut |event| renderer.render_session_event(event))
            .map_err(io::Error::other)?;
        renderer.finish()
    }
}

fn prompt_or_stdin(prompt: Option<String>) -> io::Result<String> {
    if let Some(prompt) = prompt {
        if prompt.trim().is_empty() {
            return Err(missing_prompt_error());
        }
        return Ok(prompt);
    }

    if io::stdin().is_terminal() {
        return Err(missing_prompt_error());
    }

    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let prompt = input.trim_end_matches(['\r', '\n']).to_string();
    if prompt.trim().is_empty() {
        return Err(missing_prompt_error());
    }
    Ok(prompt)
}

fn missing_prompt_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "print mode requires a prompt argument or stdin input",
    )
}

#[derive(Default)]
struct PrintRenderer {
    output: PrintOutputMode,
    message_has_delta: bool,
    wrote_output: bool,
}

impl PrintRenderer {
    fn new(output: PrintOutputMode) -> Self {
        Self {
            output,
            ..Self::default()
        }
    }

    fn render_session_event(&mut self, event: AgentSessionEvent) {
        if self.output == PrintOutputMode::Json {
            self.render_json(event);
            return;
        }

        if let AgentSessionEvent::Agent(event) = event {
            self.render(event);
        }
    }

    fn render(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::MessageStart { .. } => {
                self.message_has_delta = false;
            }
            AgentEvent::MessageDelta { delta } => {
                self.message_has_delta = true;
                self.wrote_output = true;
                print!("{delta}");
                let _ = io::stdout().flush();
            }
            AgentEvent::MessageEnd { content } => {
                if !self.message_has_delta && !content.is_empty() {
                    self.wrote_output = true;
                    print!("{content}");
                    let _ = io::stdout().flush();
                }
            }
            AgentEvent::ToolCallStart { name, .. } => {
                eprintln!("tool: {name}");
            }
            AgentEvent::ToolCallEnd {
                name,
                content,
                is_error,
                ..
            } => {
                let status = if is_error { "error" } else { "ok" };
                eprintln!("tool: {name} {status}");
                if is_error && !content.is_empty() {
                    eprintln!("{content}");
                }
            }
            AgentEvent::Error { .. }
            | AgentEvent::AssistantToolCalls { .. }
            | AgentEvent::AgentStart
            | AgentEvent::ReasoningDelta { .. }
            | AgentEvent::Usage { .. }
            | AgentEvent::AgentEnd => {}
        }
    }

    fn finish(self) -> io::Result<()> {
        if self.output == PrintOutputMode::Text && self.wrote_output {
            println!();
        }
        io::stdout().flush()
    }

    fn render_json(&mut self, event: AgentSessionEvent) {
        let value = match event {
            AgentSessionEvent::Agent(event) => json_agent_event(event),
            AgentSessionEvent::UsageUpdated(usage) => serde_json::json!({
                "type": "usage_updated",
                "usage": {
                    "input": usage.input,
                    "output": usage.output,
                    "cache_read": usage.cache_read,
                    "cache_write": usage.cache_write,
                    "cost": usage.cost,
                }
            }),
            AgentSessionEvent::CompactionStarted { message_count } => serde_json::json!({
                "type": "compaction_started",
                "message_count": message_count,
            }),
            AgentSessionEvent::CompactionFinished { compacted_count } => serde_json::json!({
                "type": "compaction_finished",
                "compacted_count": compacted_count,
            }),
            AgentSessionEvent::TurnCommitted { message_count } => serde_json::json!({
                "type": "turn_committed",
                "message_count": message_count,
            }),
            AgentSessionEvent::TurnTelemetry(telemetry) => serde_json::json!({
                "type": "turn_telemetry",
                "duration_ms": telemetry.duration_ms,
                "tool_calls": telemetry.tool_calls,
                "input_tokens": telemetry.input_tokens,
                "output_tokens": telemetry.output_tokens,
                "cache_read_tokens": telemetry.cache_read_tokens,
                "cache_write_tokens": telemetry.cache_write_tokens,
                "cost": telemetry.cost,
                "errored": telemetry.errored,
            }),
        };
        println!("{value}");
        self.wrote_output = true;
    }
}

fn json_agent_event(event: AgentEvent) -> serde_json::Value {
    match event {
        AgentEvent::AgentStart => serde_json::json!({ "type": "agent_start" }),
        AgentEvent::MessageStart { role } => serde_json::json!({
            "type": "message_start",
            "role": role,
        }),
        AgentEvent::MessageDelta { delta } => serde_json::json!({
            "type": "message_delta",
            "delta": delta,
        }),
        AgentEvent::ReasoningDelta { delta } => serde_json::json!({
            "type": "reasoning_delta",
            "delta": delta,
        }),
        AgentEvent::Usage { usage } => serde_json::json!({
            "type": "usage",
            "usage": usage,
        }),
        AgentEvent::MessageEnd { content } => serde_json::json!({
            "type": "message_end",
            "content": content,
        }),
        AgentEvent::AssistantToolCalls { calls } => serde_json::json!({
            "type": "assistant_tool_calls",
            "calls": calls,
        }),
        AgentEvent::ToolCallStart {
            id,
            name,
            arguments,
        } => serde_json::json!({
            "type": "tool_call_start",
            "id": id,
            "name": name,
            "arguments": arguments,
        }),
        AgentEvent::ToolCallEnd {
            id,
            name,
            content,
            is_error,
        } => serde_json::json!({
            "type": "tool_call_end",
            "id": id,
            "name": name,
            "content": content,
            "is_error": is_error,
        }),
        AgentEvent::AgentEnd => serde_json::json!({ "type": "agent_end" }),
        AgentEvent::Error { message } => serde_json::json!({
            "type": "error",
            "message": message,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_prompt_argument() {
        assert_eq!(
            prompt_or_stdin(Some("hello".to_string())).unwrap(),
            "hello".to_string()
        );
    }

    #[test]
    fn rejects_empty_prompt_argument() {
        assert!(prompt_or_stdin(Some("  ".to_string())).is_err());
    }
}
