use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use crate::config::Config;
use tower_http::services::ServeDir;

pub mod handlers;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(handlers::index))
        .route("/api/:mount/tree/*path", get(handlers::api_get_folder_tree))
        .route("/api/:mount/zip", post(handlers::handle_zip_download))
        .nest_service("/static", ServeDir::new("static"))
        .route("/:mount", get(handlers::handle_mount_root))
        .route("/:mount/*path", get(handlers::handle_mount_path))
        .with_state(state)
}