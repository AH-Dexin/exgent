use std::{
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use exgent_core::{
    finish_anthropic_oauth_flow, finish_github_copilot_device_flow, finish_openai_codex_oauth_flow,
    normalize_github_domain, parse_authorization_input, start_anthropic_oauth_flow,
    start_github_copilot_device_flow, start_openai_codex_oauth_flow, AnthropicOAuthFlow,
    AppRuntimeHost, AuthProviderInfo, AuthorizationCode, CompatibleModelKind,
};

use super::{
    plain::{
        exit_process, print_fitted_terminal_line, print_terminal_newline, read_cancelable_line,
        redraw_current_prompt_line, refresh_active_footer, reset_terminal_viewport, RawModeGuard,
    },
    selection::select_with_keys,
};

type TuiRuntime = AppRuntimeHost;

pub(super) fn open_auth_menu(runtime: &mut TuiRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let method = select_auth_method()?;
    match method {
        Some(AuthMethod::Subscription) => open_subscription_auth_menu(runtime),
        Some(AuthMethod::ApiKey) => open_api_key_auth_menu(runtime),
        None => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthMethod {
    Subscription,
    ApiKey,
}

fn select_auth_method() -> io::Result<Option<AuthMethod>> {
    let labels = ["Use a subscription", "Use an API key"];

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return Ok(
            select_with_keys("Select authentication method:", &labels)?.map(|index| {
                if index == 0 {
                    AuthMethod::Subscription
                } else {
                    AuthMethod::ApiKey
                }
            }),
        );
    }

    println!("Select authentication method:");
    println!("1) Use a subscription");
    println!("2) Use an API key");
    let Some(line) = read_cancelable_line("select auth method: ")? else {
        return Ok(None);
    };
    match line.trim() {
        "" => Ok(None),
        "1" => Ok(Some(AuthMethod::Subscription)),
        "2" => Ok(Some(AuthMethod::ApiKey)),
        _ => {
            println!("invalid authentication method");
            Ok(None)
        }
    }
}

fn open_subscription_auth_menu(runtime: &mut TuiRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let providers = runtime.subscription_providers();
    let labels = providers
        .iter()
        .map(|provider| {
            let status = auth_status_label(provider.has_subscription);
            format!("{}  {status}", provider.name)
        })
        .collect::<Vec<_>>();
    let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();

    let selected = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        select_with_keys("Select subscription provider:", &label_refs)?
    } else {
        println!("Select subscription provider:");
        for (index, label) in labels.iter().enumerate() {
            println!("{}) {label}", index + 1);
        }
        let Some(line) = read_cancelable_line("select provider: ")? else {
            return Ok(());
        };
        match line.trim().parse::<usize>() {
            Ok(value) if value > 0 && value <= labels.len() => Some(value - 1),
            _ => None,
        }
    };

    let Some(index) = selected else {
        println!("subscription login cancelled");
        return Ok(());
    };

    let provider = &providers[index];
    match provider.provider.as_str() {
        "anthropic" => login_anthropic_subscription(runtime, &provider.provider),
        "github-copilot" => login_github_copilot_subscription(runtime, &provider.provider),
        "openai-codex" => login_openai_codex_subscription(runtime, &provider.provider),
        _ => {
            println!("unknown subscription provider: {}", provider.provider);
            Ok(())
        }
    }
}

fn login_openai_codex_subscription(runtime: &mut TuiRuntime, provider_id: &str) -> io::Result<()> {
    refresh_active_footer(runtime);
    println!("OpenAI Codex subscription login");
    let flow = match start_openai_codex_oauth_flow() {
        Ok(flow) => flow,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(());
        }
    };

    println!("Open this URL in your browser:");
    println!("{}", flow.url);
    let _ = open::that(&flow.url);
    println!("Complete login in your browser.");
    println!("Waiting for browser callback...");

    let authorization = match flow.wait_for_callback(Duration::from_secs(10 * 60)) {
        Ok(code) => Some(code),
        Err(error) => {
            println!("{error}");
            println!("Paste final redirect URL or authorization code, or press Enter to cancel:");
            let Some(line) = read_cancelable_line("authorization: ")? else {
                return Ok(());
            };
            match parse_authorization_input(&line) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("error: {error}");
                    return Ok(());
                }
            }
        }
    };

    let Some(authorization) = authorization else {
        println!("subscription login cancelled");
        return Ok(());
    };

    match finish_openai_codex_oauth_flow(&flow, authorization)
        .and_then(|credential| runtime.set_oauth_credential(provider_id, credential))
    {
        Ok(()) => {
            println!("logged in to OpenAI Codex");
            println!("credentials saved to {}", runtime.auth_path());
            println!("selected model: {}", runtime.model_label());
        }
        Err(error) => eprintln!("error: {error}"),
    }
    Ok(())
}

fn login_anthropic_subscription(runtime: &mut TuiRuntime, provider_id: &str) -> io::Result<()> {
    refresh_active_footer(runtime);
    println!("Anthropic subscription login");
    let flow = match start_anthropic_oauth_flow() {
        Ok(flow) => flow,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(());
        }
    };

    println!("Open this URL in your browser:");
    println!("{}", flow.url);
    let _ = open::that(&flow.url);
    println!("Complete login in your browser.");
    println!("Waiting for browser callback...");

    let authorization = match wait_for_anthropic_authorization(&flow)? {
        Some(authorization) => authorization,
        None => {
            println!("subscription login cancelled");
            return Ok(());
        }
    };

    println!("Exchanging authorization code for tokens...");
    let credential = match finish_anthropic_oauth_flow(&flow, authorization) {
        Ok(credential) => credential,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(());
        }
    };

    match runtime.set_oauth_credential(provider_id, credential) {
        Ok(()) => println!(
            "Logged in to Anthropic. Credentials saved to {}",
            runtime.auth_path()
        ),
        Err(error) => eprintln!("error: {error}"),
    }
    Ok(())
}

fn wait_for_anthropic_authorization(
    flow: &AnthropicOAuthFlow,
) -> io::Result<Option<AuthorizationCode>> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return wait_for_anthropic_authorization_tty(flow);
    }

    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    if let Some(authorization) = parse_authorization_input_or_print(&line) {
        return Ok(Some(authorization));
    }

    match flow.wait_for_callback(Duration::from_secs(10 * 60)) {
        Ok(authorization) => Ok(Some(authorization)),
        Err(error) => {
            eprintln!("error: {error}");
            Ok(None)
        }
    }
}

fn wait_for_anthropic_authorization_tty(
    flow: &AnthropicOAuthFlow,
) -> io::Result<Option<AuthorizationCode>> {
    println!("Paste final redirect URL any time if callback does not return.");
    let prompt = "redirect URL: ";
    print!("{prompt}");
    io::stdout().flush()?;

    let _raw_mode = RawModeGuard::enable()?;
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    let mut input = String::new();

    loop {
        match flow.poll_callback(Duration::from_millis(50)) {
            Ok(Some(authorization)) => {
                print_terminal_newline()?;
                print_fitted_terminal_line("Browser callback received.")?;
                return Ok(Some(authorization));
            }
            Ok(None) => {}
            Err(error) => {
                print_terminal_newline()?;
                print_fitted_terminal_line(format!("error: {error}"))?;
                return Ok(None);
            }
        }

        if Instant::now() >= deadline {
            print_terminal_newline()?;
            print_fitted_terminal_line("error: timed out waiting for OAuth callback")?;
            return Ok(None);
        }

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Enter => {
                    print_terminal_newline()?;
                    if input.trim().is_empty() {
                        redraw_current_prompt_line(prompt, &input)?;
                        continue;
                    }
                    return Ok(parse_authorization_input_or_print(&input));
                }
                KeyCode::Esc => {
                    print_terminal_newline()?;
                    return Ok(None);
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    print_terminal_newline()?;
                    exit_process();
                }
                KeyCode::Backspace if input.pop().is_some() => {
                    redraw_current_prompt_line(prompt, &input)?;
                }
                KeyCode::Char(value) => {
                    input.push(value);
                    redraw_current_prompt_line(prompt, &input)?;
                }
                _ => {}
            },
            Event::Paste(value) => {
                input.push_str(&value);
                redraw_current_prompt_line(prompt, &input)?;
            }
            Event::Resize(_, _) => {
                reset_terminal_viewport()?;
            }
            _ => {}
        }
    }
}

fn parse_authorization_input_or_print(input: &str) -> Option<AuthorizationCode> {
    match parse_authorization_input(input) {
        Ok(authorization) => authorization,
        Err(error) => {
            eprintln!("error: {error}");
            None
        }
    }
}

fn login_github_copilot_subscription(
    runtime: &mut TuiRuntime,
    provider_id: &str,
) -> io::Result<()> {
    refresh_active_footer(runtime);
    println!("GitHub Copilot subscription login");
    println!("Leave enterprise domain blank for github.com.");
    let Some(enterprise_input) = read_cancelable_line("GitHub Enterprise URL/domain: ")? else {
        println!("subscription login cancelled");
        return Ok(());
    };
    let enterprise_domain = normalize_github_domain(&enterprise_input);
    if !enterprise_input.trim().is_empty() && enterprise_domain.is_none() {
        println!("invalid GitHub Enterprise URL/domain");
        return Ok(());
    }

    let flow = match start_github_copilot_device_flow(enterprise_domain.as_deref()) {
        Ok(flow) => flow,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(());
        }
    };

    println!("Open this URL in your browser:");
    println!("{}", flow.verification_uri);
    let _ = open::that(&flow.verification_uri);
    println!("Enter code: {}", flow.user_code);

    let credential = match finish_github_copilot_device_flow(&flow, |message| println!("{message}"))
    {
        Ok(credential) => credential,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(());
        }
    };

    match runtime.set_oauth_credential(provider_id, credential) {
        Ok(()) => println!(
            "Logged in to GitHub Copilot. Credentials saved to {}",
            runtime.auth_path()
        ),
        Err(error) => eprintln!("error: {error}"),
    }
    Ok(())
}

fn open_api_key_auth_menu(runtime: &mut TuiRuntime) -> io::Result<()> {
    refresh_active_footer(runtime);
    let providers = runtime.auth_providers();
    println!("auth file: {}", runtime.auth_path());
    println!("models file: {}", runtime.models_path());

    let selected_index = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let mut labels = providers
            .iter()
            .map(|provider| {
                let status = auth_status_label(provider.has_token);
                format!("{}  {status}", provider.provider)
            })
            .collect::<Vec<_>>();
        labels.push("custom model".to_string());
        let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
        select_with_keys("Select provider to configure:", &label_refs)?
    } else {
        open_api_key_auth_menu_line(&providers)?
    };

    let Some(selected_index) = selected_index else {
        println!("auth configuration cancelled");
        return Ok(());
    };

    if selected_index == providers.len() {
        return add_custom_model(runtime);
    }

    let Some(selected) = providers.get(selected_index) else {
        println!("invalid provider selection");
        return Ok(());
    };
    configure_api_key_provider(runtime, &selected.provider)?;
    refresh_active_footer(runtime);

    Ok(())
}

fn open_api_key_auth_menu_line(providers: &[AuthProviderInfo]) -> io::Result<Option<usize>> {
    for (index, provider) in providers.iter().enumerate() {
        let status = auth_status_label(provider.has_token);
        println!("{}) {}  {}", index + 1, provider.provider, status);
    }
    println!("{}) custom model", providers.len() + 1);

    let Some(line) = read_cancelable_line("select auth action: ")? else {
        return Ok(None);
    };
    let choice = line.trim();
    if choice.is_empty() {
        return Ok(None);
    }

    match choice.parse::<usize>() {
        Ok(value) if value > 0 && value <= providers.len() + 1 => Ok(Some(value - 1)),
        _ => {
            println!("invalid provider selection");
            Ok(None)
        }
    }
}

fn configure_api_key_provider(runtime: &mut TuiRuntime, provider: &str) -> io::Result<()> {
    refresh_active_footer(runtime);
    println!("enter token for {provider}.");
    println!("leave blank to remove the stored token.");
    let Some(line) = read_cancelable_line("token: ")? else {
        println!("auth configuration cancelled");
        return Ok(());
    };
    let token = line.trim();

    if token.is_empty() {
        match runtime.remove_auth_token(provider) {
            Ok(()) => println!("removed auth token for {provider}"),
            Err(error) => eprintln!("error: {error}"),
        }
    } else {
        match runtime.set_auth_token(provider, token) {
            Ok(()) => println!("saved auth token for {provider}"),
            Err(error) => eprintln!("error: {error}"),
        }
    }

    Ok(())
}

fn add_custom_model(runtime: &mut TuiRuntime) -> io::Result<()> {
    let Some(kind) = select_custom_model_kind()? else {
        println!("model configuration cancelled");
        return Ok(());
    };
    add_compatible_model(runtime, kind)
}

fn select_custom_model_kind() -> io::Result<Option<CompatibleModelKind>> {
    let labels = CompatibleModelKind::ALL
        .iter()
        .map(|kind| kind.label())
        .collect::<Vec<_>>();

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return Ok(
            select_with_keys("Select custom model compatibility:", &labels)?
                .map(|index| CompatibleModelKind::ALL[index]),
        );
    }

    println!("Select custom model compatibility:");
    for (index, label) in labels.iter().enumerate() {
        println!("{}) {label}", index + 1);
    }
    let Some(line) = read_cancelable_line("select compatibility: ")? else {
        return Ok(None);
    };
    match line.trim() {
        "" => Ok(None),
        "1" | "openai" | "OpenAI" => Ok(Some(CompatibleModelKind::OpenAi)),
        "2" | "anthropic" | "Anthropic" => Ok(Some(CompatibleModelKind::Anthropic)),
        "3" | "google" | "Google" => Ok(Some(CompatibleModelKind::Google)),
        _ => {
            println!("invalid compatibility selection");
            Ok(None)
        }
    }
}

fn add_compatible_model(runtime: &mut TuiRuntime, kind: CompatibleModelKind) -> io::Result<()> {
    refresh_active_footer(runtime);
    println!("custom model: {}", kind.label());
    let Some(provider) = prompt_required(kind.provider_hint())? else {
        return Ok(());
    };
    let Some(model_id) = prompt_required(kind.model_hint())? else {
        return Ok(());
    };
    let Some(base_url) = prompt_required(kind.base_url_hint())? else {
        return Ok(());
    };

    println!("enter API key for {provider}.");
    println!("leave blank to add the model without storing a key.");
    let Some(api_key) = read_cancelable_line("api key: ")? else {
        println!("model configuration cancelled");
        return Ok(());
    };

    match runtime.add_compatible_model(kind, &provider, &model_id, &base_url, api_key.trim()) {
        Ok(info) => {
            println!(
                "added model {} / {} at index {}",
                info.provider,
                info.model_id,
                info.index + 1
            );
            println!("selected model: {}", runtime.model_label());
        }
        Err(error) => eprintln!("error: {error}"),
    }

    Ok(())
}

fn prompt_required(label: &str) -> io::Result<Option<String>> {
    let Some(line) = read_cancelable_line(&format!("{label}: "))? else {
        println!("model configuration cancelled");
        return Ok(None);
    };
    let value = line.trim().to_string();
    if value.is_empty() {
        println!("required value was empty");
        return Ok(None);
    }
    Ok(Some(value))
}

fn auth_status_label(is_configured: bool) -> String {
    if is_configured {
        success("configured")
    } else {
        "missing".to_string()
    }
}

fn success(text: &str) -> String {
    format!("\x1b[32;1m{text}\x1b[0m")
}
