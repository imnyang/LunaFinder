use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub mounts: HashMap<String, MountPoint>,
    #[serde(default)]
    pub main_page: MainPageConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MainPageConfig {
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default = "default_description")]
    pub description: String,
    pub markdown_file: Option<PathBuf>,
}

fn default_title() -> String {
    "LunaFinder File Browser".to_string()
}

fn default_description() -> String {
    "Browse all available mount points and their contents".to_string()
}

impl Default for MainPageConfig {
    fn default() -> Self {
        MainPageConfig {
            title: default_title(),
            description: default_description(),
            markdown_file: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountPoint {
    pub path: PathBuf,
    pub description: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let mut mounts = HashMap::new();
        
        mounts.insert("home".to_string(), MountPoint {
            path: PathBuf::from("./files/home"),
            description: Some("Home directory".to_string()),
        });
        
        mounts.insert("documents".to_string(), MountPoint {
            path: PathBuf::from("./files/documents"),
            description: Some("Documents storage".to_string()),
        });
        
        mounts.insert("public".to_string(), MountPoint {
            path: PathBuf::from("./files/public"),
            description: Some("Public files".to_string()),
        });
        
        Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
            },
            mounts,
            main_page: MainPageConfig::default(),
        }
    }
}

impl Config {
    pub fn load_from_file(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
    
    pub fn save_to_file(&self, path: &str) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
    
    pub fn get_mount(&self, mount_name: &str) -> Option<&MountPoint> {
        self.mounts.get(mount_name)
    }
}