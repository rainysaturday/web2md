mod daemon;
mod dom_bridge;
mod js_engine;
mod page;

use clap::{Parser, Subcommand};
use regex::Regex;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use page::{Page, RenderConfig};

// ============================================================
// CLI Argument Parsing
// ============================================================

#[derive(Parser, Debug)]
#[command(name = "webmd")]
#[command(about = "Download a webpage and convert HTML to Markdown")]
#[command(version)]
#[command(long_about = "webmd - Webpage to Markdown converter\n\
    Fetches a webpage, optionally executes JavaScript, \
    and converts the HTML to clean Markdown.\n\
    Supports daemon mode for stateful page access.")]
struct Cli {
    /// URL to download and convert to Markdown (for simple mode)
    url: Option<String>,

    /// Output file path (defaults to stdout)
    #[arg(short = 'o', long = "output")]
    output: Option<PathBuf>,

    /// Include images in output (removed by default)
    #[arg(long = "with-images")]
    with_images: bool,

    // --- JavaScript / Render Lifecycle Flags ---

    /// Enable JavaScript execution for the fetched page
    #[arg(long = "enable-js")]
    enable_js: bool,

    /// Maximum time to wait for page stabilization (seconds, default: 30)
    #[arg(long = "render-timeout")]
    render_timeout: Option<u64>,

    /// Stability quiet period (milliseconds, default: 100)
    #[arg(long = "quiet-period")]
    quiet_period: Option<u64>,

    /// Capture the current DOM immediately without waiting for stability
    #[arg(long = "render-now")]
    render_now: bool,

    /// Do not fetch or execute external <script src="..."> references
    #[arg(long = "no-external-scripts")]
    no_external_scripts: bool,

    // --- Script Injection ---

    /// Inject a JavaScript file into the page before execution (can be specified multiple times)
    #[arg(long = "inject-script")]
    inject_scripts: Vec<String>,

    /// Inject an inline JavaScript snippet before execution (can be specified multiple times)
    #[arg(long = "inject-code")]
    inject_codes: Vec<String>,

    // --- Daemon Flags ---

    /// Start webmd in daemon mode
    #[arg(long = "daemon")]
    daemon: bool,

    /// Daemon HTTP listen port (default: 8765)
    #[arg(long = "daemon-port")]
    daemon_port: Option<u16>,

    /// Daemon Unix socket path (mutually exclusive with --daemon-port)
    #[arg(long = "daemon-socket")]
    daemon_socket: Option<String>,

    /// Fork daemon to background
    #[arg(long = "daemon-background")]
    daemon_background: bool,

    /// Log file path for daemon mode
    #[arg(long = "daemon-log-file")]
    daemon_log_file: Option<PathBuf>,

    /// Daemon URL for client commands (default: http://localhost:8765)
    #[arg(long = "daemon-url")]
    daemon_url: Option<String>,

    // --- Subcommands ---
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Enter daemon mode (alternative to --daemon flag)
    Daemon {
        /// HTTP listen port
        #[arg(short = 'p', long = "port")]
        port: Option<u16>,

        /// Unix socket path
        #[arg(short = 's', long = "socket")]
        socket: Option<String>,

        /// Fork to background
        #[arg(short = 'b', long = "background")]
        background: bool,

        /// Log file path
        #[arg(short = 'l', long = "log-file")]
        log_file: Option<PathBuf>,
    },
    /// Client commands for daemon interaction
    Client {
        #[command(subcommand)]
        client_command: ClientCommand,
    },
}

#[derive(Subcommand, Debug)]
enum ClientCommand {
    /// Fetch a new page by URL
    Fetch {
        /// URL to fetch
        url: String,
    },
    /// Get rendered HTML of a page
    Render {
        /// Page ID
        page_id: String,
    },
    /// Get Markdown output of a page
    Markdown {
        /// Page ID
        page_id: String,
    },
    /// Execute JavaScript in a page's context
    Eval {
        /// Page ID
        page_id: String,
        /// JavaScript expression/statement to execute
        script: String,
    },
    /// Simulate a click on an element
    Click {
        /// Page ID
        page_id: String,
        /// CSS selector for the element to click
        selector: String,
    },
    /// Fill an input field
    Fill {
        /// Page ID
        page_id: String,
        /// CSS selector for the input element
        selector: String,
        /// Value to fill
        value: String,
    },
    /// Close a page
    Close {
        /// Page ID
        page_id: String,
    },
    /// Check daemon health
    Health,
}

// ============================================================
// Main
// ============================================================

#[actix_web::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();

    // Initialize logging
    if args.daemon || args.command.is_some() {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .init();
    }

    // --- Handle subcommands ---

    match &args.command {
        Some(Command::Daemon {
            port,
            socket,
            background,
            log_file,
        }) => {
            // Daemon subcommand
            let listen_port = port.or(args.daemon_port);
            let socket_path = socket.clone().or(args.daemon_socket);
            let is_background = *background || args.daemon_background;

            if let Some(_log_path) = log_file.clone().or(args.daemon_log_file.clone()) {
                // Redirect logs to file (simplified)
                let _ = _log_path;
            }

            daemon::start_daemon(listen_port, socket_path, is_background).await?;
            return Ok(());
        }
        Some(Command::Client { client_command }) => {
            let daemon_url = args
                .daemon_url
                .clone()
                .unwrap_or_else(|| "http://localhost:8765".to_string());

            match client_command {
                ClientCommand::Fetch { url } => {
                    match daemon::client_fetch_page(&daemon_url, url).await {
                        Ok(response) => println!("{}", response),
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
                ClientCommand::Render { page_id } => {
                    match daemon::client_render_page(&daemon_url, page_id).await {
                        Ok(response) => println!("{}", response),
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
                ClientCommand::Markdown { page_id } => {
                    match daemon::client_markdown_page(&daemon_url, page_id).await {
                        Ok(response) => println!("{}", response),
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
                ClientCommand::Eval { page_id, script } => {
                    match daemon::client_eval_js(&daemon_url, page_id, script).await {
                        Ok(response) => println!("{}", response),
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
                ClientCommand::Click { page_id, selector } => {
                    match daemon::client_click_element(&daemon_url, page_id, selector).await {
                        Ok(response) => println!("{}", response),
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
                ClientCommand::Fill {
                    page_id,
                    selector,
                    value,
                } => {
                    match daemon::client_fill_element(&daemon_url, page_id, selector, value).await {
                        Ok(response) => println!("{}", response),
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
                ClientCommand::Close { page_id } => {
                    match daemon::client_close_page(&daemon_url, page_id).await {
                        Ok(response) => println!("{}", response),
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
                ClientCommand::Health => {
                    match daemon::client_health(&daemon_url).await {
                        Ok(response) => println!("{}", response),
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
            }
            return Ok(());
        }
        None => {
            // No subcommand, proceed with normal mode
        }
    }

    // --- Daemon mode (via flags) ---

    if args.daemon {
        daemon::start_daemon(args.daemon_port, args.daemon_socket, args.daemon_background)
            .await?;
        return Ok(());
    }

    // --- Simple mode: fetch URL and convert to Markdown ---

    let url = match &args.url {
        Some(u) => u.clone(),
        None => {
            eprintln!("Error: No URL provided. Run 'webmd --help' for usage.");
            std::process::exit(1);
        }
    };

    // Fetch the webpage
    let response = reqwest::get(&url).await?;
    let html = response.text().await?;

    // If JS is enabled, use the full page processing pipeline
    if args.enable_js {
        let config = RenderConfig {
            enable_js: true,
            with_images: args.with_images,
            render_timeout: Duration::from_secs(args.render_timeout.unwrap_or(30)),
            quiet_period: Duration::from_millis(args.quiet_period.unwrap_or(100)),
            fetch_external_scripts: !args.no_external_scripts,
            inject_scripts: args.inject_scripts.clone(),
            inject_codes: args.inject_codes.clone(),
        };

        let mut page = Page::new(url, html, config);

        // Execute external scripts if configured
        if !args.no_external_scripts {
            let _ = page.fetch_external_scripts().await;
        }

        if args.render_now {
            // Capture immediately
            let _ = page.render_now();
        } else {
            // Process through full lifecycle
            let _ = page.process();
        }

        // Get Markdown output
        let markdown = page.to_markdown();

        // Output to file or stdout
        if let Some(output_path) = args.output {
            let mut file = File::create(&output_path)?;
            file.write_all(markdown.as_bytes())?;
        } else {
            println!("{}", markdown);
        }
    } else {
        // Original mode: no JS execution
        let mut body_html = extract_body_html(&html);

        // Clean HTML - remove scripts, styles, CSS, JS
        body_html = clean_html(&body_html);

        // ARIA support: remove elements with aria-hidden="true"
        body_html = remove_aria_hidden_elements(&body_html);

        // ARIA support: use aria-label as fallback alt text for images
        body_html = apply_aria_labels_to_images(&body_html);

        // Remove images by default, unless --with-images is specified
        if !args.with_images {
            body_html = remove_images(&body_html);
        }

        // ARIA support: strip all remaining aria-* attributes from the output
        body_html = strip_aria_attributes(&body_html);

        // Convert HTML to Markdown
        let markdown = html2md::parse_html(&body_html);

        // Output to file or stdout
        if let Some(output_path) = args.output {
            let mut file = File::create(&output_path)?;
            file.write_all(markdown.as_bytes())?;
        } else {
            println!("{}", markdown);
        }
    }

    Ok(())
}

// ============================================================
// HTML Processing Functions (shared between main and page module)
// ============================================================

fn extract_body_html(html: &str) -> String {
    let document = scraper::Html::parse_document(html);
    let body_selector = scraper::Selector::parse("body").unwrap();

    if let Some(body) = document.select(&body_selector).next() {
        body.html()
    } else {
        html.to_string()
    }
}

fn remove_images(html: &str) -> String {
    let re = Regex::new(r"<img[^>]*>").unwrap();
    re.replace_all(html, "").to_string()
}

fn clean_html(html: &str) -> String {
    let mut cleaned = html.to_string();

    // Remove script tags and their content
    let script_re = Regex::new(r"(?s)<script[^>]*>.*?</script>").unwrap();
    cleaned = script_re.replace_all(&cleaned, "").to_string();

    // Remove style tags and their content
    let style_re = Regex::new(r"(?s)<style[^>]*>.*?</style>").unwrap();
    cleaned = style_re.replace_all(&cleaned, "").to_string();

    // Remove noscript tags and their content
    let noscript_re = Regex::new(r"(?s)<noscript[^>]*>.*?</noscript>").unwrap();
    cleaned = noscript_re.replace_all(&cleaned, "").to_string();

    // Remove inline event handlers (onclick, onload, etc.)
    let event_re = Regex::new(r#"\s+on\w+\s*=\s*["'][^"']*["']"#).unwrap();
    cleaned = event_re.replace_all(&cleaned, "").to_string();

    // Remove inline style attributes
    let style_attr_re = Regex::new(r#"\s+style\s*=\s*["'][^"']*["']"#).unwrap();
    cleaned = style_attr_re.replace_all(&cleaned, "").to_string();

    // Remove CSS class definitions that appear as text (e.g., .cls-1{fill:#fff})
    let css_def_re = Regex::new(r"\.[a-zA-Z_][a-zA-Z0-9_-]*\s*\{[^}]*\}").unwrap();
    cleaned = css_def_re.replace_all(&cleaned, "").to_string();

    // Remove window.* assignments (e.g., window.Di.bamData = {...})
    let window_re =
        Regex::new(r"window\.[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\};?")
            .unwrap();
    cleaned = window_re.replace_all(&cleaned, "").to_string();

    // Remove standalone CSS-like patterns (class definitions at start of text)
    let css_start_re = Regex::new(r"^\s*\.[a-zA-Z_][a-zA-Z0-9_-]*\s*\{[^}]*\}\s*").unwrap();
    cleaned = css_start_re.replace_all(&cleaned, "").to_string();

    cleaned
}

/// Remove entire HTML elements that have `aria-hidden="true"`.
fn remove_aria_hidden_elements(html: &str) -> String {
    let document = scraper::Html::parse_document(html);
    let selector = match scraper::Selector::parse(r#"[aria-hidden="true"]"#) {
        Ok(s) => s,
        Err(_) => return html.to_string(),
    };

    let mut matches: Vec<String> = Vec::new();
    for element in document.select(&selector) {
        matches.push(element.html());
    }

    if matches.is_empty() {
        return html.to_string();
    }

    let mut result = html.to_string();
    for outer_html in &matches {
        result = result.replacen(outer_html, "", 1);
    }

    result
}

/// Apply `aria-label` as alt text for `<img>` tags that lack an `alt` attribute.
fn apply_aria_labels_to_images(html: &str) -> String {
    let mut result = String::new();
    let mut pos = 0;
    let s = html;

    let img_re = Regex::new(r"<img\b[^>]*>").unwrap();
    let aria_label_re =
        Regex::new(r#"\s+aria-label\s*=\s*"([^"]*)"|aria-label\s*=\s*'([^']*)'"#).unwrap();
    let alt_check_re = Regex::new(r"\s+alt\s*=\s*").unwrap();

    while let Some(m) = img_re.find(&s[pos..]) {
        let abs_start = pos + m.start();
        let abs_end = pos + m.end();

        result.push_str(&s[pos..abs_start]);

        let img_tag = &s[abs_start..abs_end];

        let has_alt = alt_check_re.is_match(img_tag);

        if !has_alt {
            if let Some(caps) = aria_label_re.captures(img_tag) {
                let label_value = caps
                    .get(1)
                    .or_else(|| caps.get(2))
                    .map(|m| m.as_str())
                    .unwrap_or("");

                if !label_value.is_empty() {
                    let after_img = 4;
                    let modified = format!(
                        "{} alt=\"{}\"{}",
                        &img_tag[..after_img],
                        label_value.replace('"', "&quot;"),
                        &img_tag[after_img..]
                    );
                    result.push_str(&modified);
                } else {
                    result.push_str(img_tag);
                }
            } else {
                result.push_str(img_tag);
            }
        } else {
            result.push_str(img_tag);
        }

        pos = abs_end;
    }

    result.push_str(&s[pos..]);
    result
}

/// Strip all ARIA attributes (aria-*) from the HTML.
fn strip_aria_attributes(html: &str) -> String {
    let re_double =
        Regex::new(r#"\s+aria-[a-zA-Z_][a-zA-Z0-9_-]*\s*=\s*"(?:[^"\\]|\\.)*""#).unwrap();
    let re_single =
        Regex::new(r#"\s+aria-[a-zA-Z_][a-zA-Z0-9_-]*\s*=\s*'(?:[^'\\]|\\.)*'"#).unwrap();

    let mut result = re_double.replace_all(html, "").to_string();
    result = re_single.replace_all(&result, "").to_string();
    result
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Existing tests ---

    #[test]
    fn test_remove_aria_hidden_simple() {
        let html = r#"<div><span aria-hidden="true">hidden</span><span>visible</span></div>"#;
        let result = remove_aria_hidden_elements(html);
        assert_eq!(result, "<div><span>visible</span></div>");
    }

    #[test]
    fn test_remove_aria_hidden_nested() {
        let html =
            r#"<div aria-hidden="true"><div>nested</div><p>text</p></div><span>visible</span>"#;
        let result = remove_aria_hidden_elements(html);
        assert_eq!(result, "<span>visible</span>");
    }

    #[test]
    fn test_remove_aria_hidden_no_match() {
        let html = r#"<div><span>visible</span></div>"#;
        let result = remove_aria_hidden_elements(html);
        assert_eq!(result, html);
    }

    #[test]
    fn test_aria_label_on_img() {
        let html = r#"<img src="test.jpg" aria-label="A test image">"#;
        let result = apply_aria_labels_to_images(html);
        assert!(result.contains(r#"alt="A test image""#));
    }

    #[test]
    fn test_aria_label_not_applied_when_alt_exists() {
        let html = r#"<img src="test.jpg" alt="Existing" aria-label="Should not be used">"#;
        let result = apply_aria_labels_to_images(html);
        assert!(result.contains(r#"alt="Existing""#));
        assert!(!result.contains(r#"alt="Should not be used""#));
    }

    #[test]
    fn test_aria_label_single_quotes() {
        let html = r#"<img src='test.jpg' aria-label='A test image'>"#;
        let result = apply_aria_labels_to_images(html);
        assert!(result.contains(r#"alt="A test image""#));
    }

    #[test]
    fn test_strip_aria_attributes() {
        let html = r#"<div aria-hidden="true" aria-label="test" class="foo">content</div>"#;
        let result = strip_aria_attributes(html);
        assert!(!result.contains("aria-hidden"));
        assert!(!result.contains("aria-label"));
        assert!(result.contains("class=\"foo\""));
        assert!(result.contains("content"));
    }

    #[test]
    fn test_strip_aria_multiple() {
        let html = r#"<div aria-hidden="true"><span aria-label="test">text</span></div>"#;
        let result = strip_aria_attributes(html);
        assert!(!result.contains("aria-hidden"));
        assert!(!result.contains("aria-label"));
    }

    #[test]
    fn test_full_pipeline() {
        let html = r#"<body><div aria-hidden="true"><p>hidden text</p></div><p aria-label="hello">visible</p><img src="x.jpg" aria-label="photo"></body>"#;
        let mut body_html = extract_body_html(&html);
        body_html = clean_html(&body_html);
        body_html = remove_aria_hidden_elements(&body_html);
        body_html = apply_aria_labels_to_images(&body_html);
        body_html = strip_aria_attributes(&body_html);

        assert!(!body_html.contains("hidden text"));
        assert!(body_html.contains("visible"));
        assert!(body_html.contains(r#"alt="photo""#));
        assert!(!body_html.contains("aria-hidden"));
        assert!(!body_html.contains("aria-label"));
    }

    // --- New tests for JS engine ---

    #[test]
    fn test_js_engine_create() {
        let engine = js_engine::JsEngine::new(Duration::from_secs(10));
        assert_eq!(engine.pending_timer_count(), 0);
    }

    #[test]
    fn test_js_engine_execute() {
        let mut engine = js_engine::JsEngine::new(Duration::from_secs(10));
        let result = engine.execute("var x = 42;", "test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_js_engine_eval() {
        let mut engine = js_engine::JsEngine::new(Duration::from_secs(10));
        let result = engine.eval_expression("1 + 2");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "3");
    }

    // --- New tests for Page ---

    #[tokio::test]
    async fn test_page_no_js() {
        let config = page::RenderConfig {
            enable_js: false,
            ..page::RenderConfig::default()
        };
        let mut p = page::Page::new(
            "https://example.com".to_string(),
            "<html><body><p>Hello</p></body></html>".to_string(),
            config,
        );
        let result = p.process().await;
        assert!(result.is_ok());
        assert_eq!(p.state(), page::PageState::Stable);
    }

    #[test]
    fn test_page_to_markdown() {
        let config = page::RenderConfig::default();
        let p = page::Page::new(
            "https://example.com".to_string(),
            "<html><body><p>Hello World</p></body></html>".to_string(),
            config,
        );
        let md = p.to_markdown();
        assert!(md.contains("Hello World"));
    }

    // --- New test for DomBridge ---

    #[test]
    fn test_dom_bridge_create() {
        let bridge = dom_bridge::DomBridge::new("<html><body><p>Test</p></body></html>".to_string());
        assert!(!bridge.html().is_empty());
        assert_eq!(bridge.pending_count(), 0);
    }
}


#[cfg(test)]
mod integration_tests {
    use crate::page::{Page, RenderConfig};
    use std::time::Duration;

    #[test]
    fn test_dom_bridge_js_changes_reflected_in_html() {
        let html = String::from(r#"<html><head></head><body><div id="content">Original</div></body></html>"#);
        let config = RenderConfig {
            enable_js: true,
            render_timeout: Duration::from_secs(5),
            quiet_period: Duration::from_millis(10),
            fetch_external_scripts: false,
            with_images: false,
            inject_scripts: vec![],
            inject_codes: vec![],
        };

        let mut page = Page::new(String::from("http://example.com"), html, config);
        
        let js_code = String::from(r#"
            var div = document.getElementById('content');
            if (div) {
                div.textContent = 'Modified by JS';
                div.setAttribute('data-modified', 'true');
            }
            var newEl = document.createElement('p');
            newEl.textContent = 'New paragraph';
            document.body.appendChild(newEl);
        "#);
        
        let result = page.execute_js(&js_code, "test_modify");
        assert!(result.is_ok(), "JS execution failed: {:?}", result);
        
        let rendered_html = page.render_now();
        
        assert!(rendered_html.contains("Modified by JS"), 
            "Expected rendered HTML to contain 'Modified by JS', got: {}", rendered_html);
        assert!(rendered_html.contains("New paragraph"),
            "Expected rendered HTML to contain 'New paragraph', got: {}", rendered_html);
    }

    #[test]
    fn test_dom_bridge_create_element() {
        let html = String::from(r#"<html><head></head><body></body></html>"#);
        let config = RenderConfig {
            enable_js: true,
            render_timeout: Duration::from_secs(5),
            quiet_period: Duration::from_millis(10),
            fetch_external_scripts: false,
            with_images: false,
            inject_scripts: vec![],
            inject_codes: vec![],
        };

        let mut page = Page::new(String::from("http://example.com"), html, config);
        
        let js_code = String::from(r#"
            var div = document.createElement('div');
            div.textContent = 'Hello World';
            div.setAttribute('class', 'greeting');
            document.body.appendChild(div);
        "#);
        
        let result = page.execute_js(&js_code, "test_create");
        assert!(result.is_ok(), "JS execution failed: {:?}", result);
        
        let rendered_html = page.render_now();
        
        assert!(rendered_html.contains("Hello World"),
            "Expected 'Hello World' in rendered HTML, got: {}", rendered_html);
        assert!(rendered_html.contains("greeting"),
            "Expected 'greeting' in rendered HTML, got: {}", rendered_html);
    }

    #[test]
    fn test_serialize_dom_full() {
        let html = String::from(r#"<html><head></head><body><h1>Title</h1><p>Content</p></body></html>"#);
        let config = RenderConfig {
            enable_js: true,
            render_timeout: Duration::from_secs(5),
            quiet_period: Duration::from_millis(10),
            fetch_external_scripts: false,
            with_images: false,
            inject_scripts: vec![],
            inject_codes: vec![],
        };

        let mut page = Page::new(String::from("http://example.com"), html, config);
        
        let rendered_html = page.render_now();
        
        assert!(rendered_html.contains("Title"), "Expected 'Title' in rendered HTML");
        assert!(rendered_html.contains("Content"), "Expected 'Content' in rendered HTML");
    }
}
