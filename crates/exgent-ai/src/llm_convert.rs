use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{DefaultHasher, Hash, Hasher},
};

use serde::Serialize;

use crate::{ChatMessage, MessageRole, ToolArguments, ToolCall};

#[derive(Debug, Serialize)]
pub(crate) struct OpenAiChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<OpenAiChatToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenAiChatToolCall {
    id: String,
    #[serde(rename = "type")]
    tool_type: &'static str,
    function: OpenAiChatToolCallFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiChatToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct AnthropicMessage {
    role: String,
    content: Vec<serde_json::Value>,
}

pub(crate) fn openai_chat_messages(messages: &[ChatMessage]) -> Vec<OpenAiChatMessage> {
    let mut pending_tool_call_ids = Vec::new();
    messages
        .iter()
        .flat_map(|message| {
            if !message.tool_calls.is_empty() {
                pending_tool_call_ids.extend(message.tool_calls.iter().map(|call| call.id.clone()));
                return vec![OpenAiChatMessage {
                    role: "assistant".to_string(),
                    content: non_empty_content(&message.content),
                    tool_calls: message
                        .tool_calls
                        .iter()
                        .map(openai_chat_tool_call)
                        .collect(),
                    tool_call_id: None,
                }];
            }

            if message.role == MessageRole::Tool {
                if let Some(result) = tool_result_view(message) {
                    if pending_tool_call_ids
                        .iter()
                        .any(|id| id == &result.tool_call_id)
                    {
                        pending_tool_call_ids.retain(|id| id != &result.tool_call_id);
                        return vec![OpenAiChatMessage {
                            role: "tool".to_string(),
                            content: Some(result.content),
                            tool_calls: Vec::new(),
                            tool_call_id: Some(result.tool_call_id),
                        }];
                    }
                }

                return vec![OpenAiChatMessage {
                    role: "user".to_string(),
                    content: Some(llm_message_content(message)),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                }];
            }

            vec![OpenAiChatMessage {
                role: openai_role(&message.role).to_string(),
                content: Some(message.content.clone()),
                tool_calls: Vec::new(),
                tool_call_id: None,
            }]
        })
        .collect()
}

pub(crate) fn openai_responses_input(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .flat_map(|message| {
            if !message.tool_calls.is_empty() {
                return message
                    .tool_calls
                    .iter()
                    .map(|call| {
                        serde_json::json!({
                            "type": "function_call",
                            "call_id": call.id.clone(),
                            "name": call.name.clone(),
                            "arguments": tool_arguments_json(&call.arguments).to_string(),
                        })
                    })
                    .collect::<Vec<_>>();
            }

            if message.role == MessageRole::Tool {
                if let Some(result) = tool_result_view(message) {
                    return vec![serde_json::json!({
                        "type": "function_call_output",
                        "call_id": result.tool_call_id,
                        "output": result.content,
                    })];
                }
            }

            vec![serde_json::json!({
                "role": openai_responses_role(&message.role),
                "content": llm_message_content(message),
            })]
        })
        .collect()
}

pub(crate) fn anthropic_messages(messages: &[ChatMessage]) -> Vec<AnthropicMessage> {
    normalize_anthropic_tool_history(messages)
        .iter()
        .filter_map(anthropic_message)
        .collect()
}

fn openai_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        MessageRole::System => "system",
    }
}

fn openai_chat_tool_call(call: &ToolCall) -> OpenAiChatToolCall {
    OpenAiChatToolCall {
        id: call.id.clone(),
        tool_type: "function",
        function: OpenAiChatToolCallFunction {
            name: call.name.clone(),
            arguments: tool_arguments_json(&call.arguments).to_string(),
        },
    }
}

fn openai_responses_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "user",
    }
}

fn anthropic_message(message: &ChatMessage) -> Option<AnthropicMessage> {
    match message.role {
        MessageRole::System => None,
        MessageRole::User => Some(AnthropicMessage {
            role: "user".to_string(),
            content: vec![text_block(&message.content)],
        }),
        MessageRole::Assistant => {
            let mut content = Vec::new();
            if !message.content.is_empty() {
                content.push(text_block(&message.content));
            }
            content.extend(message.tool_calls.iter().map(anthropic_tool_use_block));
            if content.is_empty() {
                return None;
            }
            Some(AnthropicMessage {
                role: "assistant".to_string(),
                content,
            })
        }
        MessageRole::Tool => {
            if let Some(result) = tool_result_view(message) {
                let mut block = serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": result.tool_call_id,
                    "content": result.content,
                });
                if result.is_error {
                    block["is_error"] = serde_json::Value::Bool(true);
                }
                return Some(AnthropicMessage {
                    role: "user".to_string(),
                    content: vec![block],
                });
            }

            Some(AnthropicMessage {
                role: "user".to_string(),
                content: vec![text_block(&llm_message_content(message))],
            })
        }
    }
}

fn anthropic_tool_use_block(call: &ToolCall) -> serde_json::Value {
    serde_json::json!({
        "type": "tool_use",
        "id": call.id.clone(),
        "name": call.name.clone(),
        "input": tool_arguments_json(&call.arguments),
    })
}

fn text_block(text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "text",
        "text": text,
    })
}

fn llm_message_content(message: &ChatMessage) -> String {
    match message.role {
        MessageRole::Tool => format!("Tool result:\n{}", message.content),
        _ => message.content.clone(),
    }
}

struct ToolResultView {
    tool_call_id: String,
    content: String,
    is_error: bool,
}

fn tool_result_view(message: &ChatMessage) -> Option<ToolResultView> {
    message
        .tool_call_id
        .as_ref()
        .map(|tool_call_id| ToolResultView {
            tool_call_id: tool_call_id.clone(),
            content: message.content.clone(),
            is_error: message.tool_is_error.unwrap_or(false),
        })
        .or_else(|| legacy_tool_result_view(&message.content))
}

fn legacy_tool_result_view(content: &str) -> Option<ToolResultView> {
    let mut lines = content.lines();
    let _tool_name = lines.next()?.strip_prefix("tool: ")?;
    let tool_call_id = lines.next()?.strip_prefix("id: ")?.to_string();
    let is_error = lines
        .next()?
        .strip_prefix("is_error: ")?
        .parse::<bool>()
        .ok()?;
    if lines.next()? != "content:" {
        return None;
    }
    Some(ToolResultView {
        tool_call_id,
        content: lines.collect::<Vec<_>>().join("\n"),
        is_error,
    })
}

fn non_empty_content(content: &str) -> Option<String> {
    (!content.is_empty()).then(|| content.to_string())
}

fn tool_arguments_json(arguments: &ToolArguments) -> serde_json::Value {
    serde_json::to_value(arguments)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
}

fn normalize_anthropic_tool_history(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut id_map = BTreeMap::<String, String>::new();
    let mut result_ids = BTreeSet::<String>::new();
    let mut pending_calls = Vec::<ToolCall>::new();
    let mut output = Vec::new();

    for message in messages {
        let mut message = message.clone();
        for call in &mut message.tool_calls {
            let normalized = normalize_anthropic_tool_call_id(&call.id);
            if normalized != call.id {
                id_map.insert(call.id.clone(), normalized.clone());
                call.id = normalized;
            }
            pending_calls.push(call.clone());
        }

        if message.role == MessageRole::Tool {
            if let Some(id) = message.tool_call_id.clone() {
                if let Some(normalized) = id_map.get(&id) {
                    message.tool_call_id = Some(normalized.clone());
                }
            }
            if let Some(id) = &message.tool_call_id {
                result_ids.insert(id.clone());
            }
        }

        output.push(message);
    }

    for call in pending_calls {
        if !result_ids.contains(&call.id) {
            output.push(ChatMessage::tool_result(
                call.id,
                call.name,
                "No result provided",
                true,
            ));
        }
    }

    output
}

fn normalize_anthropic_tool_call_id(id: &str) -> String {
    let mut normalized = id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if normalized.is_empty() {
        normalized = "tool_call".to_string();
    }

    if normalized.len() <= 64 {
        return normalized;
    }

    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    let suffix = format!("{:016x}", hasher.finish());
    let prefix_len = 64usize.saturating_sub(suffix.len() + 1);
    format!("{}_{}", &normalized[..prefix_len], suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_tool_history_for_openai_chat() {
        let messages = vec![
            ChatMessage::user("inspect"),
            ChatMessage::assistant_tool_call(
                ToolCall::new("call_1", "read").with_argument("path", "Cargo.toml"),
            ),
            ChatMessage::tool_result("call_1", "read", "workspace manifest", false),
        ];

        let value = serde_json::to_value(openai_chat_messages(&messages)).unwrap();

        assert_eq!(
            value,
            serde_json::json!([
                {"role":"user","content":"inspect"},
                {
                    "role":"assistant",
                    "tool_calls":[{
                        "id":"call_1",
                        "type":"function",
                        "function":{
                            "name":"read",
                            "arguments":"{\"path\":\"Cargo.toml\"}"
                        }
                    }]
                },
                {
                    "role":"tool",
                    "content":"workspace manifest",
                    "tool_call_id":"call_1"
                }
            ])
        );
    }

    #[test]
    fn converts_tool_history_for_openai_responses() {
        let messages = vec![
            ChatMessage::assistant_tool_call(
                ToolCall::new("call_1", "bash").with_argument("command", "cargo check"),
            ),
            ChatMessage::tool_result("call_1", "bash", "exit_code: 0", false),
        ];

        assert_eq!(
            openai_responses_input(&messages),
            vec![
                serde_json::json!({
                    "type":"function_call",
                    "call_id":"call_1",
                    "name":"bash",
                    "arguments":"{\"command\":\"cargo check\"}"
                }),
                serde_json::json!({
                    "type":"function_call_output",
                    "call_id":"call_1",
                    "output":"exit_code: 0"
                })
            ]
        );
    }

    #[test]
    fn converts_tool_history_for_anthropic_messages() {
        let messages = vec![
            ChatMessage::assistant_tool_call(
                ToolCall::new("toolu_1", "read").with_argument("path", "Cargo.toml"),
            ),
            ChatMessage::tool_result("toolu_1", "read", "workspace manifest", false),
        ];

        let value = serde_json::to_value(anthropic_messages(&messages)).unwrap();

        assert_eq!(
            value,
            serde_json::json!([
                {
                    "role":"assistant",
                    "content":[{
                        "type":"tool_use",
                        "id":"toolu_1",
                        "name":"read",
                        "input":{"path":"Cargo.toml"}
                    }]
                },
                {
                    "role":"user",
                    "content":[{
                        "type":"tool_result",
                        "tool_use_id":"toolu_1",
                        "content":"workspace manifest"
                    }]
                }
            ])
        );
    }

    #[test]
    fn anthropic_conversion_normalizes_tool_ids_and_repairs_missing_results() {
        let long_id = format!("{}|{}", "x".repeat(80), "unsafe");
        let messages = vec![ChatMessage::assistant_tool_call(ToolCall::new(
            long_id, "read",
        ))];

        let value = serde_json::to_value(anthropic_messages(&messages)).unwrap();
        let normalized_id = value[0]["content"][0]["id"].as_str().unwrap();

        assert!(normalized_id.len() <= 64);
        assert!(normalized_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'));
        assert_eq!(
            value[1],
            serde_json::json!({
                "role":"user",
                "content":[{
                    "type":"tool_result",
                    "tool_use_id": normalized_id,
                    "content":"No result provided",
                    "is_error": true
                }]
            })
        );
    }
}
