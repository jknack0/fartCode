//! Pure derivation of config selector groups from raw ACP
//! `SessionConfigOption` arrays (reference `reducer/config-derive.ts`).
//!
//! Category mapping: `model` → model_options, `thought_level` → efforts,
//! `mode` → mode_options. Unknown categories (e.g. Claude's `model_config`
//! fast-mode toggle) are silently ignored — they stay extension points.
//! Returns partial: only groups present in `options` are set.

use agent_client_protocol_schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOptions,
};

use crate::transcript::models::{ConfigChoice, SelectGroup};

/// Groups derived from one `config_option_update`; missing groups stay
/// `None` (the runtime merges this partial into the existing state).
#[derive(Debug, Clone, Default)]
pub struct ConfigGroups {
    pub model_options: Option<SelectGroup>,
    pub efforts: Option<SelectGroup>,
    pub mode_options: Option<SelectGroup>,
}

pub fn derive_config_groups(options: &[SessionConfigOption]) -> ConfigGroups {
    let mut groups = ConfigGroups::default();

    for opt in options {
        let SessionConfigKind::Select(select) = &opt.kind else {
            continue;
        };
        let flat = match &select.options {
            SessionConfigSelectOptions::Ungrouped(list) => list.clone(),
            SessionConfigSelectOptions::Grouped(groups) => groups
                .iter()
                .flat_map(|g| g.options.iter().cloned())
                .collect(),
            // Non-exhaustive schema: unknown option containers contribute nothing.
            _ => Vec::new(),
        };
        let available: Vec<ConfigChoice> = flat
            .into_iter()
            .map(|raw| ConfigChoice {
                id: raw.value.0.to_string(),
                name: raw.name,
                description: raw.description,
            })
            .collect();
        let group = SelectGroup {
            config_id: opt.id.0.to_string(),
            selected: Some(select.current_value.0.to_string()),
            available,
        };

        match opt.category.as_ref() {
            Some(SessionConfigOptionCategory::Model) => groups.model_options = Some(group),
            Some(SessionConfigOptionCategory::ThoughtLevel) => groups.efforts = Some(group),
            Some(SessionConfigOptionCategory::Mode) => groups.mode_options = Some(group),
            // ModelConfig / Other / missing categories are ignored.
            _ => {}
        }
    }

    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(json: serde_json::Value) -> SessionConfigOption {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn maps_known_categories_and_ignores_the_rest() {
        let options = vec![
            option(serde_json::json!({
                "id": "model",
                "name": "Model",
                "category": "model",
                "type": "select",
                "currentValue": "sonnet",
                "options": [
                    { "value": "sonnet", "name": "Sonnet" },
                    { "value": "opus", "name": "Opus", "description": "Big" },
                ],
            })),
            option(serde_json::json!({
                "id": "mode",
                "name": "Mode",
                "category": "mode",
                "type": "select",
                "currentValue": "default",
                "options": [{ "value": "default", "name": "Default" }],
            })),
            option(serde_json::json!({
                "id": "fast",
                "name": "Fast mode",
                "category": "model_config",
                "type": "boolean",
                "currentValue": true,
            })),
            option(serde_json::json!({
                "id": "thinking",
                "name": "Thinking",
                "category": "thought_level",
                "type": "select",
                "currentValue": "high",
                "options": [{ "value": "high", "name": "High" }],
            })),
        ];

        let groups = derive_config_groups(&options);
        let models = groups.model_options.unwrap();
        assert_eq!(models.config_id, "model");
        assert_eq!(models.selected.as_deref(), Some("sonnet"));
        assert_eq!(models.available.len(), 2);
        assert_eq!(models.available[1].description.as_deref(), Some("Big"));

        assert_eq!(groups.mode_options.unwrap().config_id, "mode");
        assert_eq!(groups.efforts.unwrap().config_id, "thinking");
        // model_config boolean option is ignored.
    }

    #[test]
    fn flattens_grouped_options() {
        let options = vec![option(serde_json::json!({
            "id": "model",
            "name": "Model",
            "category": "model",
            "type": "select",
            "currentValue": "b",
            "options": [
                { "group": "fast", "name": "Fast", "options": [{ "value": "a", "name": "A" }] },
                { "group": "smart", "name": "Smart", "options": [{ "value": "b", "name": "B" }] },
            ],
        }))];
        let groups = derive_config_groups(&options);
        let models = groups.model_options.unwrap();
        let ids: Vec<&str> = models.available.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }
}
