mod config;
mod web;

use config::Config;
use std::sync::Arc;
use tokio::fs;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use web::{create_router, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 로깅 초기화
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 설정 파일 로드
    let config = match Config::load_from_file("config.toml") {
        Ok(config) => config,
        Err(_) => {
            println!("config.toml not found, creating default configuration...");
            let default_config = Config::default();
            default_config.save_to_file("config.toml")?;
            
            // 기본 디렉토리들 생성
            for (_, mount) in &default_config.mounts {
                if let Err(e) = fs::create_dir_all(&mount.path).await {
                    println!("Warning: Failed to create directory {:?}: {}", mount.path, e);
                }
            }
            
            default_config
        }
    };

    // 마운트 포인트 디렉토리들이 존재하는지 확인하고 생성
    for (name, mount) in &config.mounts {
        if !mount.path.exists() {
            println!("Creating directory for mount '{}': {:?}", name, mount.path);
            if let Err(e) = fs::create_dir_all(&mount.path).await {
                println!("Warning: Failed to create directory {:?}: {}", mount.path, e);
            }
        }
    }

    // 애플리케이션 상태 생성
    let state = AppState {
        config: Arc::new(config.clone()),
    };

    // 라우터 생성
    let app = create_router(state);

    // 서버 시작
    let listener = tokio::net::TcpListener::bind(format!("{}:{}", config.server.host, config.server.port)).await?;
    
    println!("🚀 LunaFinder started on http://{}:{}", config.server.host, config.server.port);
    println!("📁 Available mount points:");
    for (name, mount) in &config.mounts {
        println!("  - {} -> {:?}", name, mount.path);
    }

    axum::serve(listener, app).await?;

    Ok(())
}
