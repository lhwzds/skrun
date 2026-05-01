use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read;

#[derive(Debug, Deserialize)]
struct MatchInput {
    pattern: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct SkillError {
    code: String,
    message: String,
    retryable: bool,
}

#[derive(Debug, Serialize)]
struct SkillResponse {
    ok: bool,
    action: String,
    data: Option<Value>,
    error: Option<SkillError>,
}

fn main() {
    match run() {
        Ok(response) => print_response(&response),
        Err(response) => {
            print_response(&response);
            std::process::exit(1);
        }
    }
}

fn run() -> Result<SkillResponse, SkillResponse> {
    if std::env::args().len() > 1 {
        return run_cli();
    }

    let mut stdin = String::new();
    std::io::stdin().read_to_string(&mut stdin).map_err(|err| {
        error_response("read_stdin_failed", format!("Failed to read stdin: {err}"))
    })?;
    if stdin.trim().is_empty() {
        return Err(error_response(
            "empty_input",
            "Expected a JSON request on stdin or CLI arguments".to_string(),
        ));
    }

    let value: Value = serde_json::from_str(&stdin)
        .map_err(|err| error_response("invalid_json", format!("Failed to parse JSON: {err}")))?;
    run_json(value)
}

fn run_cli() -> Result<SkillResponse, SkillResponse> {
    let mut pattern = None;
    let mut text = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pattern" => pattern = args.next(),
            "--text" => text = args.next(),
            "--help" | "-h" => {
                return Ok(success_response(
                    "help",
                    json!({
                        "usage": "regex-finder --pattern <regex> --text <text>",
                        "json_usage": "{\"action\":\"match\",\"input\":{\"pattern\":\"\\\\d+\",\"text\":\"abc 123\"}}"
                    }),
                ));
            }
            other => {
                return Err(error_response(
                    "unknown_argument",
                    format!("Unknown argument: {other}"),
                ));
            }
        }
    }

    run_match(MatchInput {
        pattern: pattern
            .ok_or_else(|| error_response("missing_pattern", "Missing --pattern".to_string()))?,
        text: text.ok_or_else(|| error_response("missing_text", "Missing --text".to_string()))?,
    })
}

fn run_json(value: Value) -> Result<SkillResponse, SkillResponse> {
    let action = value
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("match");
    match action {
        "describe" => Ok(success_response(
            "describe",
            json!({
                "id": "regex-finder",
                "actions": ["describe", "match"],
                "protocol": "stdio-json"
            }),
        )),
        "match" => {
            let input_value = value.get("input").cloned().unwrap_or(value);
            let input: MatchInput = serde_json::from_value(input_value).map_err(|err| {
                error_response("invalid_input", format!("Invalid match input: {err}"))
            })?;
            run_match(input)
        }
        other => Err(error_response(
            "unsupported_action",
            format!("Unsupported action: {other}"),
        )),
    }
}

fn run_match(input: MatchInput) -> Result<SkillResponse, SkillResponse> {
    let regex = Regex::new(&input.pattern)
        .map_err(|err| error_response("invalid_regex", format!("Invalid regex pattern: {err}")))?;
    Ok(success_response(
        "match",
        json!({
            "matched": regex.is_match(&input.text)
        }),
    ))
}

fn success_response(action: impl Into<String>, data: Value) -> SkillResponse {
    SkillResponse {
        ok: true,
        action: action.into(),
        data: Some(data),
        error: None,
    }
}

fn error_response(code: impl Into<String>, message: String) -> SkillResponse {
    SkillResponse {
        ok: false,
        action: "error".to_string(),
        data: None,
        error: Some(SkillError {
            code: code.into(),
            message,
            retryable: false,
        }),
    }
}

fn print_response(response: &SkillResponse) {
    println!(
        "{}",
        serde_json::to_string(response).unwrap_or_else(|_| {
            "{\"ok\":false,\"action\":\"error\",\"data\":null,\"error\":{\"code\":\"serialize_failed\",\"message\":\"Failed to serialize response\",\"retryable\":false}}".to_string()
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_json_shape_still_matches() {
        let response = run_json(json!({
            "pattern": "\\d+",
            "text": "abc 123"
        }))
        .expect("match response");
        assert!(response.ok);
        assert_eq!(response.data.unwrap()["matched"], Value::Bool(true));
    }

    #[test]
    fn protocol_v1_shape_matches() {
        let response = run_json(json!({
            "action": "match",
            "input": {
                "pattern": "\\d+",
                "text": "abc 123"
            }
        }))
        .expect("match response");
        assert_eq!(response.action, "match");
        assert_eq!(response.data.unwrap()["matched"], Value::Bool(true));
    }

    #[test]
    fn invalid_regex_returns_structured_error() {
        let response = run_json(json!({
            "action": "match",
            "input": {
                "pattern": "[",
                "text": "abc"
            }
        }))
        .expect_err("invalid regex");
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "invalid_regex");
    }
}
