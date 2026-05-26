use std::{
    collections::{BTreeMap, VecDeque},
    thread,
};

use exgent_ai::{
    ChatMessage, Model, ProviderAdapter, ProviderEvent, ProviderRequest, ToolCall,
    ToolExecutionMode,
};

use super::{AgentEvent, ToolExecutionResult, ToolExecutor};

const MAX_TOOL_ROUNDS: usize = 8;

pub(crate) fn run_messages_with_tools_streaming<P, T, F>(
    model: &Model,
    provider: &P,
    mut messages: Vec<ChatMessage>,
    tools: &T,
    emit: &mut F,
) where
    P: ProviderAdapter,
    T: ToolExecutor,
    F: FnMut(AgentEvent),
{
    emit(AgentEvent::AgentStart);

    for _ in 0..MAX_TOOL_ROUNDS {
        let tool_definitions = tools.tool_definitions();
        let tool_modes = tool_definitions
            .iter()
            .map(|definition| (definition.name.clone(), definition.execution_mode))
            .collect::<BTreeMap<_, _>>();
        let request = ProviderRequest {
            model: model.clone(),
            messages: messages.clone(),
            tools: tool_definitions,
        };

        let turn = stream_assistant_turn(provider, request, emit);
        if turn.failed {
            emit(AgentEvent::AgentEnd);
            return;
        }

        if !turn.tool_calls.is_empty() {
            emit(AgentEvent::AssistantToolCalls {
                calls: turn.tool_calls.clone(),
            });
        }
        messages.extend(turn.assistant_messages());

        if turn.tool_calls.is_empty() {
            emit(AgentEvent::AgentEnd);
            return;
        }

        let tool_results = execute_tool_batch(&turn.tool_calls, &tool_modes, tools, emit);
        messages.extend(
            tool_results
                .into_iter()
                .map(|ExecutedToolResult { call, result }| {
                    ChatMessage::tool_result(call.id, call.name, result.content, result.is_error)
                }),
        );
    }

    emit(AgentEvent::Error {
        message: "maximum tool rounds reached".to_string(),
    });
    emit(AgentEvent::AgentEnd);
}

#[derive(Default)]
struct AssistantTurn {
    content: String,
    tool_calls: Vec<ToolCall>,
    failed: bool,
}

impl AssistantTurn {
    fn assistant_messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();
        if !self.content.is_empty() || !self.tool_calls.is_empty() {
            messages.push(ChatMessage::assistant_tool_calls(
                self.content.clone(),
                self.tool_calls.clone(),
            ));
        }
        messages
    }
}

fn stream_assistant_turn<P, F>(
    provider: &P,
    request: ProviderRequest,
    emit: &mut F,
) -> AssistantTurn
where
    P: ProviderAdapter,
    F: FnMut(AgentEvent),
{
    let mut turn = AssistantTurn::default();
    let mut message_started = false;
    let mut message_ended = false;

    provider.stream_events(request, &mut |event| {
        if turn.failed {
            return;
        }

        match event {
            ProviderEvent::Start => {
                message_started = true;
                emit(AgentEvent::MessageStart {
                    role: "assistant".to_string(),
                });
            }
            ProviderEvent::TextDelta(delta) => {
                turn.content.push_str(&delta);
                emit(AgentEvent::MessageDelta { delta });
            }
            ProviderEvent::ReasoningDelta(delta) => {
                emit(AgentEvent::ReasoningDelta { delta });
            }
            ProviderEvent::Usage(usage) => {
                emit(AgentEvent::Usage { usage });
            }
            ProviderEvent::ToolCall(call) => {
                turn.tool_calls.push(call);
            }
            ProviderEvent::Done(message) => {
                if turn.content.is_empty() {
                    turn.content = message.content;
                }
                if !turn.content.is_empty() {
                    message_ended = true;
                    emit(AgentEvent::MessageEnd {
                        content: turn.content.clone(),
                    });
                }
            }
            ProviderEvent::Error(message) => {
                turn.failed = true;
                emit(AgentEvent::Error { message });
            }
        }
    });

    if !turn.failed
        && !message_ended
        && (message_started || !turn.content.is_empty() || !turn.tool_calls.is_empty())
    {
        if !message_started {
            emit(AgentEvent::MessageStart {
                role: "assistant".to_string(),
            });
        }
        emit(AgentEvent::MessageEnd {
            content: turn.content.clone(),
        });
    }

    turn
}

struct ExecutedToolResult {
    call: ToolCall,
    result: ToolExecutionResult,
}

fn execute_tool_batch<T, F>(
    calls: &[ToolCall],
    tool_modes: &BTreeMap<String, ToolExecutionMode>,
    tools: &T,
    emit: &mut F,
) -> Vec<ExecutedToolResult>
where
    T: ToolExecutor,
    F: FnMut(AgentEvent),
{
    for call in calls {
        emit_tool_start(call, emit);
    }

    let prepared = calls
        .iter()
        .cloned()
        .map(|call| prepare_tool_call(call, tools))
        .collect::<Vec<_>>();
    let pending_calls = prepared
        .iter()
        .filter_map(|prepared| match prepared {
            PreparedToolCall::Pending(call) => Some(call.clone()),
            PreparedToolCall::Completed(_) => None,
        })
        .collect::<Vec<_>>();

    let pending_results = if should_execute_sequentially(&pending_calls, tool_modes) {
        execute_pending_tools_sequentially(&pending_calls, tools)
    } else {
        execute_pending_tools_in_parallel(&pending_calls, tools)
    };
    let mut pending_results = VecDeque::from(pending_results);
    let results = prepared
        .into_iter()
        .map(|prepared| match prepared {
            PreparedToolCall::Completed(result) => result,
            PreparedToolCall::Pending(_) => pending_results
                .pop_front()
                .expect("pending tool result count should match pending calls"),
        })
        .collect::<Vec<_>>();

    for ExecutedToolResult { call, result } in &results {
        emit(AgentEvent::ToolCallEnd {
            id: call.id.clone(),
            name: call.name.clone(),
            content: result.content.clone(),
            is_error: result.is_error,
        });
    }

    results
}

enum PreparedToolCall {
    Completed(ExecutedToolResult),
    Pending(ToolCall),
}

fn prepare_tool_call<T>(call: ToolCall, tools: &T) -> PreparedToolCall
where
    T: ToolExecutor,
{
    match tools.before_tool_call(&call) {
        Some(result) => PreparedToolCall::Completed(ExecutedToolResult {
            result: tools.after_tool_call(&call, result),
            call,
        }),
        None => PreparedToolCall::Pending(call),
    }
}

fn execute_pending_tools_sequentially<T>(calls: &[ToolCall], tools: &T) -> Vec<ExecutedToolResult>
where
    T: ToolExecutor,
{
    calls
        .iter()
        .cloned()
        .map(|call| ExecutedToolResult {
            result: execute_prepared_tool(&call, tools),
            call,
        })
        .collect()
}

fn execute_pending_tools_in_parallel<T>(calls: &[ToolCall], tools: &T) -> Vec<ExecutedToolResult>
where
    T: ToolExecutor,
{
    thread::scope(|scope| {
        calls
            .iter()
            .cloned()
            .map(|call| {
                let fallback_call = call.clone();
                let handle = scope.spawn(move || ExecutedToolResult {
                    result: execute_prepared_tool(&call, tools),
                    call,
                });
                (fallback_call, handle)
            })
            .map(|(fallback_call, handle)| {
                handle.join().unwrap_or_else(|_| ExecutedToolResult {
                    call: fallback_call,
                    result: ToolExecutionResult::error("tool execution panicked"),
                })
            })
            .collect()
    })
}

fn should_execute_sequentially(
    calls: &[ToolCall],
    tool_modes: &BTreeMap<String, ToolExecutionMode>,
) -> bool {
    calls.len() <= 1
        || calls.iter().any(|call| {
            tool_modes
                .get(&call.name)
                .copied()
                .unwrap_or(ToolExecutionMode::Sequential)
                == ToolExecutionMode::Sequential
        })
}

fn execute_prepared_tool<T>(call: &ToolCall, tools: &T) -> ToolExecutionResult
where
    T: ToolExecutor,
{
    let result = tools.execute_tool(call);
    tools.after_tool_call(call, result)
}

fn emit_tool_start<F>(call: &ToolCall, emit: &mut F)
where
    F: FnMut(AgentEvent),
{
    emit(AgentEvent::ToolCallStart {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
    });
}
