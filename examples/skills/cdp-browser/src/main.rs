use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tungstenite::{Message, connect};

const DEFAULT_PORT: u16 = 9222;

#[derive(Debug, Parser)]
#[command(name = "cdp-browser")]
#[command(about = "Control Chrome through the Chrome DevTools Protocol")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Launch {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        chrome: Option<PathBuf>,
        #[arg(long)]
        user_data_dir: Option<PathBuf>,
        #[arg(long, default_value = "about:blank")]
        url: String,
    },
    Status {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    Open {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        url: String,
    },
    List {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    Eval {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        expression: String,
        #[arg(long)]
        target_id: Option<String>,
    },
    Click {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        selector: String,
        #[arg(long)]
        target_id: Option<String>,
    },
    Type {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        selector: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        target_id: Option<String>,
    },
    Screenshot {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        target_id: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct SkillRequest {
    action: String,
    port: Option<u16>,
    chrome: Option<PathBuf>,
    user_data_dir: Option<PathBuf>,
    url: Option<String>,
    expression: Option<String>,
    selector: Option<String>,
    text: Option<String>,
    path: Option<PathBuf>,
    target_id: Option<String>,
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

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Target {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(rename = "type", default)]
    target_type: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket_debugger_url: Option<String>,
}

fn main() {
    let result = run();
    match result {
        Ok(response) => print_json(&response),
        Err(error) => {
            let response = SkillResponse {
                ok: false,
                action: "error".to_string(),
                data: None,
                error: Some(SkillError {
                    code: "execution_failed".to_string(),
                    message: error,
                    retryable: false,
                }),
            };
            print_json(&response);
            std::process::exit(1);
        }
    }
}

fn run() -> Result<SkillResponse, String> {
    let cli = Cli::parse();
    if let Some(command) = cli.command {
        execute_command(command)
    } else {
        let request = read_stdin_request()?;
        execute_request(request)
    }
}

fn print_json(response: &SkillResponse) {
    println!(
        "{}",
        serde_json::to_string_pretty(response).unwrap_or_else(|_| "{\"ok\":false}".to_string())
    );
}

fn read_stdin_request() -> Result<SkillRequest, String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|err| format!("Failed to read stdin: {err}"))?;
    if input.trim().is_empty() {
        return Err("Expected a JSON request on stdin when no CLI command is provided".to_string());
    }
    serde_json::from_str(&input).map_err(|err| format!("Invalid JSON request: {err}"))
}

fn execute_request(request: SkillRequest) -> Result<SkillResponse, String> {
    let port = request.port.unwrap_or(DEFAULT_PORT);
    match request.action.as_str() {
        "describe" => describe(),
        "launch" => launch(port, request.chrome, request.user_data_dir, request.url),
        "status" => status(port),
        "open" => open_tab(port, require(request.url, "url")?),
        "list" => list_tabs(port),
        "eval" => eval_js(
            port,
            request.target_id,
            require(request.expression, "expression")?,
        ),
        "click" => click(
            port,
            request.target_id,
            require(request.selector, "selector")?,
        ),
        "type" => type_text(
            port,
            request.target_id,
            require(request.selector, "selector")?,
            require(request.text, "text")?,
        ),
        "screenshot" => screenshot(port, request.target_id, require(request.path, "path")?),
        other => Err(format!("Unsupported action: {other}")),
    }
}

fn execute_command(command: Commands) -> Result<SkillResponse, String> {
    match command {
        Commands::Launch {
            port,
            chrome,
            user_data_dir,
            url,
        } => launch(port, chrome, user_data_dir, Some(url)),
        Commands::Status { port } => status(port),
        Commands::Open { port, url } => open_tab(port, url),
        Commands::List { port } => list_tabs(port),
        Commands::Eval {
            port,
            expression,
            target_id,
        } => eval_js(port, target_id, expression),
        Commands::Click {
            port,
            selector,
            target_id,
        } => click(port, target_id, selector),
        Commands::Type {
            port,
            selector,
            text,
            target_id,
        } => type_text(port, target_id, selector, text),
        Commands::Screenshot {
            port,
            path,
            target_id,
        } => screenshot(port, target_id, path),
    }
}

fn require<T>(value: Option<T>, name: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("Missing required field: {name}"))
}

fn describe() -> Result<SkillResponse, String> {
    Ok(SkillResponse {
        ok: true,
        action: "describe".to_string(),
        data: Some(json!({
            "id": "cdp-browser",
            "actions": ["describe", "launch", "status", "open", "list", "eval", "click", "type", "screenshot"],
            "protocol": "stdio-json"
        })),
        error: None,
    })
}

fn launch(
    port: u16,
    chrome: Option<PathBuf>,
    user_data_dir: Option<PathBuf>,
    url: Option<String>,
) -> Result<SkillResponse, String> {
    if status(port).is_ok() {
        return Ok(SkillResponse {
            ok: true,
            action: "launch".to_string(),
            data: Some(json!({
                "port": port,
                "message": "Chrome CDP is already reachable"
            })),
            error: None,
        });
    }

    let chrome_path = chrome.unwrap_or_else(default_chrome_path);
    if !chrome_path.exists() {
        return Err(format!(
            "Chrome binary was not found at {}",
            chrome_path.display()
        ));
    }

    let profile = user_data_dir.unwrap_or_else(default_user_data_dir);
    fs::create_dir_all(&profile)
        .map_err(|err| format!("Failed to create profile directory: {err}"))?;

    Command::new(&chrome_path)
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(url.unwrap_or_else(|| "about:blank".to_string()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("Failed to launch Chrome: {err}"))?;

    wait_for_cdp(port, Duration::from_secs(10))?;
    Ok(SkillResponse {
        ok: true,
        action: "launch".to_string(),
        data: Some(json!({
            "port": port,
            "chrome": chrome_path,
            "user_data_dir": profile
        })),
        error: None,
    })
}

fn status(port: u16) -> Result<SkillResponse, String> {
    let version = http_json("GET", port, "/json/version", None)?;
    Ok(SkillResponse {
        ok: true,
        action: "status".to_string(),
        data: Some(json!({
            "port": port,
            "browser": version.get("Browser").cloned(),
            "web_socket_debugger_url": version.get("webSocketDebuggerUrl").cloned()
        })),
        error: None,
    })
}

fn open_tab(port: u16, url: String) -> Result<SkillResponse, String> {
    let escaped_url = percent_encode_url_param(&url);
    let target = http_json("PUT", port, &format!("/json/new?{escaped_url}"), None)?;
    Ok(SkillResponse {
        ok: true,
        action: "open".to_string(),
        data: Some(json!({
            "target_id": target.get("id").cloned(),
            "title": target.get("title").cloned(),
            "url": target.get("url").cloned(),
            "websocket_debugger_url": target.get("webSocketDebuggerUrl").cloned()
        })),
        error: None,
    })
}

fn list_tabs(port: u16) -> Result<SkillResponse, String> {
    let targets = list_targets(port)?;
    Ok(SkillResponse {
        ok: true,
        action: "list".to_string(),
        data: Some(json!({ "targets": targets })),
        error: None,
    })
}

fn eval_js(
    port: u16,
    target_id: Option<String>,
    expression: String,
) -> Result<SkillResponse, String> {
    let result = call_runtime_evaluate(port, target_id, expression)?;
    Ok(SkillResponse {
        ok: true,
        action: "eval".to_string(),
        data: Some(json!({ "result": result })),
        error: None,
    })
}

fn click(port: u16, target_id: Option<String>, selector: String) -> Result<SkillResponse, String> {
    let expression = format!(
        r#"(function() {{
const element = document.querySelector({});
if (!element) throw new Error("Element not found");
element.scrollIntoView({{ block: "center", inline: "center" }});
element.click();
return true;
}})()"#,
        serde_json::to_string(&selector).map_err(|err| err.to_string())?
    );
    let result = call_runtime_evaluate(port, target_id, expression)?;
    Ok(SkillResponse {
        ok: true,
        action: "click".to_string(),
        data: Some(json!({ "selector": selector, "result": result })),
        error: None,
    })
}

fn type_text(
    port: u16,
    target_id: Option<String>,
    selector: String,
    text: String,
) -> Result<SkillResponse, String> {
    let expression = format!(
        r#"(function() {{
const element = document.querySelector({});
if (!element) throw new Error("Element not found");
element.focus();
element.value = {};
element.dispatchEvent(new Event("input", {{ bubbles: true }}));
element.dispatchEvent(new Event("change", {{ bubbles: true }}));
return element.value;
}})()"#,
        serde_json::to_string(&selector).map_err(|err| err.to_string())?,
        serde_json::to_string(&text).map_err(|err| err.to_string())?
    );
    let result = call_runtime_evaluate(port, target_id, expression)?;
    Ok(SkillResponse {
        ok: true,
        action: "type".to_string(),
        data: Some(json!({ "selector": selector, "result": result })),
        error: None,
    })
}

fn screenshot(
    port: u16,
    target_id: Option<String>,
    path: PathBuf,
) -> Result<SkillResponse, String> {
    let target = select_target(port, target_id)?;
    let websocket_url = target
        .websocket_debugger_url
        .ok_or_else(|| "Selected target does not expose a websocket URL".to_string())?;
    let params = json!({ "format": "png", "fromSurface": true });
    let result = call_cdp(&websocket_url, "Page.captureScreenshot", Some(params))?;
    let data = result
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| "CDP screenshot response did not include data".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|err| format!("Failed to decode screenshot data: {err}"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create screenshot directory: {err}"))?;
    }
    fs::write(&path, bytes).map_err(|err| format!("Failed to write screenshot: {err}"))?;
    Ok(SkillResponse {
        ok: true,
        action: "screenshot".to_string(),
        data: Some(json!({ "path": path })),
        error: None,
    })
}

fn call_runtime_evaluate(
    port: u16,
    target_id: Option<String>,
    expression: String,
) -> Result<Value, String> {
    let target = select_target(port, target_id)?;
    let websocket_url = target
        .websocket_debugger_url
        .ok_or_else(|| "Selected target does not expose a websocket URL".to_string())?;
    let result = call_cdp(
        &websocket_url,
        "Runtime.evaluate",
        Some(json!({
            "expression": expression,
            "awaitPromise": true,
            "returnByValue": true
        })),
    )?;
    if let Some(exception) = result.get("exceptionDetails") {
        return Err(format!("JavaScript evaluation failed: {exception}"));
    }
    Ok(result.get("result").cloned().unwrap_or(Value::Null))
}

fn call_cdp(websocket_url: &str, method: &str, params: Option<Value>) -> Result<Value, String> {
    let (mut socket, _) =
        connect(websocket_url).map_err(|err| format!("Failed to connect websocket: {err}"))?;
    let request = json!({
        "id": 1,
        "method": method,
        "params": params.unwrap_or_else(|| json!({}))
    });
    socket
        .send(Message::Text(request.to_string().into()))
        .map_err(|err| format!("Failed to send CDP message: {err}"))?;
    loop {
        let message = socket
            .read()
            .map_err(|err| format!("Failed to read CDP message: {err}"))?;
        if let Message::Text(text) = message {
            let response: Value = serde_json::from_str(&text)
                .map_err(|err| format!("Invalid CDP JSON response: {err}"))?;
            if response.get("id").and_then(Value::as_i64) == Some(1) {
                if let Some(error) = response.get("error") {
                    return Err(format!("CDP error: {error}"));
                }
                return Ok(response.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }
}

fn select_target(port: u16, target_id: Option<String>) -> Result<Target, String> {
    let targets = list_targets(port)?;
    if let Some(id) = target_id {
        return targets
            .into_iter()
            .find(|target| target.id == id)
            .ok_or_else(|| format!("Target not found: {id}"));
    }
    targets
        .into_iter()
        .find(|target| target.target_type == "page" && target.websocket_debugger_url.is_some())
        .ok_or_else(|| "No page target with websocket URL was found".to_string())
}

fn list_targets(port: u16) -> Result<Vec<Target>, String> {
    let value = http_json("GET", port, "/json/list", None)?;
    serde_json::from_value(value).map_err(|err| format!("Invalid target list response: {err}"))
}

fn wait_for_cdp(port: u16, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if status(port).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err(format!("Timed out waiting for CDP on port {port}"))
}

fn http_json(method: &str, port: u16, path: &str, body: Option<&str>) -> Result<Value, String> {
    let raw = http_request(method, port, path, body)?;
    let (_, response_body) = split_http_response(&raw)?;
    serde_json::from_str(response_body).map_err(|err| format!("Invalid JSON response: {err}"))
}

fn http_request(method: &str, port: u16, path: &str, body: Option<&str>) -> Result<String, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|err| format!("Failed to connect to 127.0.0.1:{port}: {err}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|err| format!("Failed to set HTTP read timeout: {err}"))?;
    let body = body.unwrap_or("");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("Failed to write HTTP request: {err}"))?;
    let response = read_http_response(&mut stream)?;
    if !response.starts_with("HTTP/1.1 200") {
        return Err(format!(
            "Unexpected HTTP response: {}",
            response.lines().next().unwrap_or("<empty>")
        ));
    }
    Ok(response)
}

fn read_http_response(stream: &mut TcpStream) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if has_complete_http_body(&bytes) {
                    break;
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) && !bytes.is_empty() =>
            {
                break;
            }
            Err(err) => return Err(format!("Failed to read HTTP response: {err}")),
        }
    }
    String::from_utf8(bytes).map_err(|err| format!("HTTP response was not valid UTF-8: {err}"))
}

fn has_complete_http_body(bytes: &[u8]) -> bool {
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let header = String::from_utf8_lossy(&bytes[..header_end]);
    let Some(length) = header.lines().find_map(|line| {
        line.strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
    }) else {
        return false;
    };
    bytes.len() >= header_end + 4 + length
}

fn split_http_response(response: &str) -> Result<(&str, &str), String> {
    response
        .split_once("\r\n\r\n")
        .ok_or_else(|| "Invalid HTTP response".to_string())
}

fn percent_encode_url_param(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b':' | b'/') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn default_chrome_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe")
    } else {
        PathBuf::from("google-chrome")
    }
}

fn default_user_data_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".restflow")
        .join(".restflow-browser")
        .join("cdp-browser-profile")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding_keeps_url_punctuation() {
        assert_eq!(
            percent_encode_url_param("https://example.com/a b?q=x&z=1"),
            "https://example.com/a%20b%3Fq%3Dx%26z%3D1"
        );
    }

    #[test]
    fn split_http_response_returns_body() {
        let (_, body) = split_http_response("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .expect("response should split");
        assert_eq!(body, "{}");
    }

    #[test]
    fn detects_complete_http_body_from_content_length() {
        assert!(has_complete_http_body(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"
        ));
        assert!(!has_complete_http_body(
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{}"
        ));
    }

    #[test]
    fn request_json_parses_action() {
        let request: SkillRequest =
            serde_json::from_str(r#"{"action":"status","port":9333}"#).expect("valid request");
        assert_eq!(request.action, "status");
        assert_eq!(request.port, Some(9333));
    }

    #[test]
    fn describe_action_has_no_side_effects() {
        let response = execute_request(SkillRequest {
            action: "describe".to_string(),
            port: None,
            chrome: None,
            user_data_dir: None,
            url: None,
            expression: None,
            selector: None,
            text: None,
            path: None,
            target_id: None,
        })
        .expect("describe response");
        assert!(response.ok);
        assert_eq!(response.action, "describe");
    }
}
