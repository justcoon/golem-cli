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

use crate::command::api::definition::ApiDefinitionSubcommand;
use crate::command::shared_args::ProjectNameOptionalArg;
use crate::command_handler::Handlers;
use crate::context::{Context, GolemClients};
use crate::error::service::AnyhowMapServiceError;
use crate::model::text::api_definition::{
    ApiDefinitionGetView, ApiDefinitionNewView, ApiDefinitionUpdateView,
};
use crate::model::{
    ApiDefinitionFileFormat, ApiDefinitionId, ApiDefinitionVersion, PathBufOrStdin,
};
use anyhow::{anyhow, Context as AnyhowContext};
use golem_client::api::ApiDefinitionClient as ApiDefinitionClientOss;
use golem_client::model::HttpApiDefinitionRequest as HttpApiDefinitionRequestOss;
use golem_cloud_client::api::ApiDefinitionClient as ApiDefinitionClientCloud;
use golem_cloud_client::model::HttpApiDefinitionRequest as HttpApiDefinitionRequestCloud;
use golem_wasm_rpc_stubgen::log::{log_warn_action, LogColorize};
use serde::de::DeserializeOwned;
use std::io::Read;
use std::sync::Arc;
use tokio::fs::read_to_string;

pub struct ApiDefinitionCommandHandler {
    ctx: Arc<Context>,
}

impl ApiDefinitionCommandHandler {
    pub fn new(ctx: Arc<Context>) -> Self {
        Self { ctx }
    }

    pub async fn handle_command(&mut self, command: ApiDefinitionSubcommand) -> anyhow::Result<()> {
        match command {
            ApiDefinitionSubcommand::New {
                project,
                definition,
                def_format,
            } => self.cmd_new(project, definition, def_format).await,
            ApiDefinitionSubcommand::Update {
                project,
                definition,
                def_format,
            } => self.cmd_update(project, definition, def_format).await,
            ApiDefinitionSubcommand::Import {
                project,
                definition,
                def_format,
            } => self.cmd_import(project, definition, def_format).await,
            ApiDefinitionSubcommand::Get {
                project,
                id,
                version,
            } => self.cmd_get(project, id, version).await,
            ApiDefinitionSubcommand::Delete {
                project,
                id,
                version,
            } => self.cmd_delete(project, id, version).await,
            ApiDefinitionSubcommand::List { project, id } => self.list(project, id).await,
        }
    }

    async fn cmd_new(
        &self,
        project: ProjectNameOptionalArg,
        definition: PathBufOrStdin,
        format: Option<ApiDefinitionFileFormat>,
    ) -> anyhow::Result<()> {
        let project = self
            .ctx
            .cloud_project_handler()
            .opt_select_project(None /* TODO: account id */, project.project.as_ref())
            .await?;

        let result = match self.ctx.golem_clients().await? {
            GolemClients::Oss(clients) => clients
                .api_definition
                .create_definition_json(&read_and_parse_api_definition(definition, format).await?)
                .await
                .map_service_error()?,
            GolemClients::Cloud(clients) => {
                let project = self
                    .ctx
                    .cloud_project_handler()
                    .selected_project_or_default(project)
                    .await?;
                clients
                    .api_definition
                    .create_definition_json(
                        &project.project_id.0,
                        &read_and_parse_api_definition(definition, format).await?,
                    )
                    .await
                    .map_service_error()?
            }
        };

        self.ctx
            .log_handler()
            .log_view(&ApiDefinitionNewView(result));

        Ok(())
    }

    async fn cmd_get(
        &self,
        project: ProjectNameOptionalArg,
        api_def_id: ApiDefinitionId,
        version: ApiDefinitionVersion,
    ) -> anyhow::Result<()> {
        let project = self
            .ctx
            .cloud_project_handler()
            .opt_select_project(None /* TODO: account id */, project.project.as_ref())
            .await?;

        let result = match self.ctx.golem_clients().await? {
            GolemClients::Oss(clients) => clients
                .api_definition
                .get_definition(&api_def_id.0, &version.0)
                .await
                .map_service_error()?,
            GolemClients::Cloud(clients) => {
                let project = self
                    .ctx
                    .cloud_project_handler()
                    .selected_project_or_default(project)
                    .await?;
                clients
                    .api_definition
                    .get_definition(&project.project_id.0, &api_def_id.0, &version.0)
                    .await
                    .map_service_error()?
            }
        };

        self.ctx
            .log_handler()
            .log_view(&ApiDefinitionGetView(result));

        Ok(())
    }

    async fn cmd_update(
        &self,
        project: ProjectNameOptionalArg,
        definition: PathBufOrStdin,
        format: Option<ApiDefinitionFileFormat>,
    ) -> anyhow::Result<()> {
        let project = self
            .ctx
            .cloud_project_handler()
            .opt_select_project(None /* TODO: account id */, project.project.as_ref())
            .await?;

        let result = match self.ctx.golem_clients().await? {
            GolemClients::Oss(clients) => {
                let api_def: HttpApiDefinitionRequestOss =
                    read_and_parse_api_definition(definition, format).await?;
                clients
                    .api_definition
                    .update_definition_json(&api_def.id, &api_def.version, &api_def)
                    .await
                    .map_service_error()?
            }
            GolemClients::Cloud(clients) => {
                let api_def: HttpApiDefinitionRequestCloud =
                    read_and_parse_api_definition(definition, format).await?;
                let project = self
                    .ctx
                    .cloud_project_handler()
                    .selected_project_or_default(project)
                    .await?;
                clients
                    .api_definition
                    .update_definition_json(
                        &project.project_id.0,
                        &api_def.id,
                        &api_def.version,
                        &api_def,
                    )
                    .await
                    .map_service_error()?
            }
        };

        self.ctx
            .log_handler()
            .log_view(&ApiDefinitionUpdateView(result));

        Ok(())
    }

    async fn cmd_import(
        &self,
        project: ProjectNameOptionalArg,
        definition: PathBufOrStdin,
        format: Option<ApiDefinitionFileFormat>,
    ) -> anyhow::Result<()> {
        let project = self
            .ctx
            .cloud_project_handler()
            .opt_select_project(None /* TODO: account id */, project.project.as_ref())
            .await?;

        let result = match self.ctx.golem_clients().await? {
            GolemClients::Oss(clients) => clients
                .api_definition
                .import_open_api_json(&read_and_parse_api_definition(definition, format).await?)
                .await
                .map_service_error()?,
            GolemClients::Cloud(clients) => {
                let project = self
                    .ctx
                    .cloud_project_handler()
                    .selected_project_or_default(project)
                    .await?;
                clients
                    .api_definition
                    .import_open_api_json(
                        &project.project_id.0,
                        &read_and_parse_api_definition(definition, format).await?,
                    )
                    .await
                    .map_service_error()?
            }
        };

        self.ctx
            .log_handler()
            .log_view(&ApiDefinitionUpdateView(result));

        Ok(())
    }

    async fn list(
        &self,
        project: ProjectNameOptionalArg,
        api_definition_id: Option<ApiDefinitionId>,
    ) -> anyhow::Result<()> {
        let project = self
            .ctx
            .cloud_project_handler()
            .opt_select_project(None /* TODO: account id */, project.project.as_ref())
            .await?;

        let definitions = match self.ctx.golem_clients().await? {
            GolemClients::Oss(clients) => clients
                .api_definition
                .list_definitions(api_definition_id.as_ref().map(|id| id.0.as_str()))
                .await
                .map_service_error()?,
            GolemClients::Cloud(clients) => {
                let project = self
                    .ctx
                    .cloud_project_handler()
                    .selected_project_or_default(project)
                    .await?;
                clients
                    .api_definition
                    .list_definitions(
                        &project.project_id.0,
                        api_definition_id.as_ref().map(|id| id.0.as_str()),
                    )
                    .await
                    .map_service_error()?
            }
        };

        self.ctx.log_handler().log_view(&definitions);

        Ok(())
    }

    async fn cmd_delete(
        &self,
        project: ProjectNameOptionalArg,
        api_def_id: ApiDefinitionId,
        version: ApiDefinitionVersion,
    ) -> anyhow::Result<()> {
        let project = self
            .ctx
            .cloud_project_handler()
            .opt_select_project(None /* TODO: account id */, project.project.as_ref())
            .await?;

        match self.ctx.golem_clients().await? {
            GolemClients::Oss(clients) => clients
                .api_definition
                .delete_definition(&api_def_id.0, &version.0)
                .await
                .map_service_error()?,
            GolemClients::Cloud(clients) => {
                let project = self
                    .ctx
                    .cloud_project_handler()
                    .selected_project_or_default(project)
                    .await?;
                clients
                    .api_definition
                    .delete_definition(&project.project_id.0, &api_def_id.0, &version.0)
                    .await
                    .map_service_error()?
            }
        };

        log_warn_action(
            "Deleted",
            format!(
                "API definition: {}/{}",
                api_def_id.0.log_color_highlight(),
                version.0.log_color_highlight()
            ),
        );

        Ok(())
    }
}

async fn read_definition(source: PathBufOrStdin) -> anyhow::Result<String> {
    match source {
        PathBufOrStdin::Path(path) => read_to_string(&path)
            .await
            .with_context(|| anyhow!("Failed to read definition file: {}", path.display())),
        PathBufOrStdin::Stdin => {
            let mut content = String::new();
            let _ = std::io::stdin()
                .read_to_string(&mut content)
                .with_context(|| anyhow!("Failed to read definition from STDIN"))?;
            Ok(content)
        }
    }
}

fn parse_api_definition<T: DeserializeOwned>(
    input: &str,
    format: Option<ApiDefinitionFileFormat>,
) -> anyhow::Result<T> {
    match format.unwrap_or(ApiDefinitionFileFormat::Yaml) {
        ApiDefinitionFileFormat::Json => {
            serde_json::from_str(input).context("Failed to parse API definition as JSON")
        }
        ApiDefinitionFileFormat::Yaml => {
            serde_yaml::from_str(input).context("Failed to parse API definition as YAML")
        }
    }
}

async fn read_and_parse_api_definition<T: DeserializeOwned>(
    source: PathBufOrStdin,
    format: Option<ApiDefinitionFileFormat>,
) -> anyhow::Result<T> {
    parse_api_definition(&read_definition(source).await?, format)
}
