use super::*;

const BUNDLED: &[u8] = include_bytes!("../../../../catalogue/models-dev-v1.json");

#[test]
fn canonical_validation_rejects_unknown_fields_and_order_changes() {
    let value: serde_json::Value = serde_json::from_slice(BUNDLED).expect("catalogue");
    let mut unknown = value.clone();
    unknown["providers"][0]["unexpected"] = serde_json::Value::Bool(true);
    assert_eq!(
        validate_canonical_catalogue(&serde_json::to_vec(&unknown).expect("unknown"))
            .expect_err("unknown field"),
        "The checked-in catalogue is not canonical."
    );

    let mut reordered = value;
    reordered["providers"]
        .as_array_mut()
        .expect("providers")
        .swap(0, 1);
    assert_eq!(
        validate_canonical_catalogue(&serde_json::to_vec(&reordered).expect("reordered"))
            .expect_err("provider order"),
        "The checked-in catalogue is not canonical."
    );
}

#[test]
fn source_filter_uses_fallback_identifiers_and_deduplicates_efforts() {
    let mut source = serde_json::Map::new();
    for kind in ProviderKind::ALL {
        source.insert(
            models_dev_id(kind).to_owned(),
            serde_json::json!({
                "models": {
                    kind.default_model(): {
                        "attachment": true,
                        "reasoning": true,
                        "reasoning_options": [{
                            "type": "effort",
                            "values": [null, "default", "high", "high"]
                        }],
                        "tool_call": true,
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "limit": {"context": 128000, "input": 120000, "output": 8000}
                    }
                }
            }),
        );
    }

    let openai_models = source
        .get_mut(models_dev_id(ProviderKind::OpenaiCodex))
        .and_then(|provider| provider.get_mut("models"))
        .and_then(serde_json::Value::as_object_mut)
        .expect("OpenAI models");
    for (id, tool_call, input, output, status) in [
        (
            "audio-output",
            true,
            serde_json::json!(["text"]),
            serde_json::json!(["text", "audio"]),
            serde_json::Value::Null,
        ),
        (
            "no-text-input",
            true,
            serde_json::json!(["image"]),
            serde_json::json!(["text"]),
            serde_json::Value::Null,
        ),
        (
            "no-tools",
            false,
            serde_json::json!(["text"]),
            serde_json::json!(["text"]),
            serde_json::Value::Null,
        ),
        (
            "deprecated",
            true,
            serde_json::json!(["text"]),
            serde_json::json!(["text"]),
            serde_json::json!("deprecated"),
        ),
    ] {
        openai_models.insert(
            id.to_owned(),
            serde_json::json!({
                "attachment": false,
                "tool_call": tool_call,
                "modalities": {"input": input, "output": output},
                "status": status,
                "limit": {"context": 64000, "output": 4000}
            }),
        );
    }

    let snapshot = filter_source(
        &serde_json::to_vec(&source).expect("source"),
        "W/\"fixture\"",
        42,
    )
    .expect("filtered source");

    assert_eq!(snapshot.checked_at_unix_seconds, 42);
    assert!(snapshot.providers.iter().all(|provider| {
        let model = provider
            .models
            .iter()
            .find(|model| model.id == ProviderKind::parse(&provider.id).unwrap().default_model())
            .expect("default model");
        model.efforts == ["high"] && model.attachment && model.limit.context == 128_000
    }));
    let openai = snapshot
        .providers
        .iter()
        .find(|provider| provider.id == ProviderKind::OpenaiCodex.as_str())
        .expect("OpenAI provider");
    assert_eq!(openai.models.len(), 1);

    let stored = serde_json::to_value(&snapshot).expect("stored snapshot");
    let model = &stored["providers"][0]["models"][0];
    assert!(model.get("agent_compatible").is_none());
    assert_eq!(model["limit"].as_object().expect("limit").len(), 1);
}

#[test]
fn source_filter_rejects_an_explicit_empty_identifier() {
    let mut source = serde_json::Map::new();
    for kind in ProviderKind::ALL {
        source.insert(
            models_dev_id(kind).to_owned(),
            serde_json::json!({
                "models": {
                    kind.default_model(): {
                        "id": "",
                        "attachment": false,
                        "tool_call": true,
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "limit": {"context": 128000, "output": 8000}
                    }
                }
            }),
        );
    }

    assert!(filter_source(&serde_json::to_vec(&source).expect("source"), "", 0).is_err());
}

#[test]
fn svg_validation_rejects_active_and_external_content() {
    assert!(
        validate_svg(br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>"#).is_ok()
    );
    assert!(validate_svg(br#"<svg><script /></svg>"#).is_err());
    assert!(validate_svg(br#"<svg><image href="&#x68;ttps://example.test/a" /></svg>"#).is_err());
    assert!(validate_svg(br#"<svg><path onload="run()" /></svg>"#).is_err());
}

#[test]
fn atomic_write_replaces_a_file_without_temporary_files() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("catalogue.json");
    fs::write(&path, b"old").expect("old file");

    atomic_write(&path, b"new").expect("atomic write");

    assert_eq!(fs::read(&path).expect("new file"), b"new");
    assert_eq!(fs::read_dir(directory.path()).expect("entries").count(), 1);
}
