use crate::page::{Page, RenderConfig};
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Shared application state for the daemon.
pub struct DaemonState {
    pages: Mutex<HashMap<String, Page>>,
    start_time: Instant,
    request_count: Mutex<u64>,
    _total_render_time: Mutex<Duration>,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            pages: Mutex::new(HashMap::new()),
            start_time: Instant::now(),
            request_count: Mutex::new(0),
            _total_render_time: Mutex::new(Duration::ZERO),
        }
    }

    fn increment_requests(&self) {
        if let Ok(mut count) = self.request_count.lock() {
            *count += 1;
        }
    }
}

// --- Request/Response types ---

#[derive(Deserialize)]
pub struct FetchPageRequest {
    pub url: String,
    #[serde(default)]
    pub with_images: Option<bool>,
    #[serde(default)]
    pub render_timeout: Option<u64>,
    #[serde(default)]
    pub quiet_period: Option<u64>,
    #[serde(default)]
    pub enable_js: Option<bool>,
}

#[derive(Serialize)]
pub struct PageResponse {
    pub page_id: String,
    pub url: String,
    pub state: String,
    pub timed_out: bool,
    pub error_count: usize,
    pub created_at_secs: f64,
}

#[derive(Serialize)]
pub struct PageListResponse {
    pub pages: Vec<PageResponse>,
    pub count: usize,
}

#[derive(Serialize)]
pub struct RenderResponse {
    pub html: String,
    pub state: String,
    pub timed_out: bool,
}

#[derive(Serialize)]
pub struct MarkdownResponse {
    pub markdown: String,
    pub page_id: String,
}

#[derive(Deserialize)]
pub struct EvalRequest {
    pub script: String,
}

#[derive(Serialize)]
pub struct EvalResponse {
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub uptime_secs: f64,
    pub page_count: usize,
    pub request_count: u64,
}

#[derive(Serialize)]
pub struct MetricsResponse {
    pub requests_handled: u64,
    pub pages_created: usize,
    pub total_render_time_secs: f64,
    pub uptime_secs: f64,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// --- Handlers ---

async fn fetch_page(
    state: web::Data<Arc<DaemonState>>,
    body: web::Json<FetchPageRequest>,
) -> impl Responder {
    state.increment_requests();

    let config = RenderConfig {
        enable_js: body.enable_js.unwrap_or(true),
        with_images: body.with_images.unwrap_or(false),
        render_timeout: Duration::from_secs(body.render_timeout.unwrap_or(30)),
        quiet_period: Duration::from_millis(body.quiet_period.unwrap_or(100)),
        ..Default::default()
    };

    let html = match fetch_url(&body.url) {
        Ok(h) => h,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Failed to fetch URL: {}", e),
            })
        }
    };

    let mut page = Page::new(body.url.clone(), html, config);

    match page.process() {
        Ok(_) => {}
        Err(e) => {
            log::warn!("Page processing warning: {}", e);
        }
    }

    let page_id = page.id().to_string();
    let state_str = format!("{:?}", page.state());
    let timed_out = page.timed_out();
    let error_count = page.errors().len();
    let created_at = page.created_at().elapsed().as_secs_f64();

    if let Ok(mut pages) = state.pages.lock() {
        pages.insert(page_id.clone(), page);
    }

    HttpResponse::Ok().json(PageResponse {
        page_id,
        url: body.url.clone(),
        state: state_str,
        timed_out,
        error_count,
        created_at_secs: created_at,
    })
}

async fn get_page(
    state: web::Data<Arc<DaemonState>>,
    path: web::Path<String>,
) -> impl Responder {
    state.increment_requests();

    let page_id = path.into_inner();
    let pages = match state.pages.lock() {
        Ok(p) => p,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Failed to acquire lock".to_string(),
            })
        }
    };

    match pages.get(&page_id) {
        Some(page) => HttpResponse::Ok().json(PageResponse {
            page_id: page.id().to_string(),
            url: page.url().to_string(),
            state: format!("{:?}", page.state()),
            timed_out: page.timed_out(),
            error_count: page.errors().len(),
            created_at_secs: page.created_at().elapsed().as_secs_f64(),
        }),
        None => HttpResponse::NotFound().json(ErrorResponse {
            error: format!("Page '{}' not found", page_id),
        }),
    }
}

async fn delete_page(
    state: web::Data<Arc<DaemonState>>,
    path: web::Path<String>,
) -> impl Responder {
    state.increment_requests();

    let page_id = path.into_inner();
    match state.pages.lock() {
        Ok(mut pages) => {
            if pages.remove(&page_id).is_some() {
                HttpResponse::Ok().json(serde_json::json!({ "deleted": true, "page_id": page_id }))
            } else {
                HttpResponse::NotFound().json(ErrorResponse {
                    error: format!("Page '{}' not found", page_id),
                })
            }
        }
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "Failed to acquire lock".to_string(),
        }),
    }
}

async fn list_pages(state: web::Data<Arc<DaemonState>>) -> impl Responder {
    state.increment_requests();

    let pages = match state.pages.lock() {
        Ok(p) => p,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Failed to acquire lock".to_string(),
            })
        }
    };

    let page_list: Vec<PageResponse> = pages
        .values()
        .map(|page| PageResponse {
            page_id: page.id().to_string(),
            url: page.url().to_string(),
            state: format!("{:?}", page.state()),
            timed_out: page.timed_out(),
            error_count: page.errors().len(),
            created_at_secs: page.created_at().elapsed().as_secs_f64(),
        })
        .collect();

    let count = page_list.len();
    HttpResponse::Ok().json(PageListResponse {
        pages: page_list,
        count,
    })
}

async fn delete_all_pages(state: web::Data<Arc<DaemonState>>) -> impl Responder {
    state.increment_requests();

    match state.pages.lock() {
        Ok(mut pages) => {
            let count = pages.len();
            pages.clear();
            HttpResponse::Ok().json(serde_json::json!({ "deleted": true, "count": count }))
        }
        Err(_) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: "Failed to acquire lock".to_string(),
        }),
    }
}

async fn render_page(
    state: web::Data<Arc<DaemonState>>,
    path: web::Path<String>,
) -> impl Responder {
    state.increment_requests();

    let page_id = path.into_inner();
    let mut pages = match state.pages.lock() {
        Ok(p) => p,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Failed to acquire lock".to_string(),
            })
        }
    };

    match pages.get_mut(&page_id) {
        Some(page) => {
            let html = page.render_now();
            let state_str = format!("{:?}", page.state());
            let timed_out = page.timed_out();

            HttpResponse::Ok().json(RenderResponse {
                html,
                state: state_str,
                timed_out,
            })
        }
        None => HttpResponse::NotFound().json(ErrorResponse {
            error: format!("Page '{}' not found", page_id),
        }),
    }
}

async fn markdown_page(
    state: web::Data<Arc<DaemonState>>,
    path: web::Path<String>,
) -> impl Responder {
    state.increment_requests();

    let page_id = path.into_inner();
    let pages = match state.pages.lock() {
        Ok(p) => p,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Failed to acquire lock".to_string(),
            })
        }
    };

    match pages.get(&page_id) {
        Some(page) => {
            let markdown = page.to_markdown();
            HttpResponse::Ok().json(MarkdownResponse {
                markdown,
                page_id: page.id().to_string(),
            })
        }
        None => HttpResponse::NotFound().json(ErrorResponse {
            error: format!("Page '{}' not found", page_id),
        }),
    }
}

async fn render_now_page(
    state: web::Data<Arc<DaemonState>>,
    path: web::Path<String>,
) -> impl Responder {
    state.increment_requests();

    let page_id = path.into_inner();
    let mut pages = match state.pages.lock() {
        Ok(p) => p,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Failed to acquire lock".to_string(),
            })
        }
    };

    match pages.get_mut(&page_id) {
        Some(page) => {
            let html = page.render_now();
            let state_str = format!("{:?}", page.state());

            HttpResponse::Ok().json(RenderResponse {
                html,
                state: state_str,
                timed_out: page.timed_out(),
            })
        }
        None => HttpResponse::NotFound().json(ErrorResponse {
            error: format!("Page '{}' not found", page_id),
        }),
    }
}

async fn eval_js(
    state: web::Data<Arc<DaemonState>>,
    path: web::Path<String>,
    body: web::Json<EvalRequest>,
) -> impl Responder {
    state.increment_requests();

    let page_id = path.into_inner();
    let mut pages = match state.pages.lock() {
        Ok(p) => p,
        Err(_) => {
            return HttpResponse::InternalServerError().json(EvalResponse {
                result: None,
                error: Some("Failed to acquire lock".to_string()),
            })
        }
    };

    match pages.get_mut(&page_id) {
        Some(page) => match page.eval_js(&body.script) {
            Ok(result) => HttpResponse::Ok().json(EvalResponse {
                result: Some(result),
                error: None,
            }),
            Err(e) => HttpResponse::Ok().json(EvalResponse {
                result: None,
                error: Some(e),
            }),
        },
        None => HttpResponse::NotFound().json(EvalResponse {
            result: None,
            error: Some(format!("Page '{}' not found", page_id)),
        }),
    }
}

async fn eval_js_file(
    state: web::Data<Arc<DaemonState>>,
    path: web::Path<String>,
    body: web::Json<EvalRequest>,
) -> impl Responder {
    state.increment_requests();

    let page_id = path.into_inner();
    let script_path = &body.script;

    let script_content = match std::fs::read_to_string(script_path) {
        Ok(content) => content,
        Err(e) => {
            return HttpResponse::BadRequest().json(EvalResponse {
                result: None,
                error: Some(format!("Failed to read file '{}': {}", script_path, e)),
            })
        }
    };

    let mut pages = match state.pages.lock() {
        Ok(p) => p,
        Err(_) => {
            return HttpResponse::InternalServerError().json(EvalResponse {
                result: None,
                error: Some("Failed to acquire lock".to_string()),
            })
        }
    };

    match pages.get_mut(&page_id) {
        Some(page) => match page.execute_js(&script_content, script_path) {
            Ok(()) => HttpResponse::Ok().json(EvalResponse {
                result: Some(format!("Executed '{}' successfully", script_path)),
                error: None,
            }),
            Err(e) => HttpResponse::Ok().json(EvalResponse {
                result: None,
                error: Some(e),
            }),
        },
        None => HttpResponse::NotFound().json(EvalResponse {
            result: None,
            error: Some(format!("Page '{}' not found", page_id)),
        }),
    }
}

async fn click_element(
    state: web::Data<Arc<DaemonState>>,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> impl Responder {
    state.increment_requests();

    let page_id = path.into_inner();
    let selector = body
        .get("selector")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut pages = match state.pages.lock() {
        Ok(p) => p,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Failed to acquire lock".to_string(),
            })
        }
    };

    match pages.get_mut(&page_id) {
        Some(page) => match page.simulate_click(&selector) {
            Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "clicked": selector })),
            Err(e) => HttpResponse::Ok().json(serde_json::json!({ "error": e })),
        },
        None => HttpResponse::NotFound().json(ErrorResponse {
            error: format!("Page '{}' not found", page_id),
        }),
    }
}

async fn fill_element(
    state: web::Data<Arc<DaemonState>>,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> impl Responder {
    state.increment_requests();

    let page_id = path.into_inner();
    let selector = body
        .get("selector")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let value = body
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut pages = match state.pages.lock() {
        Ok(p) => p,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Failed to acquire lock".to_string(),
            })
        }
    };

    match pages.get_mut(&page_id) {
        Some(page) => match page.simulate_fill(&selector, &value) {
            Ok(()) => HttpResponse::Ok().json(serde_json::json!({ "filled": selector, "value": value })),
            Err(e) => HttpResponse::Ok().json(serde_json::json!({ "error": e })),
        },
        None => HttpResponse::NotFound().json(ErrorResponse {
            error: format!("Page '{}' not found", page_id),
        }),
    }
}

async fn health(state: web::Data<Arc<DaemonState>>) -> impl Responder {
    state.increment_requests();

    let page_count = match state.pages.lock() {
        Ok(pages) => pages.len(),
        Err(_) => 0,
    };
    let request_count = match state.request_count.lock() {
        Ok(count) => *count,
        Err(_) => 0,
    };

    HttpResponse::Ok().json(HealthResponse {
        status: "ok".to_string(),
        uptime_secs: state.start_time.elapsed().as_secs_f64(),
        page_count,
        request_count,
    })
}

async fn metrics_endpoint(state: web::Data<Arc<DaemonState>>) -> impl Responder {
    state.increment_requests();

    let page_count = match state.pages.lock() {
        Ok(pages) => pages.len(),
        Err(_) => 0,
    };
    let request_count = match state.request_count.lock() {
        Ok(count) => *count,
        Err(_) => 0,
    };

    HttpResponse::Ok().json(MetricsResponse {
        requests_handled: request_count,
        pages_created: page_count,
        total_render_time_secs: 0.0,
        uptime_secs: state.start_time.elapsed().as_secs_f64(),
    })
}

async fn shutdown() -> impl Responder {
    log::info!("Shutdown requested");
    HttpResponse::Ok().json(serde_json::json!({ "shutdown": true }))
}

// --- Server setup ---

/// Start the daemon HTTP server.
pub async fn start_daemon(
    port: Option<u16>,
    _socket_path: Option<String>,
    _background: bool,
) -> std::io::Result<()> {
    let state = Arc::new(DaemonState::new());
    let state_data = web::Data::new(state.clone());

    if _background {
        log::info!("Running in background mode");
    }

    let listen_port = port.unwrap_or(8765);
    log::info!("Starting daemon on http://127.0.0.1:{}", listen_port);

    let server = HttpServer::new(move || {
        let state = state_data.clone();
        App::new()
            .app_data(state)
            .route("/health", web::get().to(health))
            .route("/metrics", web::get().to(metrics_endpoint))
            .route("/shutdown", web::post().to(shutdown))
            .route("/page", web::post().to(fetch_page))
            .route("/pages", web::get().to(list_pages))
            .route("/pages", web::delete().to(delete_all_pages))
            .route("/page/{id}", web::get().to(get_page))
            .route("/page/{id}", web::delete().to(delete_page))
            .route("/page/{id}/render", web::get().to(render_page))
            .route("/page/{id}/markdown", web::get().to(markdown_page))
            .route("/page/{id}/render-now", web::post().to(render_now_page))
            .route("/page/{id}/eval", web::post().to(eval_js))
            .route("/page/{id}/eval-file", web::post().to(eval_js_file))
            .route("/page/{id}/click", web::post().to(click_element))
            .route("/page/{id}/fill", web::post().to(fill_element))
    })
    .bind(("127.0.0.1", listen_port))?
    .run();

    log::info!("Daemon is ready to accept requests");
    server.await
}

// --- Client functions ---

pub fn client_fetch_page(daemon_url: &str, url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();
    let body = serde_json::json!({ "url": url });

    let response = client
        .post(format!("{}/page", daemon_url))
        .json(&body)
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    response
        .text()
        .map_err(|e| format!("Failed to read response: {}", e))
}

pub fn client_render_page(daemon_url: &str, page_id: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();

    let response = client
        .get(format!("{}/page/{}/render", daemon_url, page_id))
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    response
        .text()
        .map_err(|e| format!("Failed to read response: {}", e))
}

pub fn client_markdown_page(daemon_url: &str, page_id: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();

    let response = client
        .get(format!("{}/page/{}/markdown", daemon_url, page_id))
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    response
        .text()
        .map_err(|e| format!("Failed to read response: {}", e))
}

pub fn client_eval_js(
    daemon_url: &str,
    page_id: &str,
    script: &str,
) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();
    let body = serde_json::json!({ "script": script });

    let response = client
        .post(format!("{}/page/{}/eval", daemon_url, page_id))
        .json(&body)
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    response
        .text()
        .map_err(|e| format!("Failed to read response: {}", e))
}

pub fn client_click_element(
    daemon_url: &str,
    page_id: &str,
    selector: &str,
) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();
    let body = serde_json::json!({ "selector": selector });

    let response = client
        .post(format!("{}/page/{}/click", daemon_url, page_id))
        .json(&body)
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    response
        .text()
        .map_err(|e| format!("Failed to read response: {}", e))
}

pub fn client_fill_element(
    daemon_url: &str,
    page_id: &str,
    selector: &str,
    value: &str,
) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();
    let body = serde_json::json!({ "selector": selector, "value": value });

    let response = client
        .post(format!("{}/page/{}/fill", daemon_url, page_id))
        .json(&body)
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    response
        .text()
        .map_err(|e| format!("Failed to read response: {}", e))
}

pub fn client_close_page(daemon_url: &str, page_id: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();

    let response = client
        .delete(format!("{}/page/{}", daemon_url, page_id))
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    response
        .text()
        .map_err(|e| format!("Failed to read response: {}", e))
}

pub fn client_health(daemon_url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::new();

    let response = client
        .get(format!("{}/health", daemon_url))
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    response
        .text()
        .map_err(|e| format!("Failed to read response: {}", e))
}

// --- Helper ---

fn fetch_url(url: &str) -> Result<String, String> {
    reqwest::blocking::get(url)
        .map_err(|e| format!("Failed to fetch URL: {}", e))?
        .text()
        .map_err(|e| format!("Failed to read response body: {}", e))
}
