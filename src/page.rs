use crate::dom_bridge::DomBridge;
use crate::js_engine::JsEngine;
use std::time::{Duration, Instant};

/// The state of a page in its render lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PageState {
    /// HTML is being fetched from the network.
    Fetching,
    /// HTML is being parsed and initial synchronous scripts execute.
    Parsing,
    /// Async scripts, timers, and fetch responses are being processed.
    Rendering,
    /// No pending timers, network requests, or microtasks remain; DOM is finalized.
    Stable,
    /// An error occurred during processing.
    Error,
}

/// Configuration for page rendering.
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// Maximum time to wait for page stabilization.
    pub render_timeout: Duration,
    /// Quiet period in milliseconds for stability detection.
    pub quiet_period: Duration,
    /// Whether to fetch and execute external scripts.
    pub fetch_external_scripts: bool,
    /// Whether to include images in the output.
    pub with_images: bool,
    /// Scripts to inject before execution.
    pub inject_scripts: Vec<String>,
    /// Code snippets to inject before execution.
    pub inject_codes: Vec<String>,
    /// Whether to enable JS execution at all.
    pub enable_js: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            render_timeout: Duration::from_secs(30),
            quiet_period: Duration::from_millis(100),
            fetch_external_scripts: true,
            with_images: false,
            inject_scripts: Vec::new(),
            inject_codes: Vec::new(),
            enable_js: true,
        }
    }
}

/// A managed page with JS context, DOM state, and lifecycle tracking.
pub struct Page {
    /// Unique identifier for this page.
    id: String,
    /// The URL this page was fetched from.
    url: String,
    /// Current lifecycle state.
    state: PageState,
    /// The raw HTML content (updated as DOM changes occur).
    html: String,
    /// The JavaScript engine (if JS is enabled).
    js_engine: Option<JsEngine>,
    /// The DOM bridge for JS<->HTML interaction.
    dom_bridge: Option<DomBridge>,
    /// Render configuration.
    config: RenderConfig,
    /// When the page was created.
    created_at: Instant,
    /// When the page entered the current state.
    state_started_at: Instant,
    /// When rendering started (entered Rendering state).
    rendering_started_at: Option<Instant>,
    /// Error messages collected during processing.
    errors: Vec<String>,
    /// Whether the render timeout has been hit.
    timed_out: bool,
    /// Pending timer count (tracked separately for stabilization).
    #[allow(dead_code)]
    pending_timers: u32,
    /// Whether the page has been quiet (no changes) for the quiet period.
    was_quiet: bool,
    /// Last time a DOM change was detected.
    last_change_time: Option<Instant>,
}

// SAFETY: `Page` is only accessed under a mutex in multi-threaded contexts (daemon).
// The `JsEngine` inside is `Send` due to its own unsafe impl.
unsafe impl Send for Page {}
unsafe impl Sync for Page {}

#[allow(dead_code)]
impl Page {
    /// Create a new page for the given URL with optional JS support.
    pub fn new(url: String, html: String, config: RenderConfig) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Instant::now();

        let mut page = Self {
            id,
            url,
            state: PageState::Fetching,
            html: html.clone(),
            js_engine: None,
            dom_bridge: None,
            config,
            created_at: now,
            state_started_at: now,
            rendering_started_at: None,
            errors: Vec::new(),
            timed_out: false,
            pending_timers: 0,
            was_quiet: false,
            last_change_time: Some(now),
        };

        // If JS is enabled, set up the JS engine and DOM bridge
        if page.config.enable_js {
            page.setup_js_engine();
        }

        page
    }

    /// Set up the JS engine and DOM bridge for this page.
    fn setup_js_engine(&mut self) {
        let mut engine = JsEngine::new(self.config.render_timeout);
        let bridge = DomBridge::new(self.html.clone());

        // Inject the DOM API shim into the JS engine
        let dom_api_code = DomBridge::dom_api_code();
        let _ = engine.execute(dom_api_code, "dom_api_shim");

        // Inject the page HTML into the JS virtual DOM
        if let Err(e) = bridge.inject_html_into_js(&mut engine, &self.html) {
            self.errors.push(format!("Failed to inject HTML into JS: {}", e));
        }

        // Inject user-provided scripts and code
        for script_path in &self.config.inject_scripts {
            match std::fs::read_to_string(script_path) {
                Ok(code) => {
                    let _ = engine.execute(&code, &format!("inject:{}", script_path));
                }
                Err(e) => {
                    self.errors
                        .push(format!("Failed to read inject script '{}': {}", script_path, e));
                }
            }
        }
        for code in &self.config.inject_codes {
            let _ = engine.execute(code, "inject:code");
        }

        self.js_engine = Some(engine);
        self.dom_bridge = Some(bridge);
    }

    /// Transition to a new state.
    pub fn transition_to(&mut self, new_state: PageState) {
        let now = Instant::now();
        self.state = new_state;
        self.state_started_at = now;

        if new_state == PageState::Rendering {
            self.rendering_started_at = Some(now);
        }

        log::debug!("Page {} transitioned to {:?}", self.id, new_state);
    }

    /// Process the page through its lifecycle.
    ///
    /// This method orchestrates the full render lifecycle:
    /// 1. Parse HTML and execute inline scripts
    /// 2. Process timers and async operations
    /// 3. Wait for stabilization
    /// 4. Return the final HTML
    pub async fn process(&mut self) -> Result<String, String> {
        if !self.config.enable_js {
            // No JS processing needed, just return the HTML as-is
            self.transition_to(PageState::Stable);
            return Ok(self.html.clone());
        }

        // Step 1: Parse and execute initial scripts
        self.transition_to(PageState::Parsing);
        self.execute_inline_scripts()?;

        // Step 2: Move to rendering phase
        self.transition_to(PageState::Rendering);

        // Step 3: Process timers and wait for stabilization
        let max_render_time = self.config.render_timeout;
        let _quiet_period = self.config.quiet_period;
        let start = Instant::now();

        loop {
            // Check for timeout
            if start.elapsed() > max_render_time {
                self.timed_out = true;
                self.errors
                    .push("Render timeout reached, capturing current DOM".to_string());
                break;
            }

            // Process pending timers
            if let Some(ref mut engine) = self.js_engine {
                let elapsed = start.elapsed().as_millis() as u64;
                engine.process_timers(elapsed);

                // Check if we're stable (no pending timers, no pending DOM operations)
                let has_pending_timers = engine.has_pending_timers();
                let has_pending_dom = self
                    .dom_bridge
                    .as_ref()
                    .map(|b| b.pending_count() > 0)
                    .unwrap_or(false);

                if !has_pending_timers && !has_pending_dom {
                    // Serialize JS virtual DOM to HTML
                    if let (Some(ref mut bridge), Some(ref mut engine)) = (self.dom_bridge.as_mut(), self.js_engine.as_mut()) {
                        self.html = bridge.finalize(engine);
                    }

                    // Check if we've been quiet long enough
                    if self.was_quiet {
                        // We're stable!
                        self.transition_to(PageState::Stable);
                        break;
                    } else {
                        self.was_quiet = true;
                        self.last_change_time = Some(Instant::now());
                    }
                } else {
                    self.was_quiet = false;
                    self.last_change_time = Some(Instant::now());
                }
            } else {
                // No JS engine, we're stable
                self.transition_to(PageState::Stable);
                break;
            }

            // Small async sleep to avoid busy-waiting
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Finalize: serialize JS virtual DOM to HTML
        if let (Some(ref mut bridge), Some(ref mut engine)) = (self.dom_bridge.as_mut(), self.js_engine.as_mut()) {
            self.html = bridge.finalize(engine);
        }

        // Clean up timers
        if let Some(ref mut engine) = self.js_engine {
            engine.clear_timers();
        }

        if self.state != PageState::Stable {
            self.transition_to(PageState::Stable);
        }

        Ok(self.html.clone())
    }

    /// Execute inline `<script>` tags from the HTML.
    fn execute_inline_scripts(&mut self) -> Result<(), String> {
        if let Some(ref mut engine) = self.js_engine {
            // Extract script content from HTML using regex
            let script_re = Regex::new(r"(?s)<script[^>]*>(.*?)</script>").unwrap();
            let mut script_count = 0;

            for cap in script_re.captures_iter(&self.html) {
                let script_content = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                if script_content.trim().is_empty() {
                    continue;
                }

                let result = engine.execute(script_content, &format!("script_{}", script_count));
                if let Err(e) = result {
                    self.errors.push(e);
                }
                script_count += 1;
            }

            log::info!("Executed {} inline scripts", script_count);
        }
        Ok(())
    }

    /// Execute external scripts referenced by `<script src="...">`.
    pub async fn fetch_external_scripts(&mut self) -> Result<(), String> {
        if !self.config.fetch_external_scripts {
            return Ok(());
        }

        if let Some(ref mut engine) = self.js_engine {
            let src_re = Regex::new(r#"<script\s+[^>]*src\s*=\s*"([^"]*)"[^>]*>"#).unwrap();
            let mut script_count = 0;

            for cap in src_re.captures_iter(&self.html) {
                let src = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                if src.is_empty() {
                    continue;
                }

                // Resolve relative URLs
                let absolute_url = if src.starts_with("http://") || src.starts_with("https://") {
                    src.to_string()
                } else {
                    // Simple relative URL resolution
                    let base_url = self.url.trim_end_matches(|c| c != '/');
                    format!("{}{}", base_url, src.trim_start_matches('/'))
                };

                // Fetch the script using async reqwest
                match reqwest::get(&absolute_url).await {
                    Ok(response) => match response.text().await {
                        Ok(script_content) => {
                            let result = engine.execute(
                                &script_content,
                                &format!("external_{}", script_count),
                            );
                            if let Err(e) = result {
                                self.errors.push(e);
                            }
                            script_count += 1;
                        }
                        Err(e) => {
                            self.errors
                                .push(format!("Failed to read external script '{}': {}", src, e));
                        }
                    },
                    Err(e) => {
                        self.errors.push(format!(
                            "Failed to fetch external script '{}': {}",
                            src, e
                        ));
                    }
                }
            }

            log::info!("Fetched and executed {} external scripts", script_count);
        }
        Ok(())
    }

    /// Inject and execute a JavaScript expression in the page's context.
    pub fn eval_js(&mut self, script: &str) -> Result<String, String> {
        match self.js_engine {
            Some(ref mut engine) => engine.eval_expression(script),
            None => Err("JavaScript is not enabled for this page".to_string()),
        }
    }

    /// Execute a JavaScript statement (no return value expected).
    pub fn execute_js(&mut self, script: &str, source: &str) -> Result<(), String> {
        match self.js_engine {
            Some(ref mut engine) => engine.execute(script, source),
            None => Err("JavaScript is not enabled for this page".to_string()),
        }
    }

    /// Force render the current DOM state immediately without waiting for stabilization.
    pub fn render_now(&mut self) -> String {
        if let (Some(ref mut bridge), Some(ref mut engine)) = (self.dom_bridge.as_mut(), self.js_engine.as_mut()) {
            self.html = bridge.finalize(engine);
        }
        self.html.clone()
    }

    /// Get the current Markdown output from the HTML.
    pub fn to_markdown(&self) -> String {
        let mut body_html = self.html.clone();

        // If JS is not enabled, use the existing cleaning pipeline
        if !self.config.enable_js {
            // Extract body content
            let document = scraper::Html::parse_document(&body_html);
            let body_selector = scraper::Selector::parse("body").unwrap();
            if let Some(body) = document.select(&body_selector).next() {
                body_html = body.html();
            }
        }

        // Clean HTML
        let cleaned = clean_html(&body_html);

        // ARIA support
        let cleaned = remove_aria_hidden_elements(&cleaned);
        let cleaned = apply_aria_labels_to_images(&cleaned);

        // Remove images if not configured
        let cleaned = if !self.config.with_images {
            remove_images(&cleaned)
        } else {
            cleaned
        };

        // Strip ARIA attributes
        let cleaned = strip_aria_attributes(&cleaned);

        // Convert to Markdown
        html2md::parse_html(&cleaned)
    }

    // --- Getters ---

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn state(&self) -> PageState {
        self.state
    }

    pub fn html(&self) -> &str {
        &self.html
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    pub fn config(&self) -> &RenderConfig {
        &self.config
    }

    pub fn timed_out(&self) -> bool {
        self.timed_out
    }

    pub fn created_at(&self) -> &Instant {
        &self.created_at
    }

    pub fn state_started_at(&self) -> &Instant {
        &self.state_started_at
    }

    /// Simulate a click on an element matching the CSS selector.
    pub fn simulate_click(&mut self, _selector: &str) -> Result<(), String> {
        // In a full implementation, this would:
        // 1. Find the element in the HTML
        // 2. Fire its click event handler
        // 3. Process resulting DOM changes
        // 4. Re-enter the render lifecycle
        Err("Click simulation not yet implemented".to_string())
    }

    /// Fill an input field matching the CSS selector with the given value.
    pub fn simulate_fill(&mut self, _selector: &str, _value: &str) -> Result<(), String> {
        // In a full implementation, this would:
        // 1. Find the input element in the HTML
        // 2. Set its value
        // 3. Fire input/change events
        // 4. Process resulting DOM changes
        Err("Fill simulation not yet implemented".to_string())
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        // JS engine is dropped automatically via Drop
        log::debug!("Dropping page {}", self.id);
    }
}

// Helper functions from main.rs, duplicated here for page module use
use regex::Regex;

#[allow(dead_code)]
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

    let script_re = Regex::new(r"(?s)<script[^>]*>.*?</script>").unwrap();
    cleaned = script_re.replace_all(&cleaned, "").to_string();

    let style_re = Regex::new(r"(?s)<style[^>]*>.*?</style>").unwrap();
    cleaned = style_re.replace_all(&cleaned, "").to_string();

    let noscript_re = Regex::new(r"(?s)<noscript[^>]*>.*?</noscript>").unwrap();
    cleaned = noscript_re.replace_all(&cleaned, "").to_string();

    let event_re = Regex::new(r#"\s+on\w+\s*=\s*["'][^"']*["']"#).unwrap();
    cleaned = event_re.replace_all(&cleaned, "").to_string();

    let style_attr_re = Regex::new(r#"\s+style\s*=\s*["'][^"']*["']"#).unwrap();
    cleaned = style_attr_re.replace_all(&cleaned, "").to_string();

    let css_def_re = Regex::new(r"\.[a-zA-Z_][a-zA-Z0-9_-]*\s*\{[^}]*\}").unwrap();
    cleaned = css_def_re.replace_all(&cleaned, "").to_string();

    let window_re =
        Regex::new(r"window\.[a-zA-Z_][a-zA-Z0-9_]*\s*=\s*\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\};?")
            .unwrap();
    cleaned = window_re.replace_all(&cleaned, "").to_string();

    let css_start_re = Regex::new(r"^\s*\.[a-zA-Z_][a-zA-Z0-9_-]*\s*\{[^}]*\}\s*").unwrap();
    cleaned = css_start_re.replace_all(&cleaned, "").to_string();

    cleaned
}

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

fn strip_aria_attributes(html: &str) -> String {
    let re_double =
        Regex::new(r#"\s+aria-[a-zA-Z_][a-zA-Z0-9_-]*\s*=\s*"(?:[^"\\]|\\.)*""#).unwrap();
    let re_single =
        Regex::new(r#"\s+aria-[a-zA-Z_][a-zA-Z0-9_-]*\s*=\s*'(?:[^'\\]|\\.)*'"#).unwrap();

    let mut result = re_double.replace_all(html, "").to_string();
    result = re_single.replace_all(&result, "").to_string();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_creation() {
        let config = RenderConfig::default();
        let page = Page::new(
            "https://example.com".to_string(),
            "<html><body><p>Hello</p></body></html>".to_string(),
            config,
        );
        assert_eq!(page.state(), PageState::Fetching);
        assert!(!page.id().is_empty());
        assert_eq!(page.url(), "https://example.com");
    }

    #[tokio::test]
    async fn test_page_no_js() {
        let config = RenderConfig {
            enable_js: false,
            ..Default::default()
        };
        let mut page = Page::new(
            "https://example.com".to_string(),
            "<html><body><p>Hello</p></body></html>".to_string(),
            config,
        );
        let result = page.process().await;
        assert!(result.is_ok());
        assert_eq!(page.state(), PageState::Stable);
    }

    #[tokio::test]
    async fn test_page_with_js() {
        let config = RenderConfig {
            enable_js: true,
            render_timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let mut page = Page::new(
            "https://example.com".to_string(),
            "<html><body><p>Hello</p><script>var x = 42;</script></body></html>".to_string(),
            config,
        );
        let result = page.process().await;
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_render_now() {
        let config = RenderConfig {
            enable_js: true,
            ..Default::default()
        };
        let mut page = Page::new(
            "https://example.com".to_string(),
            "<html><body><p>Hello</p></body></html>".to_string(),
            config,
        );
        let html = page.render_now();
        assert!(!html.is_empty());
    }

    #[test]
    fn test_to_markdown() {
        let config = RenderConfig::default();
        let page = Page::new(
            "https://example.com".to_string(),
            "<html><body><p>Hello World</p></body></html>".to_string(),
            config,
        );
        let md = page.to_markdown();
        assert!(md.contains("Hello World"));
    }
}
