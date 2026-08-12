use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// Embedded build output of the React SPA (see `frontend/`). `cast run` ships
/// this inside the binary, so end users never build or host the frontend.
#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct Assets;

/// Serve the embedded SPA. Real files serve directly; unknown paths fall back
/// to index.html so client-side routing works. Unknown `/api/*` paths return a
/// JSON 404 instead of falling through to the SPA (so API clients get a proper
/// error, not an HTML page).
pub(crate) async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    // Unknown API routes get a JSON 404, never the SPA fallback.
    if path.starts_with("api/") {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"error\":\"not found\"}",
        )
            .into_response();
    }

    match Assets::get(path) {
        Some(file) => {
            let mime = mime_for(path);
            ([(header::CONTENT_TYPE, mime)], file.data).into_response()
        }
        None => {
            // SPA route fallback: a bare path with no extension -> index.html.
            if !path.contains('.') {
                if let Some(index) = Assets::get("index.html") {
                    return ([(header::CONTENT_TYPE, mime_for("index.html"))], index.data)
                        .into_response();
                }
            }
            (StatusCode::NOT_FOUND, "not found").into_response()
        }
    }
}

fn mime_for(path: &str) -> HeaderValue {
    let mime = match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "webmanifest" => "application/manifest+json",
        "map" => "application/json",
        _ => "application/octet-stream",
    };
    HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream"))
}
