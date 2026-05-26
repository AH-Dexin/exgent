use std::io;

use crate::ai::ToolCall;
use serde_json::Value;

pub(super) fn required_argument<'a>(call: &'a ToolCall, name: &str) -> io::Result<&'a str> {
    call.arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| missing_required_argument(name))
}

pub(super) fn optional_usize_argument(call: &ToolCall, name: &str) -> io::Result<Option<usize>> {
    call.arguments
        .get(name)
        .map(|value| match value {
            Value::Number(number) => number
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid numeric argument {name}: {value}"),
                    )
                }),
            Value::String(value) => value.parse::<usize>().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid numeric argument {name}: {error}"),
                )
            }),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid numeric argument {name}: {value}"),
            )),
        })
        .transpose()
}

pub(super) fn optional_string_argument<'a>(
    call: &'a ToolCall,
    name: &str,
) -> io::Result<Option<&'a str>> {
    match call.arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(value.as_str()))
            }
        }
        Some(other) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid string argument {name}: {other}"),
        )),
    }
}

pub(super) fn optional_bool_argument(call: &ToolCall, name: &str) -> io::Result<Option<bool>> {
    call.arguments
        .get(name)
        .map(|value| match value {
            Value::Bool(value) => Ok(*value),
            Value::String(value) if value == "true" => Ok(true),
            Value::String(value) if value == "false" => Ok(false),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid boolean argument {name}: {value}"),
            )),
        })
        .transpose()
}

fn missing_required_argument(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("missing required argument: {name}"),
    )
}
