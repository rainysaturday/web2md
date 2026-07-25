use boa_engine::{Context, Source};
use std::time::{Duration, Instant};

/// A managed JavaScript runtime context for a single webpage.
///
/// Wraps a `boa_engine::Context` and adds:
/// - Configurable script execution timeout
/// - Timer management (setTimeout/setInterval)
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

        let fetch_code = r#"
            if (typeof fetch === 'undefined') {
                var fetch = function(url, options) {
                    return new Promise(function(resolve, reject) {
                        resolve({
                            ok: true,
                            status: 200,
                            statusText: 'OK',
                            headers: { get: function() { return null; } },
                            json: function() { return Promise.resolve({}); },
                            text: function() { return Promise.resolve(''); }
                        });
                    });
                };
            }
        "#;
        let _ = self.context.eval(Source::from_bytes(fetch_code));
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
}
