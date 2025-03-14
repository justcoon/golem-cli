// Copyright 2024-2025 Golem Cloud
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::cloud::CloudAuthenticationConfig;
use crate::model::{Format, HasFormatConfig};
use anyhow::{anyhow, bail, Context};
use golem_wasm_rpc_stubgen::log::LogColorize;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::Duration;
use url::Url;

const CLOUD_URL: &str = "https://release.api.golem.cloud";
const DEFAULT_OSS_URL: &str = "http://localhost:9881";

// TODO: review and separate model, config and serialization parts
// TODO: when doing the serialization we can do a legacy migration

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub profiles: HashMap<ProfileName, Profile>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub default_profile: Option<ProfileName>,
    // TODO: these are deprecated now, remove them properly
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_profile: Option<ProfileName>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_cloud_profile: Option<ProfileName>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct ProfileName(pub String);

impl ProfileName {
    pub fn local() -> Self {
        ProfileName("local".to_owned())
    }
    pub fn cloud() -> Self {
        ProfileName("cloud".to_owned())
    }
}

impl Display for ProfileName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ProfileName {
    fn from(name: &str) -> Self {
        Self(name.to_string())
    }
}

impl From<String> for ProfileName {
    fn from(name: String) -> Self {
        Self(name)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct BuildProfileName(pub String);

impl Display for BuildProfileName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for BuildProfileName {
    fn from(name: &str) -> Self {
        Self(name.to_string())
    }
}

impl From<String> for BuildProfileName {
    fn from(name: String) -> Self {
        Self(name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedProfile {
    pub name: ProfileName,
    pub profile: Profile,
}

#[derive(Debug, Copy, Clone)]
pub enum ProfileKind {
    Oss,
    Cloud,
}

// TODO: cannot rename enum cases without migration, as that breaks the format,
//       if we have to migrate once, we should use a more "user-friendly" format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Profile {
    Golem(OssProfile),
    GolemCloud(CloudProfile),
}

impl Profile {
    pub fn config(self) -> ProfileConfig {
        match self {
            Profile::Golem(p) => p.config,
            Profile::GolemCloud(p) => p.config,
        }
    }

    pub fn get_config(&self) -> &ProfileConfig {
        match self {
            Profile::Golem(p) => &p.config,
            Profile::GolemCloud(p) => &p.config,
        }
    }

    pub fn get_config_mut(&mut self) -> &mut ProfileConfig {
        match self {
            Profile::Golem(p) => &mut p.config,
            Profile::GolemCloud(p) => &mut p.config,
        }
    }

    pub fn kind(&self) -> ProfileKind {
        match self {
            Profile::Golem(_) => ProfileKind::Oss,
            Profile::GolemCloud(_) => ProfileKind::Cloud,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CloudProfile {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub custom_url: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub custom_cloud_url: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub custom_worker_url: Option<Url>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub allow_insecure: bool,
    #[serde(default)]
    pub config: ProfileConfig,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub auth: Option<CloudAuthenticationConfig>,
}

impl HasFormatConfig for CloudProfile {
    fn format(&self) -> Option<Format> {
        Some(self.config.default_format)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OssProfile {
    pub url: Url,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub worker_url: Option<Url>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub allow_insecure: bool,
    #[serde(default)]
    pub config: ProfileConfig,
}

impl HasFormatConfig for OssProfile {
    fn format(&self) -> Option<Format> {
        Some(self.config.default_format)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Eq, PartialEq)]
pub struct ProfileConfig {
    #[serde(default)]
    pub default_format: Format,
}

impl Config {
    fn config_path(config_dir: &Path) -> PathBuf {
        config_dir.join("config.json")
    }

    fn from_file(config_dir: &Path) -> anyhow::Result<Config> {
        let config_path = Self::config_path(config_dir);

        if !config_path
            .try_exists()
            .with_context(|| anyhow!("Failed to check config file: {}", config_path.display()))?
        {
            return Ok(Config::default().with_local_and_cloud_profiles());
        }

        let file = File::open(&config_path)
            .with_context(|| anyhow!("Failed to open config file: {}", config_path.display()))?;

        let reader = BufReader::new(file);
        let mut config: Config = serde_json::from_reader(reader).with_context(|| {
            anyhow!(
                "Failed to deserialize config file {}",
                config_path.display(),
            )
        })?;

        // Detect if it was not yet migrated
        if config.default_profile.is_none() {
            // Drop old default profiles
            config.profiles.remove(&ProfileName::from("default"));
            config.profiles.remove(&ProfileName::from("cloud_default"));

            // Rename profiles that are conflicting with the new ones
            if let Some(profile) = config.profiles.remove(&ProfileName::from("local")) {
                config
                    .profiles
                    .insert(ProfileName::from("local-migrated"), profile);
            };
            if let Some(profile) = config.profiles.remove(&ProfileName::from("cloud")) {
                config
                    .profiles
                    .insert(ProfileName::from("cloud-migrated"), profile);
            }

            // Set default to local
            config.default_profile = Some(ProfileName::from("local"));

            // Save migrated config
            config.store_file(config_dir).with_context(|| {
                anyhow!(
                    "Failed to save config after migration: {}",
                    config_path.display()
                )
            })?;
        }

        Ok(config.with_local_and_cloud_profiles())
    }

    fn with_local_and_cloud_profiles(mut self) -> Self {
        self.profiles
            .entry(ProfileName::local())
            .or_insert_with(|| {
                let url = Url::parse(DEFAULT_OSS_URL).unwrap();
                Profile::Golem(OssProfile {
                    url,
                    worker_url: None,
                    allow_insecure: false,
                    config: ProfileConfig::default(),
                })
            });

        self.profiles
            .entry(ProfileName::cloud())
            .or_insert_with(|| Profile::GolemCloud(CloudProfile::default()));

        if self.default_profile.is_none() {
            self.default_profile = Some(ProfileName::local())
        }

        self
    }

    fn store_file(&self, config_dir: &Path) -> anyhow::Result<()> {
        create_dir_all(config_dir)
            .map_err(|err| anyhow!("Can't create config directory: {err}"))?;

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(Self::config_path(config_dir))
            .map_err(|err| anyhow!("Can't open config file: {err}"))?;
        let writer = BufWriter::new(file);

        serde_json::to_writer_pretty(writer, self)
            .map_err(|err| anyhow!("Can't save config to file: {err}"))
    }

    pub fn set_active_profile_name(
        profile_name: ProfileName,
        config_dir: &Path,
    ) -> anyhow::Result<()> {
        let mut config = Self::from_file(config_dir)?;

        if !config.profiles.contains_key(&profile_name) {
            bail!(
                "No profile {profile_name} in configuration. Available profiles: [{}]",
                config.profiles.keys().map(|n| &n.0).join(", ")
            );
        };

        config.default_profile = Some(profile_name);

        config.store_file(config_dir)?;

        Ok(())
    }

    pub fn get_active_profile(
        config_dir: &Path,
        selected_profile: Option<ProfileName>,
    ) -> anyhow::Result<NamedProfile> {
        let mut config = Self::from_file(config_dir)?;

        let name = selected_profile
            .unwrap_or_else(|| config.default_profile.unwrap_or_else(ProfileName::local));

        match config.profiles.remove(&name) {
            Some(profile) => Ok(NamedProfile {
                name: name.clone(),
                profile,
            }),
            None => {
                // TODO: add a hint error for this, and list profiles?
                bail!("Profile {} not found!", name.0.log_color_highlight());
            }
        }
    }

    pub fn get_profile(name: &ProfileName, config_dir: &Path) -> anyhow::Result<Option<Profile>> {
        let mut config = Self::from_file(config_dir)?;
        Ok(config.profiles.remove(name))
    }

    pub fn set_profile(
        name: ProfileName,
        profile: Profile,
        config_dir: &Path,
    ) -> anyhow::Result<()> {
        let mut config = Self::from_file(config_dir)?;
        config.profiles.insert(name, profile);
        config.store_file(config_dir)
    }

    pub fn delete_profile(name: &ProfileName, config_dir: &Path) -> anyhow::Result<()> {
        let mut config = Self::from_file(config_dir)?;
        config.profiles.remove(name);
        config.store_file(config_dir)
    }
}

pub struct ClientConfig {
    pub component_url: Url,
    pub worker_url: Url,
    pub cloud_url: Option<Url>,
    pub service_http_client_config: HttpClientConfig,
    pub health_check_http_client_config: HttpClientConfig,
    pub file_download_http_client_config: HttpClientConfig,
}

impl From<&Profile> for ClientConfig {
    fn from(profile: &Profile) -> Self {
        match profile {
            Profile::Golem(profile) => {
                let allow_insecure = profile.allow_insecure;

                ClientConfig {
                    component_url: profile.url.clone(),
                    worker_url: profile
                        .worker_url
                        .clone()
                        .unwrap_or_else(|| profile.url.clone()),
                    cloud_url: None,
                    service_http_client_config: HttpClientConfig::new_for_service_calls(
                        allow_insecure,
                    ),
                    health_check_http_client_config: HttpClientConfig::new_for_health_check(
                        allow_insecure,
                    ),
                    file_download_http_client_config: HttpClientConfig::new_for_file_download(
                        allow_insecure,
                    ),
                }
            }
            Profile::GolemCloud(profile) => {
                let default_cloud_url = Url::parse(CLOUD_URL).unwrap();
                let component_url = profile.custom_url.clone().unwrap_or(default_cloud_url);
                let cloud_url = Some(
                    profile
                        .custom_cloud_url
                        .clone()
                        .unwrap_or_else(|| component_url.clone()),
                );
                let worker_url = profile
                    .custom_worker_url
                    .clone()
                    .unwrap_or_else(|| component_url.clone());
                let allow_insecure = profile.allow_insecure;

                ClientConfig {
                    component_url,
                    worker_url,
                    cloud_url,
                    service_http_client_config: HttpClientConfig::new_for_service_calls(
                        allow_insecure,
                    ),
                    health_check_http_client_config: HttpClientConfig::new_for_health_check(
                        allow_insecure,
                    ),
                    file_download_http_client_config: HttpClientConfig::new_for_file_download(
                        allow_insecure,
                    ),
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    pub allow_insecure: bool,
    pub timeout: Option<Duration>,
    pub connect_timeout: Option<Duration>,
    pub read_timeout: Option<Duration>,
}

impl HttpClientConfig {
    pub fn new_for_service_calls(allow_insecure: bool) -> Self {
        Self {
            allow_insecure,
            timeout: None,
            connect_timeout: None,
            read_timeout: None,
        }
        .with_env_overrides("GOLEM_HTTP")
    }

    pub fn new_for_health_check(allow_insecure: bool) -> Self {
        Self {
            allow_insecure,
            timeout: Some(Duration::from_secs(2)),
            connect_timeout: Some(Duration::from_secs(1)),
            read_timeout: Some(Duration::from_secs(1)),
        }
        .with_env_overrides("GOLEM_HTTP_HEALTHCHECK")
    }

    pub fn new_for_file_download(allow_insecure: bool) -> Self {
        Self {
            allow_insecure,
            timeout: Some(Duration::from_secs(60)),
            connect_timeout: Some(Duration::from_secs(10)),
            read_timeout: Some(Duration::from_secs(60)),
        }
        .with_env_overrides("GOLEM_HTTP_FILE_DOWNLOAD")
    }

    fn with_env_overrides(mut self, prefix: &str) -> Self {
        fn env_duration(name: &str) -> Option<Duration> {
            let duration_str = std::env::var(name).ok()?;
            Some(iso8601::duration(&duration_str).ok()?.into())
        }

        let duration_fields: Vec<(&str, &mut Option<Duration>)> = vec![
            ("TIMEOUT", &mut self.timeout),
            ("CONNECT_TIMEOUT", &mut self.connect_timeout),
            ("READ_TIMEOUT", &mut self.read_timeout),
        ];

        for (env_var_name, field) in duration_fields {
            if let Some(duration) = env_duration(&format!("{}_{}", prefix, env_var_name)) {
                *field = Some(duration);
            }
        }

        self
    }
}
