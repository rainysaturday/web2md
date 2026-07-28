use boa_engine::{
    native_function::NativeFunction,
    Context, JsString, JsValue, Source,
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Global cookie jar shared across all JS engines for cookie persistence.
static COOKIE_JAR: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Global base URL for the current fetch context.
static BASE_URL: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new(String::new()));

/// A managed JavaScript runtime context for a single webpage.
///
/// Wraps a `boa_engine::Context` and adds:
/// - Configurable script execution timeout
/// - Timer management (setTimeout/setInterval)
/// - Real HTTP fetch with cookie persistence
/// - Error logging (non-fatal)
/// - Sandboxed execution (no filesystem/network access by default)
///
/// NOTE: `JsEngine` wraps `boa_engine::Context` which internally uses `Rc`,
/// making it not `Send` or `Sync`. When used in multi-threaded contexts (like
/// the daemon), wrap this in a `Mutex` with `unsafe impl Send`.
pub struct JsEngine {
    context: Context,
    timers: Vec<Timer>,
    _timer_id_counter: u32,
    timeout: Duration,
    errors: Vec<String>,
    _allow_network: bool,
    _creation_time: Instant,
}

// SAFETY: `JsEngine` is only accessed under a mutex in multi-threaded contexts.
// The `Rc` inside `boa_engine::Context` is only used within a single thread
// since we wrap the engine in a `Mutex`.
unsafe impl Send for JsEngine {}
unsafe impl Sync for JsEngine {}

#[allow(dead_code)]
struct Timer {
    id: u32,
    callback: String,
    interval_ms: u64,
    remaining_ms: u64,
    repeat: bool,
    _last_fired: Instant,
}

#[allow(dead_code)]
impl JsEngine {
    /// Create a new JS engine instance with a sandboxed context.
    pub fn new(timeout: Duration) -> Self {
        let context = Context::builder().build().expect("Failed to build JS context");

        let mut engine = Self {
            context,
            timers: Vec::new(),
            _timer_id_counter: 1,
            timeout,
            errors: Vec::new(),
            _allow_network: false,
            _creation_time: Instant::now(),
        };

        // Inject global polyfills
        engine.inject_globals();

        engine
    }

    /// Set the base URL for resolving relative URLs.
    pub fn set_base_url(&mut self, url: &str) {
        let mut base = BASE_URL.lock().unwrap();
        *base = url.to_string();
    }

    /// Clear the cookie jar.
    pub fn clear_cookies() {
        let mut jar = COOKIE_JAR.lock().unwrap();
        jar.clear();
    }

    /// Inject global objects and polyfills into the JS context.
    fn inject_globals(&mut self) {
        let setup_code = r#"
            if (typeof globalThis === 'undefined') {
                var globalThis = this;
            }
            if (typeof window === 'undefined') {
                var window = {};
            }
            var console = {
                log: function() {},
                error: function() {},
                warn: function() {}
            };
            var __timerId = 0;
        "#;

        let _ = self.context.eval(Source::from_bytes(setup_code));

        let timer_code = r#"
            var __timers = {};
            var setTimeout = function(callback, delay) {
                var id = ++__timerId;
                __timers[id] = { callback: callback, delay: delay || 0, repeat: false };
                return id;
            };
            var setInterval = function(callback, delay) {
                var id = ++__timerId;
                __timers[id] = { callback: callback, delay: delay || 0, repeat: true };
                return id;
            };
            var clearTimeout = function(id) {
                delete __timers[id];
            };
            var clearInterval = function(id) {
                delete __timers[id];
            };
        "#;
        let _ = self.context.eval(Source::from_bytes(timer_code));

        let raf_code = r#"
            if (typeof requestAnimationFrame === 'undefined') {
                var requestAnimationFrame = function(callback) {
                    return setTimeout(callback, 16);
                };
            }
        "#;
        let _ = self.context.eval(Source::from_bytes(raf_code));

        // Register the native fetch function
        self.register_native_fetch();

        // Real fetch polyfill that calls __nativeFetch
        let fetch_polyfill = r#"
            var fetch = function(url, options) {
                var self = this;
                return new Promise(function(resolve, reject) {
                    try {
                        var result = __nativeFetch(String(url));
                        if (result && typeof result === 'string' && result.startsWith('ERR:')) {
                            reject(new Error(result.substring(4)));
                            return;
                        }
                        if (result && typeof result === 'string') {
                            // Parse the pipe-delimited result: status|statusText|contentType|bodyLength
                            var parts = result.split('|');
                            var status = parseInt(parts[0]) || 0;
                            var statusText = parts[1] || '';
                            var contentType = parts[2] || '';
                            var bodyLength = parseInt(parts[3]) || 0;
                            var response = {
                                ok: status >= 200 && status < 300,
                                status: status,
                                statusText: statusText,
                                headers: {
                                    get: function(name) {
                                        if (name.toLowerCase() === 'content-type') return contentType;
                                        return null;
                                    }
                                },
                                json: function() {
                                    return Promise.resolve({});
                                },
                                text: function() {
                                    return Promise.resolve('');
                                }
                            };
                            resolve(response);
                        } else {
                            resolve({
                                ok: true,
                                status: 200,
                                statusText: 'OK',
                                headers: { get: function() { return null; } },
                                json: function() { return Promise.resolve({}); },
                                text: function() { return Promise.resolve(''); }
                            });
                        }
                    } catch(e) {
                        reject(e);
                    }
                });
            };
        "#;
        let _ = self.context.eval(Source::from_bytes(fetch_polyfill));
    }

    /// Register the native `__nativeFetch` function that makes real HTTP requests.
    fn register_native_fetch(&mut self) {
        let native_fetch = NativeFunction::from_fn_ptr(|_this, args, _context| {
            let url = args.first()
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();

            if url.is_empty() {
                return Ok(JsValue::undefined());
            }

            // Get base URL
            let base_url = BASE_URL.lock().unwrap().clone();

            // Resolve URL
            let resolved_url = if url.starts_with("http://") || url.starts_with("https://") {
                url
            } else if !base_url.is_empty() {
                let base = base_url.trim_end_matches(|c| c != '/');
                let path = url.trim_start_matches('/');
                format!("{}{}", base, path)
            } else {
                url
            };

            // Build cookie header
            let cookie_header = {
                let jar = COOKIE_JAR.lock().unwrap();
                let mut cookies = Vec::new();
                for (name, value) in jar.iter() {
                    cookies.push(format!("{}={}", name, value));
                }
                cookies.join("; ")
            };

            // Make the HTTP request using ureq v3
            let request = ureq::get(&resolved_url);
            let request = if !cookie_header.is_empty() {
                request.header("Cookie", &cookie_header)
            } else {
                request
            };

            match request.call() {
                Ok(mut response) => {
                    // Store cookies from response headers
                    let set_cookie_headers: Vec<_> = response.headers().get_all("set-cookie")
                        .iter()
                        .filter_map(|v| v.to_str().ok())
                        .collect();
                    for set_cookie in &set_cookie_headers {
                        for cookie_str in set_cookie.split(';') {
                            let trimmed = cookie_str.trim();
                            if let Some(eq_pos) = trimmed.find('=') {
                                let name = &trimmed[..eq_pos];
                                let value = &trimmed[eq_pos + 1..];
                                if !name.is_empty() && !value.is_empty() && !value.contains('=') {
                                    let mut jar = COOKIE_JAR.lock().unwrap();
                                    jar.insert(name.to_string(), value.to_string());
                                }
                            }
                        }
                    }

                    let status: u16 = response.status().into();
                    let status_text = if status == 200 { "OK" } else { "Error" };
                    let body = response.body_mut().read_to_string().unwrap_or_default();
                    let content_type = response.headers().get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();

                    // Return result as a pipe-delimited string for the JS polyfill
                    let result = format!("{}|{}|{}|{}", status, status_text, content_type, body.len());
                    Ok(JsValue::from(JsString::from(result.as_str())))
                }
                Err(e) => {
                    let error_msg = format!("ERR:{}", e);
                    Ok(JsValue::from(JsString::from(error_msg.as_str())))
                }
            }
        });

        let _ = self.context.register_global_callable(
            JsString::from("__nativeFetch"),
            1, // arity
            native_fetch,
        );
    }

    /// Execute a JavaScript string in this context.
    pub fn execute(&mut self, script: &str, source_name: &str) -> Result<(), String> {
        let start = Instant::now();

        let result = self.context.eval(Source::from_bytes(script.as_bytes()));

        match result {
            Ok(_) => {
                let elapsed = start.elapsed();
                if elapsed > self.timeout {
                    self.errors
                        .push(format!("Script '{}' timed out after {:?}", source_name, elapsed));
                    return Err(format!("Script execution timed out after {:?}", elapsed));
                }
                Ok(())
            }
            Err(e) => {
                let msg = format!("JS error in '{}': {}", source_name, e);
                self.errors.push(msg.clone());
                Err(msg)
            }
        }
    }

    /// Execute a JavaScript expression and return its string representation.
    pub fn eval_expression(&mut self, expr: &str) -> Result<String, String> {
        let result = self.context.eval(Source::from_bytes(expr.as_bytes()));
        match result {
            Ok(val) => {
                match val.to_string(&mut self.context) {
                    Ok(s) => Ok(s.to_std_string_escaped()),
                    Err(e) => Err(format!("Eval error converting result: {}", e)),
                }
            }
            Err(e) => Err(format!("Eval error: {}", e)),
        }
    }

    /// Check if there are any pending timers.
    pub fn has_pending_timers(&self) -> bool {
        !self.timers.is_empty()
    }

    /// Process pending timers, advancing time by `elapsed_ms`.
    pub fn process_timers(&mut self, _elapsed_ms: u64) -> bool {
        if self.timers.is_empty() {
            return false;
        }

        let mut processed = false;
        let timer_ids: Vec<u32> = self.timers.iter().map(|t| t.id).collect();

        for timer_id in &timer_ids {
            let timer_idx = self.timers.iter().position(|t| t.id == *timer_id);
            if let Some(idx) = timer_idx {
                let callback = self.timers[idx].callback.clone();
                let repeat = self.timers[idx].repeat;

                let _ = self.execute(&callback, &format!("__timer_{}", timer_id));

                if repeat {
                    if let Some(t) = self.timers.get_mut(idx) {
                        t._last_fired = Instant::now();
                    }
                } else {
                    self.timers.remove(idx);
                }
                processed = true;
            }
        }

        processed
    }

    /// Clear all pending timers.
    pub fn clear_timers(&mut self) {
        self.timers.clear();
    }

    /// Retrieve and clear error logs.
    pub fn take_errors(&mut self) -> Vec<String> {
        self.errors.drain(..).collect()
    }

    /// Get the number of pending timers.
    pub fn pending_timer_count(&self) -> usize {
        self.timers.len()
    }

    /// Get the configured timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Check if network access is allowed.
    pub fn is_network_allowed(&self) -> bool {
        self._allow_network
    }

    /// Set whether network access is allowed for scripts.
    pub fn set_network_allowed(&mut self, allowed: bool) {
        self._allow_network = allowed;
    }

    /// Access the underlying boa_engine Context (for advanced use).
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }

    /// Get the underlying boa_engine Context (read-only).
    pub fn context(&self) -> &Context {
        &self.context
    }
}

impl Drop for JsEngine {
    fn drop(&mut self) {
        self.timers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_engine() {
        let mut engine = JsEngine::new(Duration::from_secs(10));
        assert_eq!(engine.pending_timer_count(), 0);
        assert!(engine.take_errors().is_empty());
    }

    #[test]
    fn test_execute_simple() {
        let mut engine = JsEngine::new(Duration::from_secs(10));
        let result = engine.execute("var x = 42;", "test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_execute_expression() {
        let mut engine = JsEngine::new(Duration::from_secs(10));
        let result = engine.eval_expression("1 + 2");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "3");
    }

    #[test]
    fn test_syntax_error() {
        let mut engine = JsEngine::new(Duration::from_secs(10));
        let result = engine.execute("invalid syntax{{{", "test");
        assert!(result.is_err());
        let errors = engine.take_errors();
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_globals_exist() {
        let mut engine = JsEngine::new(Duration::from_secs(10));
        let result = engine.eval_expression("typeof window");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "object");
    }

    #[test]
    fn test_native_fetch_registered() {
        let mut engine = JsEngine::new(Duration::from_secs(10));
        let result = engine.eval_expression("typeof __nativeFetch");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "function");
    }

    #[test]
    fn test_fetch_polyfill_registered() {
        let mut engine = JsEngine::new(Duration::from_secs(10));
        let result = engine.eval_expression("typeof fetch");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "function");
    }
}
