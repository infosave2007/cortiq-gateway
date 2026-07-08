//! Embedded static assets for the admin panel (SPA). The `web/` directory is compiled
//! into the binary via `rust-embed`, so the gateway remains a single self-contained file.
//!
//! Served as the main router's fallback: everything under `/admin/*` not matched by
//! an explicit route is served as a file; an unknown path under `/admin` falls back to
//! `index.html` (the client-side hash router handles navigation).

use axum::{
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/"]
struct WebAssets;

fn serve_file(path: &str) -> Response {
    // The panel is rebuilt often (assets are embedded at compile time and served
    // from memory), so tell the browser to always revalidate — otherwise a stale
    // cached SPA hides freshly shipped fixes until a hard reload.
    const NO_CACHE: &str = "no-store, must-revalidate";
    match WebAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                [
                    (header::CONTENT_TYPE, mime.as_ref()),
                    (header::CACHE_CONTROL, NO_CACHE),
                ],
                content.data.into_owned(),
            )
                .into_response()
        }
        None => {
            // SPA: unknown path under /admin → serve index.html
            match WebAssets::get("index.html") {
                Some(content) => (
                    [
                        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (header::CACHE_CONTROL, NO_CACHE),
                    ],
                    content.data.into_owned(),
                )
                    .into_response(),
                None => (StatusCode::NOT_FOUND, "not found").into_response(),
            }
        }
    }
}

/// Application-wide fallback: serves `/admin/*`; everything else → 404 JSON.
pub async fn fallback(uri: Uri) -> Response {
    let path = uri.path();
    if let Some(rest) = path.strip_prefix("/admin") {
        let rest = rest.trim_start_matches('/');
        let file = if rest.is_empty() { "index.html" } else { rest };
        serve_file(file)
    } else {
        (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": { "message": "not found", "type": "not_found", "code": "not_found" }
            })),
        )
            .into_response()
    }
}
