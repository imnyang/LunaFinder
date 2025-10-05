use anyhow::{Context, Result};
use serde::de::{self, Deserializer, SeqAccess};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub main_page: MainPageConfig,
    #[serde(rename = "user")]
    pub users: HashMap<String, UserConfig>,
    #[serde(rename = "mounts")]
    pub mounts: HashMap<String, MountConfig>,
    #[serde(default)]
    pub permissions: HashMap<String, PermissionProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MainPageConfig {
    pub title: String,
    pub description: String,
    pub markdown_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub password: String,
    pub group: Vec<String>,
    pub hash_algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    pub path: PathBuf,
    pub description: String,
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub group: HashMap<String, PermissionSpec>,
    #[serde(default)]
    pub user: HashMap<String, PermissionSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Permission {
    actions: BTreeSet<String>,
}

impl Permission {
    pub fn from_actions<I, S>(actions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut permission = Permission::default();
        for action in actions {
            permission.add_action(action);
        }
        permission
    }

    pub fn add_action<S: AsRef<str>>(&mut self, action: S) {
        let action = action.as_ref().trim();
        if action.is_empty() {
            return;
        }
        self.actions.insert(action.to_lowercase());
    }

    pub fn merge(&mut self, other: &Permission) {
        for action in &other.actions {
            self.actions.insert(action.clone());
        }
    }

    pub fn allows_action(&self, action: &str) -> bool {
        self.actions.contains(&action.to_lowercase())
    }

    pub fn allows_any(&self, actions: &[&str]) -> bool {
        actions.iter().any(|action| self.allows_action(action))
    }

    pub fn allows_read(&self) -> bool {
        self.allows_action("read") || self.allows_write()
    }

    pub fn allows_write(&self) -> bool {
        self.allows_any(&[
            "write",
            "upload",
            "delete",
            "rename",
            "modify",
            "create_file",
            "create_folder",
        ])
    }

    pub fn allows_upload(&self) -> bool {
        self.allows_any(&["upload", "write", "create_file"])
    }

    pub fn allows_delete(&self) -> bool {
        self.allows_any(&["delete", "write"])
    }

    pub fn allows_rename(&self) -> bool {
        self.allows_any(&["rename", "write"])
    }

    pub fn allows_modify(&self) -> bool {
        self.allows_any(&["modify", "write"])
    }

    #[allow(dead_code)]
    pub fn allows_create_file(&self) -> bool {
        self.allows_any(&["create_file", "write"])
    }

    #[allow(dead_code)]
    pub fn allows_create_folder(&self) -> bool {
        self.allows_any(&["create_folder", "write"])
    }

    pub fn actions(&self) -> Vec<String> {
        self.actions.iter().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.actions().join(", "))
    }
}

impl serde::ser::Serialize for Permission {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        self.actions().serialize(serializer)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSpec {
    entries: Vec<String>,
}

impl PermissionSpec {
    fn from_vec(values: Vec<String>) -> Self {
        let entries = values
            .into_iter()
            .flat_map(|value| {
                value
                    .split(',')
                    .map(|part| part.trim().to_string())
                    .collect::<Vec<_>>()
            })
            .filter(|entry| !entry.is_empty())
            .collect::<Vec<_>>();
        PermissionSpec { entries }
    }

    fn from_string(value: String) -> Self {
        PermissionSpec::from_vec(vec![value])
    }

    pub fn tokens(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.as_str())
    }
}

impl<'de> Deserialize<'de> for PermissionSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PermSpecVisitor;

        impl<'de> de::Visitor<'de> for PermSpecVisitor {
            type Value = PermissionSpec;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a comma separated string or array of permission tokens")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PermissionSpec::from_string(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PermissionSpec::from_string(value))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<String>()? {
                    values.push(value);
                }
                Ok(PermissionSpec::from_vec(values))
            }
        }

        deserializer.deserialize_any(PermSpecVisitor)
    }
}

impl serde::ser::Serialize for PermissionSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        match self.entries.len() {
            0 => {
                let seq = serializer.serialize_seq(Some(0))?;
                seq.end()
            }
            1 => serializer.serialize_str(&self.entries[0]),
            _ => {
                let mut seq = serializer.serialize_seq(Some(self.entries.len()))?;
                for entry in &self.entries {
                    seq.serialize_element(entry)?;
                }
                seq.end()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionProfile {
    #[serde(flatten)]
    actions: HashMap<String, bool>,
}

impl PermissionProfile {
    fn normalized(self) -> Self {
        let mut normalized = HashMap::new();
        for (action, allowed) in self.actions {
            let key = action.to_lowercase();
            let entry = normalized.entry(key).or_insert(false);
            *entry = *entry || allowed;
        }
        PermissionProfile {
            actions: normalized,
        }
    }

    fn to_permission(&self) -> Permission {
        let mut permission = Permission::default();
        for (action, allowed) in &self.actions {
            if *allowed {
                permission.add_action(action);
            }
        }
        permission
    }
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path))?;

        let mut config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path))?;

        config.normalize();

        Ok(config)
    }

    pub fn save(&self, path: &str) -> Result<()> {
        let toml = toml::to_string_pretty(self).context("Failed to serialize configuration")?;
        fs::write(path, toml).with_context(|| format!("Failed to write config file: {}", path))?;
        Ok(())
    }

    pub fn load_or_create(path: &str) -> Result<Self> {
        if Path::new(path).exists() {
            Self::load(path)
        } else {
            let mut config = Self::default();
            config.normalize();
            config.save(path)?;
            Ok(config)
        }
    }

    fn normalize(&mut self) {
        self.normalize_permissions();
    }

    fn normalize_permissions(&mut self) {
        let mut normalized = HashMap::new();
        for (name, profile) in mem::take(&mut self.permissions) {
            normalized.insert(name.to_lowercase(), profile.normalized());
        }
        self.permissions = normalized;
    }

    pub fn resolve_permission_spec(&self, spec: &PermissionSpec) -> Permission {
        let mut permission = Permission::default();
        for token in spec.tokens() {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            permission.merge(&self.resolve_permission_token(token));
        }
        permission
    }

    fn resolve_permission_token(&self, token: &str) -> Permission {
        let normalized = token.trim();
        if normalized.is_empty() {
            return Permission::default();
        }

        let lower = normalized.to_lowercase();
        match lower.as_str() {
            "r" | "read" => Permission::from_actions(["read"]),
            "w" | "write" => Permission::from_actions(["write"]),
            "rw" | "readwrite" | "read_write" => Permission::from_actions(["read", "write"]),
            other => {
                if let Some(profile) = self.permissions.get(other) {
                    profile.to_permission()
                } else {
                    Permission::from_actions([other])
                }
            }
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut mounts = HashMap::new();
        mounts.insert(
            "public".to_string(),
            MountConfig {
                path: PathBuf::from("./public"),
                description: "Public files".to_string(),
                public: true,
                group: HashMap::new(),
                user: HashMap::new(),
            },
        );

        Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
            },
            main_page: MainPageConfig {
                title: "LunaFinder".to_string(),
                description: "Welcome to LunaFinder".to_string(),
                markdown_file: "./page.md".to_string(),
            },
            users: HashMap::new(),
            mounts,
            permissions: HashMap::new(),
        }
    }
}
