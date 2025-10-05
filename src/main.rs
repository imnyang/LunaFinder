mod auth;
mod config;

use actix_files::NamedFile;
use actix_multipart::Multipart;
use actix_web::{
    cookie::{time::Duration, Cookie},
    error,
    http::header,
    middleware::Logger,
    web, App, HttpRequest, HttpResponse, HttpServer,
};
use anyhow::{anyhow, Context as AnyhowContext};
use futures_util::TryStreamExt as _;
use pulldown_cmark::{html, Options, Parser};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tera::{Context as TeraContext, Tera};

use auth::verify_password;
use config::{Config, MountConfig, Permission};

type ActixResult<T> = Result<T, actix_web::Error>;

const SESSION_COOKIE: &str = "lunafinder_session";
const TREE_MAX_DEPTH: usize = 12;

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    tera: Arc<Tera>,
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct RenameForm {
    target_path: String,
    new_name: String,
}

#[derive(Deserialize)]
struct DeleteForm {
    target_path: String,
}

#[derive(Deserialize)]
struct EditForm {
    content: String,
}

#[derive(Serialize)]
struct FileEntry {
    name: String,
    is_dir: bool,
    size: Option<u64>,
}

#[derive(Serialize)]
struct DirectoryNode {
    name: String,
    path: String,
    children: Vec<DirectoryNode>,
}

#[derive(Serialize)]
struct MountSummary {
    name: String,
    description: String,
    public: bool,
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    env_logger::init();

    let mut config = Config::load_or_create("config.toml")?;
    ensure_mount_directories(&config)?;

    config = Config::load_or_create("config.toml")?;

    let tera = Tera::new("templates/**/*").context("Failed to load templates")?;

    let state = AppState {
        config: Arc::new(config),
        tera: Arc::new(tera),
    };

    let server_host = state.config.server.host.clone();
    let server_port = state.config.server.port;

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .app_data(web::Data::new(state.clone()))
            .route("/", web::get().to(index))
            .service(
                web::resource("/login")
                    .route(web::get().to(login_page))
                    .route(web::post().to(login)),
            )
            .route("/logout", web::get().to(logout))
            .service(
                web::scope("/browse")
                    .route("/{mount}/{tail:.*}", web::get().to(browse))
                    .route("/{mount}/{tail:.*}/upload", web::post().to(upload_file))
                    .route("/{mount}/{tail:.*}/delete", web::post().to(delete_entry))
                    .route("/{mount}/{tail:.*}/rename", web::post().to(rename_entry)),
            )
            .service(
                web::resource("/edit/{mount}/{tail:.*}")
                    .route(web::get().to(edit_page))
                    .route(web::post().to(edit_save)),
            )
    })
    .bind((server_host.as_str(), server_port))?
    .run()
    .await?;

    Ok(())
}

async fn index(state: web::Data<AppState>, req: HttpRequest) -> ActixResult<HttpResponse> {
    let username = get_username_from_cookie(&req);
    let config = &state.config;

    let markdown_content = if let Ok(markdown) = fs::read_to_string(&config.main_page.markdown_file)
    {
        Some(render_markdown(&markdown))
    } else {
        None
    };

    let mut mounts = Vec::new();
    for (name, mount) in &config.mounts {
        let permission = effective_permission(config, username.as_deref(), mount);
        if username.is_some() {
            if permission.is_some() {
                mounts.push(MountSummary {
                    name: name.clone(),
                    description: mount.description.clone(),
                    public: mount.public,
                });
            }
        } else if mount.public {
            mounts.push(MountSummary {
                name: name.clone(),
                description: mount.description.clone(),
                public: true,
            });
        }
    }

    mounts.sort_by(|a, b| a.name.cmp(&b.name));

    let mut context = TeraContext::new();
    context.insert("title", &config.main_page.title);
    context.insert("description", &config.main_page.description);
    context.insert("markdown_content", &markdown_content);
    context.insert("mounts", &mounts);
    if let Some(ref username) = username {
        context.insert("username", username);
    }

    let html = state
        .tera
        .render("index.html", &context)
        .map_err(error::ErrorInternalServerError)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

async fn login_page(state: web::Data<AppState>) -> ActixResult<HttpResponse> {
    let context = TeraContext::new();
    let html = state
        .tera
        .render("login.html", &context)
        .map_err(error::ErrorInternalServerError)?;
    Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

async fn login(
    state: web::Data<AppState>,
    form: web::Form<LoginForm>,
) -> ActixResult<HttpResponse> {
    let config = &state.config;
    let mut context = TeraContext::new();

    if let Some(user_config) = config.users.get(&form.username) {
        if !user_config.password.is_empty()
            && verify_password(
                &form.password,
                &user_config.password,
                &user_config.hash_algorithm,
            )
        {
            let mut response = HttpResponse::Found()
                .append_header((header::LOCATION, "/"))
                .finish();

            let cookie = Cookie::build(SESSION_COOKIE, form.username.clone())
                .http_only(true)
                .path("/")
                .max_age(Duration::hours(24))
                .finish();

            response
                .add_cookie(&cookie)
                .map_err(error::ErrorInternalServerError)?;

            return Ok(response);
        }
    }

    context.insert("error", &true);
    let html = state
        .tera
        .render("login.html", &context)
        .map_err(error::ErrorInternalServerError)?;

    Ok(HttpResponse::BadRequest()
        .content_type("text/html")
        .body(html))
}

async fn logout(req: HttpRequest) -> ActixResult<HttpResponse> {
    let mut response = HttpResponse::Found()
        .append_header((header::LOCATION, "/"))
        .finish();

    if req.cookie(SESSION_COOKIE).is_some() {
        let cookie = Cookie::build(SESSION_COOKIE, "")
            .path("/")
            .max_age(Duration::seconds(0))
            .finish();
        response
            .add_cookie(&cookie)
            .map_err(error::ErrorInternalServerError)?;
    }

    Ok(response)
}

async fn browse(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
) -> ActixResult<HttpResponse> {
    let (mount_name, tail) = path.into_inner();
    let config = &state.config;
    let mount = config
        .mounts
        .get(&mount_name)
        .ok_or_else(|| error::ErrorNotFound("Mount not found"))?;

    let username = get_username_from_cookie(&req);
    let permission = effective_permission(config, username.as_deref(), mount);

    let can_read = permission
        .as_ref()
        .map(|p| p.allows_read())
        .unwrap_or(false);

    if !can_read {
        return Ok(HttpResponse::Found()
            .append_header((header::LOCATION, "/login"))
            .finish());
    }

    let relative_path =
        normalize_relative_path(&tail).ok_or_else(|| error::ErrorBadRequest("Invalid path"))?;

    let base_path = canonicalize_mount(&mount.path).map_err(error::ErrorInternalServerError)?;
    let target_path = resolve_path(&base_path, &relative_path)
        .map_err(|e| error::ErrorBadRequest(e.to_string()))?;

    if target_path.is_file() {
        let file = NamedFile::open(&target_path).map_err(error::ErrorInternalServerError)?;
        return Ok(file.into_response(&req));
    }

    if !target_path.is_dir() {
        return Err(error::ErrorNotFound("Path not found"));
    }

    let can_write = permission
        .as_ref()
        .map(|p| p.allows_write())
        .unwrap_or(false);
    let permission_label = permission
        .as_ref()
        .map(|p| p.actions().join(", "))
        .unwrap_or_default();
    let has_permission = can_read;

    let entries = collect_entries(&target_path).map_err(error::ErrorInternalServerError)?;

    let current_path_string = if relative_path.as_os_str().is_empty() {
        ".".to_string()
    } else {
        pathbuf_to_string(&relative_path)
    };

    let parent_path = if relative_path.as_os_str().is_empty() {
        None
    } else {
        let mut parent = relative_path.clone();
        parent.pop();
        Some(if parent.as_os_str().is_empty() {
            ".".to_string()
        } else {
            pathbuf_to_string(&parent)
        })
    };

    let directory_tree = build_directory_tree(&base_path, Path::new(""), 0)
        .map_err(error::ErrorInternalServerError)?;
    let open_paths = build_open_paths(&current_path_string);

    let mut context = TeraContext::new();
    context.insert("mount_name", &mount_name);
    context.insert("mount_description", &mount.description);
    context.insert("current_path", &current_path_string);
    context.insert("entries", &entries);
    if let Some(parent_path) = &parent_path {
        context.insert("parent_path", parent_path);
    }
    if let Some(ref username) = username {
        context.insert("username", username);
    }
    context.insert("is_public", &mount.public);
    context.insert("can_write", &can_write);
    context.insert("has_permission", &has_permission);
    context.insert("permission", &permission_label);
    context.insert("tree", &directory_tree);
    context.insert("open_paths", &open_paths);

    let html = state
        .tera
        .render("browse.html", &context)
        .map_err(error::ErrorInternalServerError)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

async fn upload_file(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
    mut payload: Multipart,
) -> ActixResult<HttpResponse> {
    let (mount_name, tail) = path.into_inner();
    let config = &state.config;
    let mount = config
        .mounts
        .get(&mount_name)
        .ok_or_else(|| error::ErrorNotFound("Mount not found"))?;

    let username = get_username_from_cookie(&req);
    let permission = effective_permission(config, username.as_deref(), mount)
        .ok_or_else(|| error::ErrorForbidden("Write permission required"))?;
    if !permission.allows_upload() {
        return Err(error::ErrorForbidden("Write permission required"));
    }

    let relative_path =
        normalize_relative_path(&tail).ok_or_else(|| error::ErrorBadRequest("Invalid path"))?;

    let base_path = canonicalize_mount(&mount.path).map_err(error::ErrorInternalServerError)?;
    let directory_path = resolve_path(&base_path, &relative_path)
        .map_err(|e| error::ErrorBadRequest(e.to_string()))?;

    if !directory_path.is_dir() {
        return Err(error::ErrorBadRequest("Target is not a directory"));
    }

    while let Some(mut field) = payload
        .try_next()
        .await
        .map_err(error::ErrorInternalServerError)?
    {
        if let Some(filename) = field.content_disposition().and_then(|cd| cd.get_filename()) {
            if let Some(sanitized) = sanitize_file_name(filename) {
                let file_path = directory_path.join(sanitized);
                let mut file =
                    fs::File::create(&file_path).map_err(error::ErrorInternalServerError)?;
                while let Some(chunk) = field
                    .try_next()
                    .await
                    .map_err(error::ErrorInternalServerError)?
                {
                    file.write_all(&chunk)
                        .map_err(error::ErrorInternalServerError)?;
                }
            }
        }
    }

    Ok(HttpResponse::Found()
        .append_header((header::LOCATION, format!("/browse/{}/{}", mount_name, tail)))
        .finish())
}

async fn delete_entry(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
    form: web::Form<DeleteForm>,
) -> ActixResult<HttpResponse> {
    let (mount_name, tail) = path.into_inner();
    let config = &state.config;
    let mount = config
        .mounts
        .get(&mount_name)
        .ok_or_else(|| error::ErrorNotFound("Mount not found"))?;

    let username = get_username_from_cookie(&req);
    let permission = effective_permission(config, username.as_deref(), mount)
        .ok_or_else(|| error::ErrorForbidden("Write permission required"))?;
    if !permission.allows_delete() {
        return Err(error::ErrorForbidden("Write permission required"));
    }

    let current_relative =
        normalize_relative_path(&tail).ok_or_else(|| error::ErrorBadRequest("Invalid path"))?;
    let target_relative = normalize_relative_path(&form.target_path)
        .ok_or_else(|| error::ErrorBadRequest("Invalid target path"))?;

    let base_path = canonicalize_mount(&mount.path).map_err(error::ErrorInternalServerError)?;
    let current_directory = resolve_path(&base_path, &current_relative)
        .map_err(|e| error::ErrorBadRequest(e.to_string()))?;
    let target_path = resolve_path(&base_path, &target_relative)
        .map_err(|e| error::ErrorBadRequest(e.to_string()))?;

    if !target_path.starts_with(&current_directory)
        && target_path.parent() != Some(&current_directory)
    {
        return Err(error::ErrorBadRequest("Target outside directory"));
    }

    if target_path.is_dir() {
        fs::remove_dir_all(&target_path).map_err(error::ErrorInternalServerError)?;
    } else {
        fs::remove_file(&target_path).map_err(error::ErrorInternalServerError)?;
    }

    Ok(HttpResponse::Found()
        .append_header((header::LOCATION, format!("/browse/{}/{}", mount_name, tail)))
        .finish())
}

async fn rename_entry(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
    form: web::Form<RenameForm>,
) -> ActixResult<HttpResponse> {
    let (mount_name, tail) = path.into_inner();
    let config = &state.config;
    let mount = config
        .mounts
        .get(&mount_name)
        .ok_or_else(|| error::ErrorNotFound("Mount not found"))?;

    let username = get_username_from_cookie(&req);
    let permission = effective_permission(config, username.as_deref(), mount)
        .ok_or_else(|| error::ErrorForbidden("Write permission required"))?;
    if !permission.allows_rename() {
        return Err(error::ErrorForbidden("Write permission required"));
    }

    let current_relative =
        normalize_relative_path(&tail).ok_or_else(|| error::ErrorBadRequest("Invalid path"))?;
    let target_relative = normalize_relative_path(&form.target_path)
        .ok_or_else(|| error::ErrorBadRequest("Invalid target path"))?;

    let new_name = sanitize_file_name(&form.new_name)
        .ok_or_else(|| error::ErrorBadRequest("Invalid new name"))?;

    let base_path = canonicalize_mount(&mount.path).map_err(error::ErrorInternalServerError)?;
    let current_directory = resolve_path(&base_path, &current_relative)
        .map_err(|e| error::ErrorBadRequest(e.to_string()))?;
    let source_path = resolve_path(&base_path, &target_relative)
        .map_err(|e| error::ErrorBadRequest(e.to_string()))?;

    if source_path.parent() != Some(&current_directory) {
        return Err(error::ErrorBadRequest("Target outside directory"));
    }

    let destination = current_directory.join(new_name);
    fs::rename(&source_path, &destination).map_err(error::ErrorInternalServerError)?;

    Ok(HttpResponse::Found()
        .append_header((header::LOCATION, format!("/browse/{}/{}", mount_name, tail)))
        .finish())
}

async fn edit_page(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
) -> ActixResult<HttpResponse> {
    let (mount_name, tail) = path.into_inner();
    let config = &state.config;
    let mount = config
        .mounts
        .get(&mount_name)
        .ok_or_else(|| error::ErrorNotFound("Mount not found"))?;

    let username = get_username_from_cookie(&req);
    let permission = effective_permission(config, username.as_deref(), mount)
        .ok_or_else(|| error::ErrorForbidden("Permission required"))?;
    if !permission.allows_modify() {
        return Err(error::ErrorForbidden("Modify permission required"));
    }

    let relative_path =
        normalize_relative_path(&tail).ok_or_else(|| error::ErrorBadRequest("Invalid path"))?;

    let base_path = canonicalize_mount(&mount.path).map_err(error::ErrorInternalServerError)?;
    let target_path = resolve_path(&base_path, &relative_path)
        .map_err(|e| error::ErrorBadRequest(e.to_string()))?;

    if !target_path.is_file() {
        return Err(error::ErrorBadRequest("Target is not a file"));
    }

    let content = fs::read_to_string(&target_path).map_err(error::ErrorInternalServerError)?;

    let parent_path = relative_path
        .parent()
        .map(|p| {
            if p.as_os_str().is_empty() {
                ".".to_string()
            } else {
                pathbuf_to_string(p)
            }
        })
        .unwrap_or_else(|| ".".to_string());

    let filename = target_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_string();

    let mut context = TeraContext::new();
    context.insert("mount_name", &mount_name);
    context.insert("target_path", &pathbuf_to_string(&relative_path));
    context.insert("parent_path", &parent_path);
    context.insert("filename", &filename);
    context.insert("content", &content);

    let html = state
        .tera
        .render("edit.html", &context)
        .map_err(error::ErrorInternalServerError)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

async fn edit_save(
    state: web::Data<AppState>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
    form: web::Form<EditForm>,
) -> ActixResult<HttpResponse> {
    let (mount_name, tail) = path.into_inner();
    let config = &state.config;
    let mount = config
        .mounts
        .get(&mount_name)
        .ok_or_else(|| error::ErrorNotFound("Mount not found"))?;

    let username = get_username_from_cookie(&req);
    let permission = effective_permission(config, username.as_deref(), mount)
        .ok_or_else(|| error::ErrorForbidden("Write permission required"))?;
    if !permission.allows_modify() {
        return Err(error::ErrorForbidden("Modify permission required"));
    }

    let relative_path =
        normalize_relative_path(&tail).ok_or_else(|| error::ErrorBadRequest("Invalid path"))?;

    let base_path = canonicalize_mount(&mount.path).map_err(error::ErrorInternalServerError)?;
    let target_path = resolve_path(&base_path, &relative_path)
        .map_err(|e| error::ErrorBadRequest(e.to_string()))?;

    if !target_path.is_file() {
        return Err(error::ErrorBadRequest("Target is not a file"));
    }

    fs::write(&target_path, form.content.as_bytes()).map_err(error::ErrorInternalServerError)?;

    let parent = relative_path
        .parent()
        .map(|p| {
            if p.as_os_str().is_empty() {
                ".".to_string()
            } else {
                pathbuf_to_string(p)
            }
        })
        .unwrap_or_else(|| ".".to_string());

    Ok(HttpResponse::Found()
        .append_header((
            header::LOCATION,
            format!("/browse/{}/{}", mount_name, parent),
        ))
        .finish())
}

fn ensure_mount_directories(config: &Config) -> anyhow::Result<()> {
    for (name, mount) in &config.mounts {
        let mount_path = mount.path.as_path();
        if !mount_path.exists() {
            fs::create_dir_all(mount_path).with_context(|| {
                format!(
                    "Failed to create directory for mount '{}': {:?}",
                    name, mount_path
                )
            })?;
        }
    }
    Ok(())
}

fn get_username_from_cookie(req: &HttpRequest) -> Option<String> {
    req.cookie(SESSION_COOKIE)
        .map(|cookie| cookie.value().to_string())
}

fn render_markdown(content: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);

    let parser = Parser::new_ext(content, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

fn normalize_relative_path(path: &str) -> Option<PathBuf> {
    if path == "." || path.is_empty() {
        return Some(PathBuf::new());
    }

    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }

    Some(normalized)
}

fn sanitize_file_name(filename: &str) -> Option<String> {
    let candidate = Path::new(filename)
        .file_name()
        .and_then(|f| f.to_str())?
        .trim();

    if candidate.is_empty() || candidate.contains('/') || candidate.contains('\\') {
        return None;
    }
    Some(candidate.to_string())
}

fn canonicalize_mount(path: &Path) -> anyhow::Result<PathBuf> {
    if !path.exists() {
        fs::create_dir_all(path)
            .with_context(|| format!("Failed to create mount directory: {:?}", path))?;
    }
    fs::canonicalize(path).with_context(|| format!("Failed to canonicalize path: {:?}", path))
}

fn resolve_path(base: &Path, relative: &Path) -> anyhow::Result<PathBuf> {
    let target = if relative.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(relative)
    };

    if !target.starts_with(base) {
        return Err(anyhow!("Access outside of mount detected"));
    }

    Ok(target)
}

fn collect_entries(path: &Path) -> anyhow::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    if path.is_dir() {
        for entry in
            fs::read_dir(path).with_context(|| format!("Failed to read directory: {:?}", path))?
        {
            let entry = entry?;
            let metadata = entry.metadata()?;
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = metadata.is_dir();
            let size = if is_dir { None } else { Some(metadata.len()) };

            entries.push(FileEntry { name, is_dir, size });
        }

        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
    }

    Ok(entries)
}

fn pathbuf_to_string(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        ".".to_string()
    } else {
        path.components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/")
    }
}

fn build_directory_tree(
    base: &Path,
    relative: &Path,
    depth: usize,
) -> anyhow::Result<DirectoryNode> {
    if depth > TREE_MAX_DEPTH {
        return Err(anyhow!("Directory tree depth exceeded"));
    }

    let current_path = if relative.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(relative)
    };

    let name = if relative.as_os_str().is_empty() {
        ".".to_string()
    } else {
        relative
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("")
            .to_string()
    };

    let path_string = pathbuf_to_string(relative);
    let mut node = DirectoryNode {
        name,
        path: path_string,
        children: Vec::new(),
    };

    let mut directories = Vec::new();
    for entry in fs::read_dir(&current_path)
        .with_context(|| format!("Failed to read directory: {:?}", current_path))?
    {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            directories.push(entry.file_name());
        }
    }

    directories.sort_by(|a, b| {
        a.to_string_lossy()
            .to_lowercase()
            .cmp(&b.to_string_lossy().to_lowercase())
    });

    for dir_name in directories {
        let child_relative = relative.join(&dir_name);
        node.children
            .push(build_directory_tree(base, &child_relative, depth + 1)?);
    }

    Ok(node)
}

fn build_open_paths(current_path: &str) -> Vec<String> {
    let mut paths = Vec::new();
    paths.push(".".to_string());

    if current_path == "." {
        return paths;
    }

    let mut accumulated = PathBuf::new();
    for part in current_path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        accumulated.push(part);
        paths.push(pathbuf_to_string(&accumulated));
    }

    paths
}

fn effective_permission(
    config: &Config,
    username: Option<&str>,
    mount: &MountConfig,
) -> Option<Permission> {
    let mut aggregated = if mount.public {
        Some(Permission::from_actions(["read"]))
    } else {
        None
    };

    if let Some(username) = username {
        if let Some(spec) = mount.user.get(username) {
            let resolved = config.resolve_permission_spec(spec);
            aggregated = merge_permission(aggregated, resolved);
        }

        if let Some(user_config) = config.users.get(username) {
            for group in &user_config.group {
                if let Some(spec) = mount.group.get(group) {
                    let resolved = config.resolve_permission_spec(spec);
                    aggregated = merge_permission(aggregated, resolved);
                }
            }
        }
    }

    match aggregated {
        Some(ref permission) if permission.is_empty() => None,
        other => other,
    }
}

fn merge_permission(current: Option<Permission>, addition: Permission) -> Option<Permission> {
    if addition.is_empty() {
        return current;
    }

    match current {
        Some(mut existing) => {
            existing.merge(&addition);
            Some(existing)
        }
        None => Some(addition),
    }
}
