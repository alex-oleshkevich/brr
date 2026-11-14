use axum::{
    http::{StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

const INDEX: &str = "index.html";
const NOT_BUILT: &str = "web UI not built; run just web-build";

#[derive(RustEmbed)]
#[folder = "../../web/build"]
#[allow_missing = true]
struct Assets;

pub fn serve(path: &str) -> Response {
    let wanted = path.trim_start_matches('/');
    // Unknown paths get the app shell: the SPA routes them on the client.
    let name = if Assets::get(wanted).is_some() {
        wanted
    } else {
        INDEX
    };
    let Some(file) = Assets::get(name) else {
        return (StatusCode::NOT_FOUND, NOT_BUILT).into_response();
    };

    ([(CONTENT_TYPE, file.metadata.mimetype())], file.data).into_response()
}
