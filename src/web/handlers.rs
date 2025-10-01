use axum::{
    body::Body,
    extract::{Path, State, Json as AxumJson},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response, Json},
};
use std::path::PathBuf;
use std::io::{Write, Cursor};
use tokio::fs;
use mime_guess::from_path;
use serde::Deserialize;
use serde_json::json;
use zip::write::FileOptions;
use zip::ZipWriter;
use crate::web::AppState;

#[derive(Deserialize)]
pub struct ZipRequest {
    pub paths: Vec<String>,
}

// 루트 페이지 - 사용 가능한 마운트 포인트 목록과 전체 파일 트리 표시
pub async fn index(State(state): State<AppState>) -> impl IntoResponse {
    let mut mount_list: Vec<String> = state.config.mounts.keys().cloned().collect();
    mount_list.sort();
    
    // 전체 마운트 포인트의 파일 트리 생성 (사이드바용 - 루트부터 시작)
    let mut all_mount_trees = Vec::new();
    
    for mount in &mount_list {
        let mount_config = state.config.get_mount(mount).unwrap();
        // 루트 마운트를 tree-folder로 감싸서 생성
        let tree = generate_file_tree_for_mount(&mount_config.path, mount);
        all_mount_trees.push(tree);
    }
    
    let file_tree = all_mount_trees.join("\n");
    
    // 메인 콘텐츠에 표시할 마운트 포인트 목록
    let mut mount_items = Vec::new();
    for mount in &mount_list {
        let mount_config = state.config.get_mount(mount).unwrap();
        let default_desc = "No description".to_string();
        let description = mount_config
            .description
            .as_ref()
            .unwrap_or(&default_desc);
        
        let (dir_count, file_count) = count_items_sync(&mount_config.path);
        
        mount_items.push(format!(
            r#"<li class="file-item">
                <a href="/{}" class="directory">{}</a>
                <div style="display: flex; flex-direction: column; gap: 3px;">
                    <span style="color: #666; font-size: 0.9em;">{}</span>
                    <span class="file-size">{} directories, {} files</span>
                </div>
            </li>"#,
            mount, mount, description, dir_count, file_count
        ));
    }
    
    // 마크다운 파일 내용 읽기 (설정에 지정된 경우)
    let markdown_content = if let Some(md_path) = &state.config.main_page.markdown_file {
        std::fs::read_to_string(md_path).ok().map(|content| {render_simple_markdown_for_preview(&content)})
    } else {
        None
    };
    
    let page_title = &state.config.main_page.title;
    let page_description = &state.config.main_page.description;

    let html = format!(
        r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>{}</title>
            <style>
                body {{ 
                    font-family: Arial, sans-serif; 
                    margin: 0;
                    display: flex;
                    min-height: 100vh;
                }}
                .sidebar {{
                    width: 300px;
                    background: #f8f9fa;
                    border-right: 1px solid #dee2e6;
                    padding: 20px;
                    overflow-y: auto;
                    position: fixed;
                    height: 100vh;
                }}
                .main-content {{
                    margin-left: 300px;
                    padding: 40px;
                    flex: 1;
                }}
                .nav {{ margin-bottom: 20px; }}
                .nav a {{ text-decoration: none; color: #007acc; }}
                .nav a:hover {{ text-decoration: underline; }}
                .file-list {{ list-style: none; padding: 0; }}
                .file-item {{ 
                    background: #f9f9f9; 
                    margin: 5px 0; 
                    padding: 10px; 
                    border-radius: 3px;
                    display: flex;
                    justify-content: space-between;
                    align-items: center;
                }}
                .file-item:hover {{ background: #f0f0f0; }}
                .file-item a {{ text-decoration: none; color: #333; }}
                .file-item a:hover {{ color: #007acc; }}
                .directory {{ font-weight: bold; }}
                .directory:before {{ content: "📁 "; }}
                .file:before {{ content: "� "; }}
                .file-size {{ color: #666; font-size: 0.9em; }}
                .readme-preview {{
                    margin-top: 30px;
                    padding: 20px;
                    background: #f8f9fa;
                    border: 1px solid #e9ecef;
                    border-radius: 8px;
                    border-left: 4px solid #007acc;
                }}
                .readme-preview h3, .readme-preview h4 {{
                    margin: 10px 0;
                    color: #333;
                }}
                .readme-preview p {{
                    margin: 8px 0;
                    line-height: 1.5;
                }}
                .readme-preview li {{
                    margin: 5px 0;
                    list-style: none;
                    padding-left: 15px;
                }}
                .readme-preview li:before {{
                    content: '• ';
                    color: #007acc;
                    font-weight: bold;
                }}
                .tree-folder, .tree-file {{
                    margin: 2px 0;
                    font-size: 0.9em;
                }}
                .tree-folder a, .tree-file a {{
                    text-decoration: none;
                    color: #333;
                }}
                .tree-folder a:hover, .tree-file a:hover {{
                    color: #007acc;
                }}
                .folder-toggle {{
                    cursor: pointer;
                    user-select: none;
                    display: inline-block;
                    transition: transform 0.2s ease;
                    margin-right: 5px;
                }}
                .folder-toggle.expanded {{
                    transform: rotate(90deg);
                }}
                .tree-children {{
                    margin-left: 16px;
                    border-left: 1px dotted #ccc;
                    padding-left: 8px;
                    transition: all 0.3s ease;
                }}
                .tree-children.collapsed {{
                    display: none;
                }}
                .file-icon {{
                    margin-right: 5px;
                }}
                .sidebar h3 {{
                    margin-top: 0;
                    color: #333;
                    border-bottom: 2px solid #007acc;
                    padding-bottom: 10px;
                }}
                .tree-folder.current-path {{
                    background: rgba(0, 122, 204, 0.1);
                    border-radius: 3px;
                    padding: 2px 4px;
                }}
                .loading {{
                    color: #666;
                    font-style: italic;
                    font-size: 0.8em;
                }}
            </style>
        </head>
        <body>
            <div class="sidebar">
                <h3>� File Tree</h3>
                <div class="file-tree">
                    {}
                </div>
            </div>
            <div class="main-content">
                <h1>📁 {}</h1>
                <p style="color: #666; margin-bottom: 30px;">{}</p>
                {}
                <h2>Mount Points</h2>
                <ul class="file-list">
                    {}
                </ul>
            </div>
            <script>
                // 폴더 토글 및 동적 로딩 기능
                document.addEventListener('DOMContentLoaded', function() {{
                    attachToggleListeners();
                    
                    function attachToggleListeners() {{
                        document.querySelectorAll('.folder-toggle').forEach(function(toggle) {{
                            if (toggle.hasAttribute('data-listener')) return;
                            toggle.setAttribute('data-listener', 'true');
                            
                            toggle.addEventListener('click', function(e) {{
                                e.preventDefault();
                                e.stopPropagation();
                                
                                const folder = this.closest('.tree-folder');
                                const children = folder.querySelector('.tree-children');
                                const path = folder.getAttribute('data-path');
                                const mount = folder.getAttribute('data-mount');
                                
                                if (children.classList.contains('collapsed')) {{
                                    children.classList.remove('collapsed');
                                    this.textContent = '📂';
                                    this.classList.add('expanded');
                                    
                                    if (!children.hasAttribute('data-loaded') && children.innerHTML.trim() === '') {{
                                        loadSubfolders(mount, path, children);
                                        children.setAttribute('data-loaded', 'true');
                                    }}
                                }} else {{
                                    children.classList.add('collapsed');
                                    this.textContent = '📁';
                                    this.classList.remove('expanded');
                                }}
                            }});
                        }});
                    }}
                    
                    async function loadSubfolders(mount, path, container) {{
                        try {{
                            container.innerHTML = '<div class="loading">로딩 중...</div>';
                            const response = await fetch(`/api/${{mount}}/tree/${{path}}`);
                            const data = await response.json();
                            
                            if (data.success && data.items && data.items.length > 0) {{
                                container.innerHTML = data.items.map(item => {{
                                    if (item.is_dir) {{
                                        return `<div class="tree-folder" data-path="${{item.path}}" data-mount="${{mount}}">
                                                  <span class="folder-toggle">📁</span> 
                                                  <a href="/${{mount}}/${{item.path}}">${{item.name}}</a>
                                                  <div class="tree-children collapsed"></div>
                                                </div>`;
                                    }} else {{
                                        return `<div class="tree-file">
                                                  <span class="file-icon">📄</span> 
                                                  <a href="/${{mount}}/${{item.path}}">${{item.name}}</a>
                                                </div>`;
                                    }}
                                }}).join('');
                                
                                attachToggleListeners();
                            }} else {{
                                container.innerHTML = '<div style="color: #666; font-size: 0.8em;">하위 폴더 없음</div>';
                            }}
                        }} catch (error) {{
                            console.error('폴더 로딩 실패:', error);
                            container.innerHTML = '<div style="color: #cc0000; font-size: 0.8em;">로딩 실패</div>';
                        }}
                    }}
                    
                    // 사이드바 토글 기능
                    window.toggleSidebar = function() {{
                        const sidebar = document.querySelector('.sidebar');
                        const mainContent = document.querySelector('.main-content');
                        const toggleBtn = document.querySelector('.sidebar-toggle');
                        
                        sidebar.classList.toggle('collapsed');
                        mainContent.classList.toggle('expanded');
                        toggleBtn.classList.toggle('collapsed');
                        
                        if (sidebar.classList.contains('collapsed')) {{
                            toggleBtn.textContent = '☰';
                        }} else {{
                            toggleBtn.textContent = '✕';
                        }}
                    }};
                }});
            </script>
        </body>
        </html>
        "#,
        page_title, // HTML title
        file_tree, // sidebar
        page_title, // h1
        page_description, // description
        markdown_content.unwrap_or_default(), // optional markdown content
        mount_items.join("\n") // mount points list
    );

    Html(html)
}

// 디렉토리와 파일 개수를 세는 헬퍼 함수
fn count_items_sync(base_path: &PathBuf) -> (usize, usize) {
    let mut dir_count = 0;
    let mut file_count = 0;
    
    if let Ok(entries) = std::fs::read_dir(base_path) {
        for entry in entries {
            if let Ok(entry) = entry {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_dir() {
                        dir_count += 1;
                        let subpath = entry.path();
                        let (sub_dirs, sub_files) = count_items_sync(&subpath);
                        dir_count += sub_dirs;
                        file_count += sub_files;
                    } else {
                        file_count += 1;
                    }
                }
            }
        }
    }
    
    (dir_count, file_count)
}

// 마운트 포인트 또는 파일/폴더 핸들러
pub async fn handle_mount_path(
    Path((mount, path)): Path<(String, String)>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // 마운트 포인트가 존재하는지 확인
    let mount_config = match state.config.get_mount(&mount) {
        Some(config) => config,
        None => {
            return (StatusCode::NOT_FOUND, "Mount point not found").into_response();
        }
    };

    // 실제 파일 시스템 경로 구성
    let mut full_path = mount_config.path.clone();
    if !path.is_empty() {
        full_path.push(&path);
    }

    // 경로 보안 검사 (path traversal 공격 방지)
    if !is_safe_path(&mount_config.path, &full_path) {
        return (StatusCode::FORBIDDEN, "Access denied").into_response();
    }

    // 파일/디렉토리 존재 확인
    let metadata = match fs::metadata(&full_path).await {
        Ok(metadata) => metadata,
        Err(_) => {
            return (StatusCode::NOT_FOUND, "File or directory not found").into_response();
        }
    };

    if metadata.is_dir() {
        // 디렉토리인 경우 - 파일 목록 표시
        serve_directory(&mount, &path, &full_path, &mount_config.path, &state).await
    } else {
        // 파일인 경우 - 파일 다운로드
        serve_file(&full_path).await
    }
}

// 마운트 루트 핸들러
pub async fn handle_mount_root(
    Path(mount): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let mount_config = match state.config.get_mount(&mount) {
        Some(config) => config,
        None => {
            return (StatusCode::NOT_FOUND, "Mount point not found").into_response();
        }
    };

    let full_path = &mount_config.path;

    let metadata = match fs::metadata(full_path).await {
        Ok(metadata) => metadata,
        Err(_) => {
            return (StatusCode::NOT_FOUND, "Mount point path not found").into_response();
        }
    };

    if metadata.is_dir() {
        serve_directory(&mount, "", full_path, &mount_config.path, &state).await
    } else {
        serve_file(full_path).await
    }
}

async fn serve_directory(mount: &str, current_path: &str, full_path: &PathBuf, _mount_base_path: &PathBuf, state: &AppState) -> Response {
    let mut entries = match fs::read_dir(full_path).await {
        Ok(entries) => entries,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read directory").into_response();
        }
    };

    let mut files = Vec::new();
    let mut dirs = Vec::new();

    while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
        if let Ok(metadata) = entry.metadata().await {
            let name = entry.file_name().to_string_lossy().to_string();
            let item_path = if current_path.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", current_path, name)
            };

            if metadata.is_dir() {
                dirs.push((name, item_path));
            } else {
                let size = metadata.len();
                files.push((name, item_path, size));
            }
        }
    }

    // 알파벳 순으로 정렬
    dirs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    // README 파일 내용 읽기 (첫 번째 README 파일만)
    let readme_content = files.iter()
        .find(|(name, _, _)| is_readme_file(name))
        .and_then(|(readme_name, _, _)| {
            let readme_path = full_path.join(readme_name);
            std::fs::read_to_string(&readme_path).ok().map(|content| {
                let is_markdown = readme_name.to_lowercase().ends_with(".md");
                if is_markdown {
                    render_simple_markdown_for_preview(&content)
                } else {
                    format!("<pre>{}</pre>", content.replace('<', "&lt;").replace('>', "&gt;"))
                }
            })
        });

    // 사이드바용 파일 트리 생성 (모든 마운트 포인트의 전역 트리)
    let mut mount_list: Vec<String> = state.config.mounts.keys().cloned().collect();
    mount_list.sort();
    
    let mut all_mount_trees = Vec::new();
    for mount_name in &mount_list {
        let mount_config = state.config.get_mount(mount_name).unwrap();
        let tree = generate_file_tree_for_mount(&mount_config.path, mount_name);
        all_mount_trees.push(tree);
    }
    
    let file_tree = all_mount_trees.join("\n");

    let parent_path = if current_path.is_empty() {
        None
    } else {
        let parent = std::path::Path::new(current_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string());
        if let Some(p) = parent {
            if p.is_empty() {
                Some(format!("/{}", mount))
            } else {
                Some(format!("/{}/{}", mount, p))
            }
        } else {
            Some(format!("/{}", mount))
        }
    };

    let current_url_path = if current_path.is_empty() {
        format!("/{}", mount)
    } else {
        format!("/{}/{}", mount, current_path)
    };

    let html = format!(
        r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>LunaFinder - {}</title>
            <style>
                body {{ 
                    font-family: Arial, sans-serif; 
                    margin: 0;
                    display: flex;
                    min-height: 100vh;
                }}
                .sidebar {{
                    width: 300px;
                    background: #f8f9fa;
                    border-right: 1px solid #dee2e6;
                    padding: 20px;
                    overflow-y: auto;
                    position: fixed;
                    height: 100vh;
                }}
                .main-content {{
                    margin-left: 300px;
                    padding: 40px;
                    flex: 1;
                }}
                .nav {{ margin-bottom: 20px; }}
                .nav a {{ text-decoration: none; color: #007acc; }}
                .nav a:hover {{ text-decoration: underline; }}
                .file-list {{ list-style: none; padding: 0; }}
                .file-item {{ 
                    background: #f9f9f9; 
                    margin: 5px 0; 
                    padding: 10px; 
                    border-radius: 3px;
                    display: flex;
                    justify-content: space-between;
                    align-items: center;
                }}
                .file-item:hover {{ background: #f0f0f0; }}
                .file-item a {{ text-decoration: none; color: #333; }}
                .file-item a:hover {{ color: #007acc; }}
                .directory {{ font-weight: bold; }}
                .directory:before {{ content: "📁 "; }}
                .file:before {{ content: "📄 "; }}
                .file-size {{ color: #666; font-size: 0.9em; }}
                .readme-preview {{
                    margin-top: 30px;
                    padding: 20px;
                    background: #f8f9fa;
                    border: 1px solid #e9ecef;
                    border-radius: 8px;
                    border-left: 4px solid #007acc;
                }}
                .readme-preview h3, .readme-preview h4 {{
                    margin: 10px 0;
                    color: #333;
                }}
                .readme-preview p {{
                    margin: 8px 0;
                    line-height: 1.5;
                }}
                .readme-preview li {{
                    margin: 5px 0;
                    list-style: none;
                    padding-left: 15px;
                }}
                .readme-preview li:before {{
                    content: '• ';
                    color: #007acc;
                    font-weight: bold;
                }}
                .tree-folder, .tree-file {{
                    margin: 2px 0;
                    font-size: 0.9em;
                }}
                .tree-folder a, .tree-file a {{
                    text-decoration: none;
                    color: #333;
                }}
                .tree-folder a:hover, .tree-file a:hover {{
                    color: #007acc;
                }}
                .folder-toggle {{
                    cursor: pointer;
                    user-select: none;
                    display: inline-block;
                    transition: transform 0.2s ease;
                }}
                .folder-toggle.expanded {{
                    transform: rotate(90deg);
                }}
                .tree-children {{
                    margin-left: 16px;
                    border-left: 1px dotted #ccc;
                    padding-left: 8px;
                    transition: all 0.3s ease;
                }}
                .tree-children.collapsed {{
                    display: none;
                }}
                .file-icon, .folder-toggle {{
                    margin-right: 5px;
                }}
                .sidebar h3 {{
                    margin-top: 0;
                    color: #333;
                    border-bottom: 2px solid #007acc;
                    padding-bottom: 10px;
                }}
                .tree-folder.current-path {{
                    background: rgba(0, 122, 204, 0.1);
                    border-radius: 3px;
                    padding: 2px 4px;
                }}
                .loading {{
                    color: #666;
                    font-style: italic;
                    font-size: 0.8em;
                }}
            </style>
        </head>
        <body>
            <button class="sidebar-toggle" onclick="toggleSidebar()">☰</button>
            <div class="sidebar">
                <h3>📁 File Tree</h3>
                <div class="file-tree">
                    {}
                </div>
            </div>
            <div class="main-content">
                <div class="nav">
                    <a href="/">← Home</a>
                    {}
                    <span> / Current: {}</span>
                </div>
                <h1>📁 {}</h1>
                <ul class="file-list">
                    {}
                    {}
                    {}
                </ul>
                {}
            </div>
            <div class="action-bar">
                <span class="selection-info">0 items selected</span>
                <button onclick="clearSelection()">Clear Selection</button>
                <button class="primary" onclick="downloadSelected()">Download as ZIP</button>
            </div>
            <script>
                // 파일 선택 기능
                let selectedFiles = new Set();
                
                function updateSelectionUI() {{
                    const actionBar = document.querySelector('.action-bar');
                    const selectionInfo = document.querySelector('.selection-info');
                    const count = selectedFiles.size;
                    
                    if (count > 0) {{
                        actionBar.classList.add('active');
                        selectionInfo.textContent = `${{count}} item${{count > 1 ? 's' : ''}} selected`;
                    }} else {{
                        actionBar.classList.remove('active');
                    }}
                }}
                
                function clearSelection() {{
                    selectedFiles.clear();
                    document.querySelectorAll('.file-checkbox').forEach(cb => cb.checked = false);
                    document.querySelectorAll('.file-item').forEach(item => item.classList.remove('selected'));
                    updateSelectionUI();
                }}
                
                function downloadSelected() {{
                    if (selectedFiles.size === 0) return;
                    
                    const paths = Array.from(selectedFiles);
                    const mount = window.location.pathname.split('/')[1];
                    
                    // ZIP 다운로드 요청
                    const form = document.createElement('form');
                    form.method = 'POST';
                    form.action = `/api/${{mount}}/zip`;
                    
                    const input = document.createElement('input');
                    input.type = 'hidden';
                    input.name = 'paths';
                    input.value = JSON.stringify(paths);
                    
                    form.appendChild(input);
                    document.body.appendChild(form);
                    form.submit();
                    document.body.removeChild(form);
                }}
                
                // 파일 아이템 클릭 핸들러
                document.querySelectorAll('.file-item').forEach(item => {{
                    const checkbox = item.querySelector('.file-checkbox');
                    const path = item.getAttribute('data-path');
                    
                    // 체크박스 변경 핸들러
                    checkbox.addEventListener('change', function(e) {{
                        e.stopPropagation();
                        if (this.checked) {{
                            selectedFiles.add(path);
                            item.classList.add('selected');
                        }} else {{
                            selectedFiles.delete(path);
                            item.classList.remove('selected');
                        }}
                        updateSelectionUI();
                    }});
                    
                    // 파일 아이템 클릭 (체크박스 제외)
                    item.addEventListener('click', function(e) {{
                        if (e.target.tagName === 'A' || e.target.tagName === 'INPUT') return;
                        checkbox.checked = !checkbox.checked;
                        checkbox.dispatchEvent(new Event('change'));
                    }});
                    
                    // Ctrl/Cmd + 클릭: 토글 선택
                    item.addEventListener('click', function(e) {{
                        if ((e.ctrlKey || e.metaKey) && e.target.tagName === 'A') {{
                            e.preventDefault();
                            checkbox.checked = !checkbox.checked;
                            checkbox.dispatchEvent(new Event('change'));
                        }}
                    }});
                }});
                
                // 폴더 토글 및 동적 로딩 기능
                document.addEventListener('DOMContentLoaded', function() {{
                    // 현재 경로에 해당하는 폴더들만 확장
                    expandCurrentPath();
                    
                    document.querySelectorAll('.folder-toggle').forEach(function(toggle) {{
                        toggle.addEventListener('click', function(e) {{
                            e.preventDefault();
                            e.stopPropagation();
                            
                            const folder = this.closest('.tree-folder');
                            const children = folder.querySelector('.tree-children');
                            const path = folder.getAttribute('data-path');
                            
                            if (children.classList.contains('collapsed')) {{
                                // 폴더 열기
                                children.classList.remove('collapsed');
                                this.textContent = '📂';
                                this.classList.add('expanded');
                                
                                // 하위 폴더가 비어있다면 동적으로 로드
                                if (!children.hasAttribute('data-loaded')) {{
                                    loadSubfolders(path, children);
                                    children.setAttribute('data-loaded', 'true');
                                }}
                            }} else {{
                                // 폴더 닫기
                                children.classList.add('collapsed');
                                this.textContent = '📁';
                                this.classList.remove('expanded');
                            }}
                        }});
                    }});
                    
                    function expandCurrentPath() {{
                        const currentPath = '{}';
                        if (!currentPath) return;
                        
                        const pathParts = currentPath.split('/').filter(p => p);
                        let currentPartialPath = '';
                        
                        for (let i = 0; i < pathParts.length; i++) {{
                            if (i > 0) currentPartialPath += '/';
                            currentPartialPath += pathParts[i];
                            
                            const folder = document.querySelector(`[data-path="${{currentPartialPath}}"]`);
                            if (folder) {{
                                const toggle = folder.querySelector('.folder-toggle');
                                const children = folder.querySelector('.tree-children');
                                if (toggle && children) {{
                                    children.classList.remove('collapsed');
                                    toggle.textContent = '📂';
                                    toggle.classList.add('expanded');
                                    children.setAttribute('data-loaded', 'true');
                                }}
                                folder.classList.add('current-path');
                            }}
                        }}
                    }}
                    
                    async function loadSubfolders(path, container) {{
                        try {{
                            container.innerHTML = '<div class="loading">로딩 중...</div>';
                            const response = await fetch(`/api/{mount}/tree/${{path}}`);
                            const data = await response.json();
                            
                            if (data.success && data.items && data.items.length > 0) {{
                                container.innerHTML = data.items.map(item => {{
                                    if (item.is_dir) {{
                                        return `<div class="tree-folder" data-path="${{item.path}}">
                                                  <span class="folder-toggle">📁</span> 
                                                  <a href="/{mount}/${{item.path}}">${{item.name}}</a>
                                                  <div class="tree-children collapsed"></div>
                                                </div>`;
                                    }} else {{
                                        return `<div class="tree-file">
                                                  <span class="file-icon">📄</span> 
                                                  <a href="/{mount}/${{item.path}}">${{item.name}}</a>
                                                </div>`;
                                    }}
                                }}).join('');
                                
                                // 새로 추가된 토글들에 이벤트 리스너 추가
                                container.querySelectorAll('.folder-toggle').forEach(function(newToggle) {{
                                    newToggle.addEventListener('click', function(e) {{
                                        e.preventDefault();
                                        e.stopPropagation();
                                        
                                        const newFolder = this.closest('.tree-folder');
                                        const newChildren = newFolder.querySelector('.tree-children');
                                        const newPath = newFolder.getAttribute('data-path');
                                        
                                        if (newChildren.classList.contains('collapsed')) {{
                                            newChildren.classList.remove('collapsed');
                                            this.textContent = '�';
                                            this.classList.add('expanded');
                                            
                                            if (!newChildren.hasAttribute('data-loaded')) {{
                                                loadSubfolders(newPath, newChildren);
                                                newChildren.setAttribute('data-loaded', 'true');
                                            }}
                                        }} else {{
                                            newChildren.classList.add('collapsed');
                                            this.textContent = '�📁';
                                            this.classList.remove('expanded');
                                        }}
                                    }});
                                }});
                            }} else {{
                                container.innerHTML = '<div style="color: #666; font-size: 0.8em;">하위 폴더 없음</div>';
                            }}
                        }} catch (error) {{
                            console.error('폴더 로딩 실패:', error);
                            container.innerHTML = '<div style="color: #cc0000; font-size: 0.8em;">로딩 실패</div>';
                        }}
                    }}
                    
                    // 사이드바 토글 기능
                    window.toggleSidebar = function() {{
                        const sidebar = document.querySelector('.sidebar');
                        const mainContent = document.querySelector('.main-content');
                        const toggleBtn = document.querySelector('.sidebar-toggle');
                        
                        sidebar.classList.toggle('collapsed');
                        mainContent.classList.toggle('expanded');
                        toggleBtn.classList.toggle('collapsed');
                        
                        if (sidebar.classList.contains('collapsed')) {{
                            toggleBtn.textContent = '☰';
                        }} else {{
                            toggleBtn.textContent = '✕';
                        }}
                    }};
                }});
            </script>
        </body>
        </html>
        "#,
        current_url_path, // title
        file_tree, // sidebar file tree
        if let Some(parent) = parent_path { // nav parent
            format!(r#" | <a href="{}">← Parent Directory</a>"#, parent)
        } else {
            String::new()
        },
        current_url_path, // nav current
        current_url_path, // h1 title
        dirs.iter()
            .map(|(name, path)| {
                format!(
                    r#"<li class="file-item" data-path="{}" data-type="dir">
                        <input type="checkbox" class="file-checkbox" onclick="event.stopPropagation()">
                        <a href="/{}/{}" class="directory">{}</a>
                    </li>"#,
                    path, mount, path, name
                )
            })
            .collect::<Vec<_>>()
            .join(""),
        files
            .iter()
            .map(|(name, path, size)| {
                format!(
                    r#"<li class="file-item" data-path="{}" data-type="file">
                        <input type="checkbox" class="file-checkbox" onclick="event.stopPropagation()">
                        <a href="/{}/{}" class="file">{}</a>
                        <span class="file-size">{}</span>
                    </li>"#,
                    path,
                    mount,
                    path,
                    name,
                    format_file_size(*size)
                )
            })
            .collect::<Vec<_>>()
            .join(""),
        "", // 빈 문자열로 세 번째 파라미터 채우기
        readme_content.unwrap_or_default(), // README content preview
        current_path // JavaScript expandCurrentPath에서 사용
    );

    Html(html).into_response()
}

async fn serve_file(full_path: &PathBuf) -> Response {
    let file_content = match fs::read_to_string(full_path).await {
        Ok(content) => content,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response();
        }
    };

    let file_name = full_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");

    // README 파일이나 텍스트 파일인 경우 HTML로 렌더링
    if is_readme_file(file_name) || is_text_file(file_name) {
        render_text_file(file_name, &file_content)
    } else {
        // 바이너리 파일은 다운로드
        let file_bytes = match fs::read(full_path).await {
            Ok(bytes) => bytes,
            Err(_) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response();
            }
        };

        let mime_type = from_path(full_path).first_or_octet_stream();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            mime_type.as_ref().parse().unwrap(),
        );
        headers.insert(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", file_name)
                .parse()
                .unwrap(),
        );

        (headers, file_bytes).into_response()
    }
}

fn is_safe_path(base: &PathBuf, full: &PathBuf) -> bool {
    match full.canonicalize() {
        Ok(canonical_full) => {
            if let Ok(canonical_base) = base.canonicalize() {
                canonical_full.starts_with(canonical_base)
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

fn is_readme_file(filename: &str) -> bool {
    let lower_name = filename.to_lowercase();
    matches!(lower_name.as_str(), "readme.md" | "readme.txt" | "readme" | "readme.rst")
}

fn is_text_file(filename: &str) -> bool {
    let lower_name = filename.to_lowercase();
    lower_name.ends_with(".txt") || 
    lower_name.ends_with(".md") || 
    lower_name.ends_with(".rst") ||
    lower_name.ends_with(".log") ||
    lower_name.ends_with(".cfg") ||
    lower_name.ends_with(".conf") ||
    lower_name.ends_with(".toml") ||
    lower_name.ends_with(".json") ||
    lower_name.ends_with(".xml") ||
    lower_name.ends_with(".csv")
}

fn render_text_file(filename: &str, content: &str) -> Response {
    let is_markdown = filename.to_lowercase().ends_with(".md");
    let file_type = if is_markdown { "Markdown" } else { "Text" };
    
    // HTML 이스케이프
    let escaped_content = content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;");

    let html = format!(
        r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>LunaFinder - {}</title>
            <meta charset="utf-8">
            <style>
                body {{ 
                    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Arial, sans-serif;
                    margin: 40px;
                    line-height: 1.6;
                    background-color: #fff;
                }}
                .header {{
                    background: #f8f9fa;
                    padding: 15px;
                    border-radius: 5px;
                    margin-bottom: 20px;
                    border-left: 4px solid #007acc;
                }}
                .header h1 {{
                    margin: 0;
                    color: #333;
                    font-size: 1.5em;
                }}
                .file-type {{
                    color: #666;
                    font-size: 0.9em;
                    margin-top: 5px;
                }}
                .content {{
                    background: #f8f9fa;
                    padding: 20px;
                    border-radius: 5px;
                    border: 1px solid #e9ecef;
                    white-space: pre-wrap;
                    font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
                    font-size: 0.9em;
                    overflow-x: auto;
                }}
                .nav {{
                    margin-bottom: 20px;
                }}
                .nav a {{
                    text-decoration: none;
                    color: #007acc;
                    font-weight: bold;
                }}
                .nav a:hover {{
                    text-decoration: underline;
                }}
                {}
            </style>
        </head>
        <body>
            <div class="nav">
                <a href="javascript:history.back()">← Back</a> | 
                <a href="/">🏠 Home</a>
            </div>
            <div class="header">
                <h1>📄 {}</h1>
                <div class="file-type">{} File</div>
            </div>
            <div class="content">{}</div>
        </body>
        </html>
        "#,
        filename,
        if is_markdown {
            r#"
                .content {
                    background: white;
                    border: 1px solid #ddd;
                }
                h1, h2, h3, h4, h5, h6 { color: #333; margin-top: 1.5em; }
                h1 { border-bottom: 2px solid #eee; padding-bottom: 10px; }
                h2 { border-bottom: 1px solid #eee; padding-bottom: 5px; }
                code { background: #f1f3f4; padding: 2px 4px; border-radius: 3px; }
                pre { background: #f8f9fa; padding: 10px; border-radius: 5px; overflow-x: auto; }
                blockquote { border-left: 4px solid #ddd; margin-left: 0; padding-left: 16px; color: #666; }
                ul, ol { margin-left: 20px; }
                li { margin: 5px 0; }
                a { color: #007acc; }
            "#
        } else {
            ""
        },
        filename,
        file_type,
        if is_markdown {
            // 간단한 마크다운 렌더링
            render_simple_markdown(&escaped_content)
        } else {
            escaped_content
        }
    );

    Html(html).into_response()
}

fn render_simple_markdown(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut in_list = false;
    
    for line in lines {
        let trimmed = line.trim();
        
        // 제목 처리
        if trimmed.starts_with("### ") {
            if in_list { result.push("</ul>".to_string()); in_list = false; }
            result.push(format!("<h3>{}</h3>", &trimmed[4..]));
        } else if trimmed.starts_with("## ") {
            if in_list { result.push("</ul>".to_string()); in_list = false; }
            result.push(format!("<h2>{}</h2>", &trimmed[3..]));
        } else if trimmed.starts_with("# ") {
            if in_list { result.push("</ul>".to_string()); in_list = false; }
            result.push(format!("<h1>{}</h1>", &trimmed[2..]));
        }
        // 리스트 처리
        else if trimmed.starts_with("- ") {
            if !in_list {
                result.push("<ul>".to_string());
                in_list = true;
            }
            let item_text = &trimmed[2..];
            result.push(format!("<li>{}</li>", format_inline_markdown(item_text)));
        }
        // 빈 줄
        else if trimmed.is_empty() {
            if in_list { result.push("</ul>".to_string()); in_list = false; }
            result.push("<br>".to_string());
        }
        // 일반 텍스트
        else {
            if in_list { result.push("</ul>".to_string()); in_list = false; }
            result.push(format!("<p>{}</p>", format_inline_markdown(trimmed)));
        }
    }
    
    if in_list {
        result.push("</ul>".to_string());
    }
    
    result.join("\n")
}

fn format_inline_markdown(text: &str) -> String {
    let mut result = text.to_string();
    
    // 볼드 처리 (**text**)
    while let Some(start) = result.find("**") {
        if let Some(end) = result[start + 2..].find("**") {
            let end = end + start + 2;
            let bold_text = &result[start + 2..end];
            result = format!("{}<strong>{}</strong>{}",
                           &result[..start],
                           bold_text,
                           &result[end + 2..]);
        } else {
            break;
        }
    }
    
    // 이탤릭 처리 (*text*)
    while let Some(start) = result.find('*') {
        if let Some(end) = result[start + 1..].find('*') {
            let end = end + start + 1;
            let italic_text = &result[start + 1..end];
            result = format!("{}<em>{}</em>{}",
                           &result[..start],
                           italic_text,
                           &result[end + 1..]);
        } else {
            break;
        }
    }
    
    // 인라인 코드 처리 (`code`)
    while let Some(start) = result.find('`') {
        if let Some(end) = result[start + 1..].find('`') {
            let end = end + start + 1;
            let code_text = &result[start + 1..end];
            result = format!("{}<code>{}</code>{}",
                           &result[..start],
                           code_text,
                           &result[end + 1..]);
        } else {
            break;
        }
    }
    
    result
}

fn generate_file_tree_sync(base_path: &std::path::PathBuf, mount: &str, current_path: &str, depth: usize) -> String {
    if depth > 2 { // 초기 로딩 시 깊이를 줄임 (더 많은 동적 로딩을 위해)
        return String::new();
    }
    
    let mut result = Vec::new();
    let full_path = if current_path.is_empty() {
        base_path.clone()
    } else {
        base_path.join(current_path)
    };
    
    if let Ok(entries) = std::fs::read_dir(&full_path) {
        let mut items = Vec::new();
        
        for entry in entries {
            if let Ok(entry) = entry {
                if let Ok(metadata) = entry.metadata() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let item_path = if current_path.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", current_path, name)
                    };
                    
                    items.push((name, item_path, metadata.is_dir()));
                }
            }
        }
        
        // 정렬: 디렉토리 먼저, 그 다음 파일
        items.sort_by(|a, b| {
            match (a.2, b.2) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.0.to_lowercase().cmp(&b.0.to_lowercase()),
            }
        });
        
        for (name, item_path, is_dir) in items {
            let indent = "  ".repeat(depth);
            if is_dir {
                // 하위 디렉토리가 있는지 확인
                let has_children = std::fs::read_dir(full_path.join(&name))
                    .map(|mut entries| entries.next().is_some())
                    .unwrap_or(false);
                
                if depth < 2 && has_children {
                    // 깊이가 2 미만이고 하위 폴더가 있으면 재귀적으로 생성
                    let children = generate_file_tree_sync(base_path, mount, &item_path, depth + 1);
                    result.push(format!(
                        r#"{}<div class="tree-folder" data-path="{}" data-mount="{}">
{}  <span class="folder-toggle">📁</span> <a href="/{}/{}">{}</a>
{}  <div class="tree-children collapsed">
{}
{}  </div>
{}</div>"#,
                        indent, item_path, mount, indent, mount, item_path, name, indent,
                        children,
                        indent, indent
                    ));
                } else {
                    // 빈 컨테이너로 생성 (동적 로딩용)
                    result.push(format!(
                        r#"{}<div class="tree-folder" data-path="{}" data-mount="{}">
{}  <span class="folder-toggle">📁</span> <a href="/{}/{}">{}</a>
{}  <div class="tree-children collapsed"></div>
{}</div>"#,
                        indent, item_path, mount, indent, mount, item_path, name, indent, indent
                    ));
                }
            } else {
                result.push(format!(
                    r#"{}<div class="tree-file"><span class="file-icon">📄</span> <a href="/{}/{}">{}</a></div>"#,
                    indent, mount, item_path, name
                ));
            }
        }
    }
    
    result.join("\n")
}

fn render_simple_markdown_for_preview(content: &str) -> String {
    // 미리보기용으로 간단하게 처리
    let lines: Vec<&str> = content.lines().take(20).collect(); // 처음 20줄만
    let mut result = Vec::new();
    
    for line in lines {
        let trimmed = line.trim();
        
        if trimmed.starts_with("# ") {
            result.push(format!("<h3>{}</h3>", &trimmed[2..]));
        } else if trimmed.starts_with("## ") {
            result.push(format!("<h4>{}</h4>", &trimmed[3..]));
        } else if trimmed.starts_with("- ") {
            result.push(format!("<li>{}</li>", &trimmed[2..]));
        } else if !trimmed.is_empty() {
            result.push(format!("<p>{}</p>", trimmed));
        }
    }
    
    format!("<div class='readme-preview'>{}</div>", result.join(""))
}

fn format_file_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = size as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
}

// API 엔드포인트: 폴더의 하위 항목들을 JSON으로 반환
pub async fn api_get_folder_tree(
    Path((mount, path)): Path<(String, String)>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // 마운트 포인트가 존재하는지 확인
    let mount_config = match state.config.get_mount(&mount) {
        Some(config) => config,
        None => {
            return Json(json!({
                "success": false,
                "error": "Mount point not found"
            })).into_response();
        }
    };

    // 실제 파일 시스템 경로 구성
    let mut full_path = mount_config.path.clone();
    if !path.is_empty() {
        full_path.push(&path);
    }

    // 경로 보안 검사
    if !is_safe_path(&mount_config.path, &full_path) {
        return Json(json!({
            "success": false,
            "error": "Access denied"
        })).into_response();
    }

    // 디렉토리 존재 확인
    let metadata = match fs::metadata(&full_path).await {
        Ok(metadata) => metadata,
        Err(_) => {
            return Json(json!({
                "success": false,
                "error": "Directory not found"
            })).into_response();
        }
    };

    if !metadata.is_dir() {
        return Json(json!({
            "success": false,
            "error": "Path is not a directory"
        })).into_response();
    }

    // 디렉토리 읽기
    let mut entries = match fs::read_dir(&full_path).await {
        Ok(entries) => entries,
        Err(_) => {
            return Json(json!({
                "success": false,
                "error": "Failed to read directory"
            })).into_response();
        }
    };

    let mut items = Vec::new();

    while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
        if let Ok(metadata) = entry.metadata().await {
            let name = entry.file_name().to_string_lossy().to_string();
            let item_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", path, name)
            };

            items.push(json!({
                "name": name,
                "path": item_path,
                "is_dir": metadata.is_dir(),
                "size": if !metadata.is_dir() { Some(metadata.len()) } else { None }
            }));
        }
    }

    // 정렬: 디렉토리 먼저, 그 다음 파일
    items.sort_by(|a, b| {
        let a_is_dir = a["is_dir"].as_bool().unwrap_or(false);
        let b_is_dir = b["is_dir"].as_bool().unwrap_or(false);
        let a_name = a["name"].as_str().unwrap_or("").to_lowercase();
        let b_name = b["name"].as_str().unwrap_or("").to_lowercase();

        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a_name.cmp(&b_name),
        }
    });

    Json(json!({
        "success": true,
        "items": items
    })).into_response()
}

// 마운트 포인트 전체를 tree-folder로 감싸서 생성하는 함수
fn generate_file_tree_for_mount(base_path: &PathBuf, mount: &str) -> String {
    // 최상위 마운트 트리 생성
    let children = generate_file_tree_sync(base_path, mount, "", 0);
    
    format!(
        r#"<div class="tree-folder" data-path="" data-mount="{}">
  <span class="folder-toggle">📁</span>
  <a href="/{}">{}</a>
  <div class="tree-children collapsed">
{}
  </div>
</div>"#,
        mount, mount, mount, children
    )
}

// HTML 헤더 생성 헬퍼 함수
fn generate_html_head(title: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{}</title>
    <link rel="stylesheet" href="/static/styles.css">
</head>"#,
        title
    )
}

// JavaScript 공통 코드
fn get_common_javascript() -> &'static str {
    r#"<script>
// 폴더 토글 및 동적 로딩 기능
document.addEventListener('DOMContentLoaded', function() {
    attachToggleListeners();
    
    function attachToggleListeners() {
        document.querySelectorAll('.folder-toggle').forEach(function(toggle) {
            if (toggle.hasAttribute('data-listener')) return;
            toggle.setAttribute('data-listener', 'true');
            
            toggle.addEventListener('click', function(e) {
                e.preventDefault();
                e.stopPropagation();
                
                const folder = this.closest('.tree-folder');
                const children = folder.querySelector('.tree-children');
                const path = folder.getAttribute('data-path');
                const mount = folder.getAttribute('data-mount');
                
                if (children.classList.contains('collapsed')) {
                    children.classList.remove('collapsed');
                    this.textContent = '📂';
                    this.classList.add('expanded');
                    
                    if (!children.hasAttribute('data-loaded') && children.innerHTML.trim() === '') {
                        loadSubfolders(mount, path, children);
                        children.setAttribute('data-loaded', 'true');
                    }
                } else {
                    children.classList.add('collapsed');
                    this.textContent = '📁';
                    this.classList.remove('expanded');
                }
            });
        });
    }
    
    async function loadSubfolders(mount, path, container) {
        try {
            container.innerHTML = '<div class="loading">로딩 중...</div>';
            const response = await fetch(`/api/${mount}/tree/${path}`);
            const data = await response.json();
            
            if (data.success && data.items && data.items.length > 0) {
                container.innerHTML = data.items.map(item => {
                    if (item.is_dir) {
                        return `<div class="tree-folder" data-path="${item.path}" data-mount="${mount}">
                                  <span class="folder-toggle">📁</span> 
                                  <a href="/${mount}/${item.path}">${item.name}</a>
                                  <div class="tree-children collapsed"></div>
                                </div>`;
                    } else {
                        return `<div class="tree-file">
                                  <span class="file-icon">📄</span> 
                                  <a href="/${mount}/${item.path}">${item.name}</a>
                                </div>`;
                    }
                }).join('');
                
                attachToggleListeners();
            } else {
                container.innerHTML = '<div style="color: var(--muted-foreground); font-size: 0.8em;">하위 폴더 없음</div>';
            }
        } catch (error) {
            console.error('폴더 로딩 실패:', error);
            container.innerHTML = '<div style="color: var(--destructive); font-size: 0.8em;">로딩 실패</div>';
        }
    }
});
</script>"#
}

pub async fn handle_zip_download(
    Path((mount,)): Path<(String,)>,
    State(state): State<AppState>,
    AxumJson(req): AxumJson<ZipRequest>,
) -> Result<Response<Body>, (StatusCode, String)> {
    // 마운트 포인트 확인
    let (_, mount_config) = state.config.mounts.iter()
        .find(|(name, _)| *name == &mount)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Mount not found".to_string()))?;

    // ZIP 파일을 메모리에 생성
    let buffer = Vec::new();
    let cursor = Cursor::new(buffer);
    let mut zip = ZipWriter::new(cursor);
    
    let options = FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    // 각 선택된 파일을 ZIP에 추가
    for path in &req.paths {
        let full_path = mount_config.path.join(path.trim_start_matches('/'));
        
        // 경로 검증 (path traversal 방지)
        if !full_path.starts_with(&mount_config.path) {
            continue;
        }

        if full_path.is_file() {
            // 파일 읽기
            let content: Vec<u8> = match tokio::fs::read(&full_path).await {
                Ok(c) => c,
                Err(_) => continue, // 읽기 실패한 파일은 스킵
            };

            // ZIP에 파일 추가
            let zip_path = path.trim_start_matches('/');
            if let Err(_) = zip.start_file(zip_path, options) {
                continue;
            }
            if let Err(_) = zip.write_all(&content) {
                continue;
            }
        }
    }

    // ZIP 완료
    let cursor = zip.finish().map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to finalize zip: {}", e))
    })?;

    let buffer = cursor.into_inner();

    // ZIP 파일명 생성
    let filename = format!("{}_files.zip", mount);

    // 응답 생성
    let body = Body::from(buffer);
    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/zip")
        .header("Content-Disposition", format!("attachment; filename=\"{}\"", filename))
        .body(body)
        .map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to build response: {}", e))
        })?;

    Ok(response)
}