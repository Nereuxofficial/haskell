use zed::lsp::{Symbol, SymbolKind};
use zed::{CodeLabel, CodeLabelSpan};
use zed_extension_api::process::Command;
use zed_extension_api::settings::LspSettings;
use zed_extension_api::{self as zed, Result};

struct HaskellExtension;

impl zed::Extension for HaskellExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let lsp_settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?;

        // If the user has specified a binary in their LSP settings,
        // that takes precedence.
        if let Some(binary_settings) = lsp_settings.binary {
            if let Some(path) = binary_settings.path {
                return Ok(zed::Command {
                    command: path,
                    args: binary_settings.arguments.unwrap_or_else(Vec::new),
                    env: worktree.shell_env(),
                });
            }
        }

        // Otherwise, default to hls installed via ghcup.
        let path = worktree
            .which("haskell-language-server-wrapper")
            .ok_or_else(|| "hls must be installed via ghcup".to_string())?;

        Ok(zed::Command {
            command: path,
            args: vec!["lsp".to_string()],
            env: worktree.shell_env(),
        })
    }

    fn label_for_symbol(
        &self,
        _language_server_id: &zed::LanguageServerId,
        symbol: Symbol,
    ) -> Option<CodeLabel> {
        let name = &symbol.name;

        let (code, display_range, filter_range) = match symbol.kind {
            SymbolKind::Struct => {
                let data_decl = "data ";
                let code = format!("{data_decl}{name} = A");
                let display_range = 0..data_decl.len() + name.len();
                let filter_range = data_decl.len()..display_range.end;
                (code, display_range, filter_range)
            }
            SymbolKind::Constructor => {
                let data_decl = "data A = ";
                let code = format!("{data_decl}{name}");
                let display_range = data_decl.len()..data_decl.len() + name.len();
                let filter_range = 0..name.len();
                (code, display_range, filter_range)
            }
            SymbolKind::Variable => {
                let code = format!("{name} :: T");
                let display_range = 0..name.len();
                let filter_range = 0..name.len();
                (code, display_range, filter_range)
            }
            _ => return None,
        };

        Some(CodeLabel {
            spans: vec![CodeLabelSpan::code_range(display_range)],
            filter_range: filter_range.into(),
            code,
        })
    }

    fn language_server_initialization_options_schema(&self, binary_path: String) -> Option<String> {
        // This is more difficult to do asynchronously...
        let output = Command::new(binary_path)
            .arg("vscode-extension-schema")
            .output()
            .ok()?;
        if output.status != Some(0) {
            return None;
        }
        let data = String::from_utf8_lossy(&output.stdout);
        // The schema emitted is not the one used by Zed.
        let value: serde_json::Value = serde_json::from_str(&data).ok()?;
        Some(convert_to_zed_schema(&value).to_string())
    }
}

fn convert_to_zed_schema(raw_schema: &serde_json::Value) -> serde_json::Value {
    let Some(schema_map) = raw_schema.as_object() else {
        return raw_schema.clone();
    };

    let mut root_properties = serde_json::Map::new();

    for (key, value) in schema_map {
        let parts: Vec<&str> = key.split('.').collect();
        if parts.is_empty() {
            continue;
        }

        // Skip root prefix "haskell"
        let parts = &parts[1..];
        if parts.is_empty() {
            continue;
        }

        insert_nested_property(&mut root_properties, parts, value);
    }

    serde_json::json!({
        "type": "object",
        "properties": root_properties
    })
}

fn insert_nested_property(
    properties: &mut serde_json::Map<String, serde_json::Value>,
    path: &[&str],
    leaf_value: &serde_json::Value,
) {
    if path.is_empty() {
        return;
    }

    let key = path[0].to_string();

    if path.len() == 1 {
        properties.insert(key, convert_leaf_schema(leaf_value));
    } else {
        let entry = properties.entry(key).or_insert_with(|| {
            serde_json::json!({
                "type": "object",
                "properties": {}
            })
        });

        if let Some(obj) = entry.as_object_mut() {
            let props = obj
                .entry("properties")
                .or_insert_with(|| serde_json::json!({}));

            if let Some(props_map) = props.as_object_mut() {
                insert_nested_property(props_map, &path[1..], leaf_value);
            }
        }
    }
}

fn convert_leaf_schema(leaf_value: &serde_json::Value) -> serde_json::Value {
    let Some(leaf_obj) = leaf_value.as_object() else {
        return leaf_value.clone();
    };

    let mut result = serde_json::Map::new();

    if let Some(desc) = leaf_obj.get("description") {
        result.insert("markdownDescription".to_string(), desc.clone());
    }
    if let Some(desc) = leaf_obj.get("markdownDescription") {
        result.insert("markdownDescription".to_string(), desc.clone());
    }

    for (key, value) in leaf_obj {
        match key.as_str() {
            "default" | "type" | "enum" | "enumDescriptions" | "items" | "minimum" | "maximum"
            | "anyOf" => {
                result.insert(key.clone(), value.clone());
            }
            "scope" | "description" => {}
            _ => {
                result.insert(key.clone(), value.clone());
            }
        }
    }

    serde_json::Value::Object(result)
}

zed::register_extension!(HaskellExtension);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_to_zed_schema() {
        let input = serde_json::json!({
            "haskell.plugin.alternateNumberFormat.globalOn": {
                "default": true,
                "description": "Enables alternateNumberFormat plugin",
                "scope": "resource",
                "type": "boolean"
            },
            "haskell.plugin.eval.config.diff": {
                "default": true,
                "markdownDescription": "Enable the diff output (WAS/NOW) of eval lenses",
                "scope": "resource",
                "type": "boolean"
            },
            "haskell.plugin.eval.globalOn": {
                "default": true,
                "description": "Enables eval plugin",
                "scope": "resource",
                "type": "boolean"
            }
        });

        let result = convert_to_zed_schema(&input);

        assert_eq!(result["type"], "object");
        assert!(result["properties"].is_object());

        let props = &result["properties"];

        assert!(props["plugin"].is_object());
        assert!(props["plugin"]["type"] == "object");
        assert!(props["plugin"]["properties"].is_object());
        assert!(props["plugin"]["properties"]["alternateNumberFormat"].is_object());
        assert!(
            props["plugin"]["properties"]["alternateNumberFormat"]["properties"]["globalOn"]
                .is_object()
        );

        let global_on =
            &props["plugin"]["properties"]["alternateNumberFormat"]["properties"]["globalOn"];
        assert_eq!(global_on["default"], true);
        assert_eq!(global_on["type"], "boolean");
        assert_eq!(
            global_on["markdownDescription"],
            "Enables alternateNumberFormat plugin"
        );
        assert!(global_on.get("scope").is_none());

        let eval_config_diff =
            &props["plugin"]["properties"]["eval"]["properties"]["config"]["properties"]["diff"];
        assert_eq!(eval_config_diff["default"], true);
        assert_eq!(eval_config_diff["type"], "boolean");
        assert_eq!(
            eval_config_diff["markdownDescription"],
            "Enable the diff output (WAS/NOW) of eval lenses"
        );

        let eval_global_on = &props["plugin"]["properties"]["eval"]["properties"]["globalOn"];
        assert_eq!(eval_global_on["default"], true);
        assert_eq!(eval_global_on["type"], "boolean");
        assert_eq!(eval_global_on["markdownDescription"], "Enables eval plugin");

        println!(
            "Converted schema: {}",
            serde_json::to_string_pretty(&result).unwrap()
        );
    }
}
