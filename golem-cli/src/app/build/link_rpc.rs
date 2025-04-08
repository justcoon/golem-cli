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

use crate::app::build::is_up_to_date;
use crate::app::build::task_result_marker::{LinkRpcMarkerHash, TaskResultMarker};
use crate::app::context::ApplicationContext;
use crate::fs;
use crate::log::{log_action, log_skipping_up_to_date, LogColorize, LogIndent};
use crate::model::app::DependencyType;
use crate::wasm_rpc_stubgen::commands;
use itertools::Itertools;
use std::collections::BTreeSet;

pub async fn link_rpc(ctx: &ApplicationContext) -> anyhow::Result<()> {
    log_action("Linking", "RPC");
    let _indent = LogIndent::new();

    for component_name in ctx.selected_component_names() {
        let static_dependencies = ctx
            .application
            .component_wasm_rpc_dependencies(component_name)
            .iter()
            .filter(|dep| dep.dep_type == DependencyType::StaticWasmRpc)
            .collect::<BTreeSet<_>>();
        let dynamic_dependencies = ctx
            .application
            .component_wasm_rpc_dependencies(component_name)
            .iter()
            .filter(|dep| dep.dep_type == DependencyType::DynamicWasmRpc)
            .collect::<BTreeSet<_>>();
        let client_wasms = static_dependencies
            .iter()
            .map(|dep| ctx.application.client_wasm(&dep.name))
            .collect::<Vec<_>>();
        let component_wasm = ctx
            .application
            .component_wasm(component_name, ctx.profile());
        let linked_wasm = ctx.application.component_linked_wasm(component_name);

        let task_result_marker = TaskResultMarker::new(
            &ctx.application.task_result_marker_dir(),
            LinkRpcMarkerHash {
                component_name,
                dependencies: &static_dependencies,
            },
        )?;

        if !dynamic_dependencies.is_empty() {
            log_action(
                "Found",
                format!(
                    "dynamic WASM RPC dependencies ({}) for {}",
                    dynamic_dependencies
                        .iter()
                        .map(|s| s.name.as_str().log_color_highlight())
                        .join(", "),
                    component_name.as_str().log_color_highlight(),
                ),
            );
        }

        if !static_dependencies.is_empty() {
            log_action(
                "Found",
                format!(
                    "static WASM RPC dependencies ({}) for {}",
                    static_dependencies
                        .iter()
                        .map(|s| s.name.as_str().log_color_highlight())
                        .join(", "),
                    component_name.as_str().log_color_highlight(),
                ),
            );
        }

        if is_up_to_date(
            ctx.config.skip_up_to_date_checks || !task_result_marker.is_up_to_date(),
            || {
                let mut inputs = client_wasms.clone();
                inputs.push(component_wasm.clone());
                inputs
            },
            || [linked_wasm.clone()],
        ) {
            log_skipping_up_to_date(format!(
                "linking RPC for {}",
                component_name.as_str().log_color_highlight(),
            ));
            continue;
        }

        task_result_marker.result(
            async {
                if static_dependencies.is_empty() {
                    log_action(
                        "Copying",
                        format!(
                            "{} without linking, no static WASM RPC dependencies were found",
                            component_name.as_str().log_color_highlight(),
                        ),
                    );
                    fs::copy(&component_wasm, &linked_wasm).map(|_| ())
                } else {
                    log_action(
                        "Linking",
                        format!(
                            "static WASM RPC dependencies ({}) into {}",
                            static_dependencies
                                .iter()
                                .map(|s| s.name.as_str().log_color_highlight())
                                .join(", "),
                            component_name.as_str().log_color_highlight(),
                        ),
                    );
                    let _indent = LogIndent::new();

                    commands::composition::compose(
                        ctx.application
                            .component_wasm(component_name, ctx.profile())
                            .as_path(),
                        &client_wasms,
                        linked_wasm.as_path(),
                    )
                    .await
                }
            }
            .await,
        )?;
    }

    Ok(())
}
