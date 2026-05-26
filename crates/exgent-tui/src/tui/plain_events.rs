use std::io::{self, Write};

use exgent_core::AgentEvent;

use super::{dim, thinking};

pub(crate) struct EventRenderer {
    show_prompt_details: bool,
    message_started: bool,
    reasoning_started: bool,
    reasoning_ended_with_newline: bool,
    leading_blank_available: bool,
}

impl EventRenderer {
    pub(crate) fn new(show_prompt_details: bool, leading_blank_printed: bool) -> Self {
        Self {
            show_prompt_details,
            message_started: false,
            reasoning_started: false,
            reasoning_ended_with_newline: false,
            leading_blank_available: leading_blank_printed,
        }
    }

    pub(crate) fn render(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::AgentStart => {}
            AgentEvent::MessageStart { .. } => {}
            AgentEvent::MessageDelta { delta } => {
                let finished_reasoning = self.finish_reasoning();
                if !self.message_started && !finished_reasoning && !self.leading_blank_available {
                    println!();
                }
                self.leading_blank_available = false;
                self.message_started = true;
                print!("{delta}");
                let _ = io::stdout().flush();
            }
            AgentEvent::ReasoningDelta { delta } => {
                if self.show_prompt_details {
                    if !self.reasoning_started && !self.leading_blank_available {
                        println!();
                    }
                    self.leading_blank_available = false;
                    self.reasoning_started = true;
                    self.reasoning_ended_with_newline = delta.ends_with('\n');
                    print!("{}", thinking(&delta));
                    let _ = io::stdout().flush();
                }
            }
            AgentEvent::MessageEnd { .. } => {
                self.finish_reasoning();
                if self.message_started {
                    println!("\n");
                    self.message_started = false;
                }
            }
            AgentEvent::AssistantToolCalls { .. } => {}
            AgentEvent::ToolCallStart {
                name, arguments, ..
            } => {
                self.finish_reasoning();
                self.leading_blank_available = false;
                println!("{} {name}: {arguments:?}", dim("tool"));
            }
            AgentEvent::ToolCallEnd {
                name,
                content,
                is_error,
                ..
            } => {
                self.finish_reasoning();
                self.leading_blank_available = false;
                let status = if is_error { "error" } else { "ok" };
                println!("{} {name} {status}:\n{content}", dim("tool"));
            }
            AgentEvent::AgentEnd => {}
            AgentEvent::Usage { .. } => {}
            AgentEvent::Error { message } => {
                self.finish_reasoning();
                self.leading_blank_available = false;
                eprintln!("error: {message}");
            }
        }
    }

    fn finish_reasoning(&mut self) -> bool {
        if self.reasoning_started {
            if !self.reasoning_ended_with_newline {
                println!();
            }
            self.reasoning_started = false;
            self.reasoning_ended_with_newline = false;
            return true;
        }
        false
    }
}

pub(crate) fn event_has_visible_output(event: &AgentEvent, show_prompt_details: bool) -> bool {
    match event {
        AgentEvent::MessageDelta { .. }
        | AgentEvent::MessageEnd { .. }
        | AgentEvent::ToolCallStart { .. }
        | AgentEvent::ToolCallEnd { .. }
        | AgentEvent::Error { .. } => true,
        AgentEvent::ReasoningDelta { .. } => show_prompt_details,
        AgentEvent::AgentStart
        | AgentEvent::MessageStart { .. }
        | AgentEvent::AssistantToolCalls { .. }
        | AgentEvent::Usage { .. }
        | AgentEvent::AgentEnd => false,
    }
}
