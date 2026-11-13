//! `eushully-asset://` — binary payloads for the Browse & Extract workspace
//! (decoded preview images as PNG), so pixels never go through JSON IPC.
//! Frontend: `convertFileSrc("<session>/<index>/image", "eushully-asset")`.
//! Requests are answered off the main thread.

use std::sync::Arc;

use tauri::http::{header, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use super::commands::{build_preview, PreviewResult};
use super::route::{parse_route, AssetKind};
use super::AssetsState;

pub const SCHEME: &str = "eushully-asset";

fn plain_response(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(message.as_bytes().to_vec())
        .expect("static response parts are valid")
}

fn png_response(png: &[u8]) -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        // Immutable per session + entry index.
        .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
        .body(png.to_vec())
        .expect("static response parts are valid")
}

fn respond<R: Runtime>(app: &tauri::AppHandle<R>, path: &str) -> Response<Vec<u8>> {
    let Some(route) = parse_route(path) else {
        return plain_response(StatusCode::BAD_REQUEST, "expected <session>/<index>/image");
    };
    let Some(state) = app.try_state::<AssetsState>() else {
        return plain_response(StatusCode::INTERNAL_SERVER_ERROR, "assets state missing");
    };
    let session = match state.session(route.session) {
        Ok(session) => session,
        Err(message) => return plain_response(StatusCode::NOT_FOUND, &message),
    };
    match route.kind {
        AssetKind::Image => {
            let png = session.cached_image(route.entry_index).or_else(|| match build_preview(&session, route.entry_index) {
                PreviewResult::Image { .. } => session.cached_image(route.entry_index),
                _ => None,
            });
            match png {
                Some(png) => png_response(&png),
                None => plain_response(StatusCode::NOT_FOUND, "entry is not an image"),
            }
        }
    }
}

/// Register with `Builder::register_asynchronous_uri_scheme_protocol(SCHEME, handle_request)`.
pub fn handle_request<R: Runtime>(context: UriSchemeContext<'_, R>, request: tauri::http::Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = Arc::new(context.app_handle().clone());
    let path = request.uri().path().to_string();
    std::thread::spawn(move || responder.respond(respond(&app, &path)));
}
