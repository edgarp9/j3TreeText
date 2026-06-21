use super::*;

const REPLACE_ALL_TEST_OUTPUT_LIMIT: usize = 1024;

#[test]
fn third_party_license_about_summary_matches_notice_inventory() {
    let notices = include_str!("../../THIRD_PARTY_NOTICES.txt");
    let mut in_summary = false;
    let mut package_count = 0usize;
    let mut license_expressions = Vec::new();

    for line in notices.lines() {
        if line == "License Summary" {
            in_summary = true;
            continue;
        }
        if in_summary && line == "Embedded Resource Inventory" {
            break;
        }

        if !in_summary {
            continue;
        }
        if let Some(license) = line.strip_prefix("License expression: ") {
            license_expressions.push(license.to_owned());
        } else if let Some(count) = line.strip_prefix("Count: ") {
            package_count += count
                .parse::<usize>()
                .unwrap_or_else(|error| panic!("invalid license summary count `{line}`: {error}"));
        }
    }

    let expected_license_expressions = APP_THIRD_PARTY_LICENSE_EXPRESSIONS
        .iter()
        .map(|license| (*license).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(package_count, APP_THIRD_PARTY_PACKAGE_COUNT);
    assert_eq!(license_expressions, expected_license_expressions);
}

fn all_ui_setting_keys() -> [&'static str; 16] {
    [
        SETTING_WINDOW_X,
        SETTING_WINDOW_Y,
        SETTING_WINDOW_WIDTH,
        SETTING_WINDOW_HEIGHT,
        SETTING_SPLITTER_LEFT_WIDTH,
        SETTING_SELECTION_NODE_ID,
        SETTING_EDITOR_FONT_FAMILY,
        SETTING_EDITOR_FONT_SIZE_PT,
        SETTING_EDITOR_WORD_WRAP,
        SETTING_TEXT_IMPORT_ENCODING,
        SETTING_TEXT_EXPORT_ENCODING,
        SETTING_APPEARANCE_THEME,
        SETTING_APPEARANCE_DARK_THEME,
        SETTING_UI_LANGUAGE,
        SETTING_AUTO_SAVE_ENABLED,
        SETTING_AUTO_SAVE_INTERVAL_SECONDS,
    ]
}

#[test]
fn initial_document_has_root_and_default_document() -> Result<(), DomainError> {
    let root = Node::root_document(
        "2026-04-30T00:00:00Z".to_owned(),
        "2026-04-30T00:00:00Z".to_owned(),
    );
    let document = Node::default_document(
        "2026-04-30T00:00:00Z".to_owned(),
        "2026-04-30T00:00:00Z".to_owned(),
    );

    let model = Document::new(vec![root, document])?;

    assert_eq!(model.node_count(), 2);
    assert!(matches!(
        model.root(),
        Some(Node {
            id: ROOT_NODE_ID,
            ..
        })
    ));
    assert_eq!(model.root().map(|node| node.content.as_str()), Some(""));
    Ok(())
}

#[test]
fn document_rejects_parent_cycle_disconnected_from_root() {
    let root = Node::root_document(
        "2026-04-30T00:00:00Z".to_owned(),
        "2026-04-30T00:00:00Z".to_owned(),
    );
    let parent = Node {
        id: 10,
        parent_id: Some(11),
        title: "Parent".to_owned(),
        sort_order: 0,
        content: String::new(),
        created_at: "2026-04-30T00:00:00Z".to_owned(),
        updated_at: "2026-04-30T00:00:00Z".to_owned(),
        deleted_at: None,
    };
    let document = Node {
        id: 11,
        parent_id: Some(10),
        title: "Document".to_owned(),
        sort_order: 0,
        content: String::new(),
        created_at: "2026-04-30T00:00:00Z".to_owned(),
        updated_at: "2026-04-30T00:00:00Z".to_owned(),
        deleted_at: None,
    };

    assert!(matches!(
        Document::new(vec![root, parent, document]),
        Err(DomainError::ParentCycle { node_id: 10 })
    ));
}

#[test]
fn document_applies_incremental_node_changes() -> Result<(), DomainError> {
    let root = Node::root_document(test_timestamp(), test_timestamp());
    let first = test_node(10, Some(ROOT_NODE_ID), "First", 0, "one");
    let second = test_node(11, Some(ROOT_NODE_ID), "Second", 1, "two");
    let mut document = Document::new(vec![root, first, second])?;

    document.insert_node(test_node(12, Some(ROOT_NODE_ID), "Draft", 2, ""))?;
    document.rename_node(12, "Renamed".to_owned(), "2026-04-30T00:00:01Z".to_owned())?;
    document.replace_node_content(12, "updated".to_owned(), "2026-04-30T00:00:02Z".to_owned())?;
    document.apply_sibling_order_updates(&[
        sibling_order_update(12, 0, "2026-04-30T00:00:03Z"),
        sibling_order_update(10, 1, "2026-04-30T00:00:03Z"),
        sibling_order_update(11, 2, "2026-04-30T00:00:03Z"),
    ])?;

    let renamed = document
        .node_by_id(12)
        .ok_or(DomainError::NodeNotFound { node_id: 12 })?;
    assert_eq!(renamed.title, "Renamed");
    assert_eq!(renamed.content, "updated");
    assert_eq!(renamed.sort_order, 0);

    document.remove_nodes_and_apply_sibling_order_updates(
        &[12],
        &[
            sibling_order_update(10, 0, "2026-04-30T00:00:04Z"),
            sibling_order_update(11, 1, "2026-04-30T00:00:04Z"),
        ],
    )?;

    assert!(document.node_by_id(12).is_none());
    assert_eq!(document.node_by_id(10).map(|node| node.sort_order), Some(0));
    assert_eq!(document.node_by_id(11).map(|node| node.sort_order), Some(1));
    Ok(())
}

#[test]
fn document_applies_many_sibling_order_updates() -> Result<(), DomainError> {
    let mut nodes = vec![Node::root_document(test_timestamp(), test_timestamp())];
    for offset in 0_i64..64 {
        nodes.push(test_node(
            10 + offset,
            Some(ROOT_NODE_ID),
            &format!("Node {offset:02}"),
            offset,
            "",
        ));
    }
    let mut document = Document::new(nodes)?;

    let updates: Vec<NodeSiblingOrderUpdate> = (0_i64..64)
        .map(|offset| NodeSiblingOrderUpdate {
            node_id: 10 + offset,
            parent_id: Some(ROOT_NODE_ID),
            sort_order: 63 - offset,
            updated_at: "2026-04-30T00:00:05Z".to_owned(),
        })
        .collect();

    document.apply_sibling_order_updates(&updates)?;

    for offset in 0_i64..64 {
        let node = document
            .node_by_id(10 + offset)
            .ok_or(DomainError::NodeNotFound {
                node_id: 10 + offset,
            })?;
        assert_eq!(node.parent_id, Some(ROOT_NODE_ID));
        assert_eq!(node.sort_order, 63 - offset);
        assert_eq!(node.updated_at, "2026-04-30T00:00:05Z");
    }
    Ok(())
}

#[test]
fn sibling_order_updates_reject_missing_parent_after_large_batch() -> Result<(), DomainError> {
    let mut nodes = vec![Node::root_document(test_timestamp(), test_timestamp())];
    for offset in 0_i64..64 {
        nodes.push(test_node(
            10 + offset,
            Some(ROOT_NODE_ID),
            &format!("Node {offset:02}"),
            offset,
            "",
        ));
    }
    let mut document = Document::new(nodes)?;

    let updates: Vec<NodeSiblingOrderUpdate> = (0_i64..64)
        .map(|offset| NodeSiblingOrderUpdate {
            node_id: 10 + offset,
            parent_id: if offset == 63 {
                Some(999)
            } else {
                Some(ROOT_NODE_ID)
            },
            sort_order: offset,
            updated_at: "2026-04-30T00:00:05Z".to_owned(),
        })
        .collect();

    assert!(matches!(
        document.apply_sibling_order_updates(&updates),
        Err(DomainError::MissingParent {
            node_id: 73,
            parent_id: 999
        })
    ));
    Ok(())
}

#[test]
fn sibling_order_parent_updates_keep_title_and_root_validation() -> Result<(), DomainError> {
    let root = Node::root_document(test_timestamp(), test_timestamp());
    let first_parent = test_node(10, Some(ROOT_NODE_ID), "First parent", 0, "");
    let second_parent = test_node(11, Some(ROOT_NODE_ID), "Second parent", 1, "");
    let first_child = test_node(20, Some(10), "Shared", 0, "");
    let second_child = test_node(21, Some(11), "Shared", 0, "");
    let unique_child = test_node(22, Some(10), "Unique", 1, "");
    let mut document = Document::new(vec![
        root,
        first_parent,
        second_parent,
        first_child,
        second_child,
        unique_child,
    ])?;

    assert!(matches!(
        document.apply_sibling_order_updates(&[NodeSiblingOrderUpdate {
            node_id: 20,
            parent_id: Some(11),
            sort_order: 1,
            updated_at: "2026-04-30T00:00:05Z".to_owned(),
        }]),
        Err(DomainError::DuplicateSiblingTitle {
            parent_id: Some(11),
            title
        }) if title == "Shared"
    ));

    assert!(matches!(
        document.apply_sibling_order_updates(&[NodeSiblingOrderUpdate {
            node_id: 22,
            parent_id: None,
            sort_order: 1,
            updated_at: "2026-04-30T00:00:05Z".to_owned(),
        }]),
        Err(DomainError::MultipleRoots)
    ));
    Ok(())
}

#[test]
fn node_rejects_embedded_nul_in_title_or_content() {
    let mut node = Node::default_document(
        "2026-04-30T00:00:00Z".to_owned(),
        "2026-04-30T00:00:00Z".to_owned(),
    );

    node.title = "Bad\0Title".to_owned();
    assert!(matches!(
        node.validate(),
        Err(DomainError::EmbeddedNulTitle {
            node_id: DEFAULT_DOCUMENT_ID
        })
    ));

    node.title = "Title".to_owned();
    node.content = "Bad\0Content".to_owned();
    assert!(matches!(
        node.validate(),
        Err(DomainError::EmbeddedNulContent {
            node_id: DEFAULT_DOCUMENT_ID
        })
    ));
}

#[test]
fn ui_settings_fall_back_when_values_are_missing_or_invalid() {
    let settings = UiSettings::from_entries([
        (SETTING_WINDOW_X, "not-a-number"),
        (SETTING_WINDOW_Y, "42"),
        (SETTING_WINDOW_WIDTH, "12"),
        (SETTING_WINDOW_HEIGHT, "700"),
        (SETTING_SPLITTER_LEFT_WIDTH, "bad"),
        (SETTING_SELECTION_NODE_ID, "-1"),
    ]);

    assert_eq!(settings.window.x, None);
    assert_eq!(settings.window.y, Some(42));
    assert_eq!(settings.window.width, DEFAULT_WINDOW_WIDTH);
    assert_eq!(settings.window.height, 700);
    assert_eq!(settings.splitter.left_width, DEFAULT_SPLITTER_LEFT_WIDTH);
    assert_eq!(settings.selection.node_id, None);
    assert!(settings.editor.word_wrap);
    assert_eq!(settings.language, UiLanguage::English);
    assert_eq!(settings.auto_save, AutoSaveSettings::default());
}

#[test]
fn default_editor_font_settings_are_created() {
    let settings = UiSettings::default();

    assert_eq!(settings.editor_font.family, DEFAULT_EDITOR_FONT_FAMILY);
    assert_eq!(settings.editor_font.size_pt, DEFAULT_EDITOR_FONT_SIZE_PT);
}

#[test]
fn editor_word_wrap_defaults_to_enabled_when_missing() {
    let settings = UiSettings::from_entries(std::iter::empty::<(&str, &str)>());

    assert!(settings.editor.word_wrap);
}

#[test]
fn editor_word_wrap_storage_values_round_trip() {
    for word_wrap in [true, false] {
        let settings = UiSettings {
            editor: EditorSettings { word_wrap },
            ..UiSettings::default()
        };
        let entries = settings.entries();
        let loaded =
            UiSettings::from_entries(entries.iter().map(|(key, value)| (*key, value.as_str())));

        assert_eq!(loaded.editor.word_wrap, word_wrap);
        assert_eq!(
            editor_word_wrap_storage_value(loaded.editor.word_wrap),
            editor_word_wrap_storage_value(word_wrap)
        );
    }
}

#[test]
fn editor_word_wrap_invalid_storage_falls_back_to_enabled() {
    let settings = UiSettings::from_entries([(SETTING_EDITOR_WORD_WRAP, "not-a-bool")]);

    assert!(settings.editor.word_wrap);
}

#[test]
fn ui_settings_entries_include_editor_word_wrap() {
    let settings = UiSettings::default();
    let entries = settings.entries();

    assert!(entries
        .iter()
        .any(|(key, value)| *key == SETTING_EDITOR_WORD_WRAP && value == "true"));
}

#[test]
fn ui_settings_changed_entries_include_only_modified_values() {
    let previous = UiSettings {
        window: WindowSettings::new(100, 120, 900, 600),
        splitter: SplitterSettings::new(280),
        selection: SelectionSettings {
            node_id: Some(DEFAULT_DOCUMENT_ID),
        },
        appearance: AppearanceSettings {
            theme: AppearanceTheme::Light,
        },
        ..UiSettings::default()
    };
    let next = UiSettings {
        splitter: SplitterSettings::new(360),
        selection: SelectionSettings { node_id: None },
        appearance: AppearanceSettings {
            theme: AppearanceTheme::Forest,
        },
        ..previous.clone()
    };

    assert!(previous.changed_entries(&previous).is_empty());

    let entries = next.changed_entries(&previous);
    let keys = entries.iter().map(|(key, _)| *key).collect::<Vec<_>>();

    assert_eq!(
        keys,
        vec![
            SETTING_SPLITTER_LEFT_WIDTH,
            SETTING_SELECTION_NODE_ID,
            SETTING_APPEARANCE_THEME,
            SETTING_APPEARANCE_DARK_THEME,
        ]
    );
    assert!(entries
        .iter()
        .any(|(key, value)| *key == SETTING_SELECTION_NODE_ID && value == "0"));
    assert!(entries
        .iter()
        .any(|(key, value)| *key == SETTING_APPEARANCE_THEME && value == "forest"));
}

#[test]
fn ui_settings_entries_and_changed_entries_cover_every_persisted_key() {
    let previous = UiSettings::default();
    let next = UiSettings {
        window: WindowSettings::new(100, 120, 1024, 768),
        splitter: SplitterSettings::new(360),
        selection: SelectionSettings { node_id: Some(42) },
        editor_font: EditorFontSettings::new("Cascadia Mono", 14),
        editor: EditorSettings { word_wrap: false },
        text_encoding: TextEncodingSettings {
            import_encoding: TextEncoding::KoreanEucKr,
            export_encoding: TextEncoding::Utf16LeWithBom,
        },
        appearance: AppearanceSettings {
            theme: AppearanceTheme::Forest,
        },
        language: UiLanguage::Korean,
        auto_save: AutoSaveSettings::new(true, 300),
    };

    let all_keys = all_ui_setting_keys();
    let entry_keys = next
        .entries()
        .into_iter()
        .map(|(key, _)| key)
        .collect::<std::collections::BTreeSet<_>>();
    let changed_keys = next
        .changed_entries(&previous)
        .into_iter()
        .map(|(key, _)| key)
        .collect::<std::collections::BTreeSet<_>>();
    let expected_keys = all_keys
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(entry_keys, expected_keys);
    assert_eq!(changed_keys, expected_keys);
}

#[test]
fn auto_save_settings_default_to_disabled_when_missing() {
    let settings = UiSettings::from_entries(std::iter::empty::<(&str, &str)>());

    assert!(!settings.auto_save.enabled);
    assert_eq!(
        settings.auto_save.interval_seconds,
        DEFAULT_AUTO_SAVE_INTERVAL_SECONDS
    );
}

#[test]
fn auto_save_settings_storage_values_round_trip() {
    let settings = UiSettings {
        auto_save: AutoSaveSettings::new(true, 300),
        ..UiSettings::default()
    };
    let entries = settings.entries();
    let loaded =
        UiSettings::from_entries(entries.iter().map(|(key, value)| (*key, value.as_str())));

    assert_eq!(loaded.auto_save, AutoSaveSettings::new(true, 300));
    assert_eq!(
        auto_save_enabled_storage_value(loaded.auto_save.enabled),
        "true"
    );
}

#[test]
fn auto_save_settings_invalid_storage_falls_back_to_defaults() {
    let settings = UiSettings::from_entries([
        (SETTING_AUTO_SAVE_ENABLED, "yes"),
        (SETTING_AUTO_SAVE_INTERVAL_SECONDS, "5"),
    ]);

    assert_eq!(settings.auto_save, AutoSaveSettings::default());
}

#[test]
fn ui_settings_entries_include_auto_save_settings() {
    let settings = UiSettings::default();
    let entries = settings.entries();
    let default_interval = DEFAULT_AUTO_SAVE_INTERVAL_SECONDS.to_string();

    assert!(entries
        .iter()
        .any(|(key, value)| *key == SETTING_AUTO_SAVE_ENABLED && value == "false"));
    assert!(entries.iter().any(|(key, value)| {
        *key == SETTING_AUTO_SAVE_INTERVAL_SECONDS && value == &default_interval
    }));
}

#[test]
fn appearance_theme_defaults_to_light_when_missing() {
    let settings = UiSettings::from_entries(std::iter::empty::<(&str, &str)>());

    assert_eq!(settings.appearance.theme, AppearanceTheme::Light);
}

#[test]
fn appearance_theme_storage_values_round_trip() {
    for theme in AppearanceTheme::options() {
        let settings = UiSettings {
            appearance: AppearanceSettings { theme: *theme },
            ..UiSettings::default()
        };
        let entries = settings.entries();
        let loaded =
            UiSettings::from_entries(entries.iter().map(|(key, value)| (*key, value.as_str())));

        assert_eq!(loaded.appearance.theme, *theme);
        assert_eq!(
            loaded.appearance.theme.storage_value(),
            theme.storage_value()
        );
    }
}

#[test]
fn appearance_theme_invalid_storage_falls_back_to_light() {
    let settings = UiSettings::from_entries([(SETTING_APPEARANCE_THEME, "not-a-theme")]);

    assert_eq!(settings.appearance.theme, AppearanceTheme::Light);
}

#[test]
fn legacy_dark_theme_storage_is_still_read() {
    let dark = UiSettings::from_entries([(SETTING_APPEARANCE_DARK_THEME, "true")]);
    let light = UiSettings::from_entries([(SETTING_APPEARANCE_DARK_THEME, "false")]);

    assert_eq!(dark.appearance.theme, AppearanceTheme::ClassicDark);
    assert_eq!(light.appearance.theme, AppearanceTheme::Light);
}

#[test]
fn appearance_theme_storage_takes_precedence_over_legacy_dark_theme() {
    let settings = UiSettings::from_entries([
        (SETTING_APPEARANCE_THEME, "forest"),
        (SETTING_APPEARANCE_DARK_THEME, "false"),
    ]);

    assert_eq!(settings.appearance.theme, AppearanceTheme::Forest);
}

#[test]
fn ui_settings_entries_include_appearance_theme_and_legacy_dark_theme() {
    let settings = UiSettings::default();
    let entries = settings.entries();

    assert!(entries
        .iter()
        .any(|(key, value)| { *key == SETTING_APPEARANCE_THEME && value == "light" }));
    assert!(entries
        .iter()
        .any(|(key, value)| { *key == SETTING_APPEARANCE_DARK_THEME && value == "false" }));
}

#[test]
fn ui_language_storage_values_round_trip() {
    for language in UiLanguage::options() {
        let settings = UiSettings {
            language: *language,
            ..UiSettings::default()
        };
        let entries = settings.entries();
        let loaded =
            UiSettings::from_entries(entries.iter().map(|(key, value)| (*key, value.as_str())));

        assert_eq!(loaded.language, *language);
        assert_eq!(loaded.language.storage_value(), language.storage_value());
    }
}

#[test]
fn ui_language_invalid_storage_falls_back_to_english() {
    let settings = UiSettings::from_entries([(SETTING_UI_LANGUAGE, "not-a-language")]);

    assert_eq!(settings.language, UiLanguage::English);
}

#[test]
fn editor_word_wrap_toggle_switches_between_enabled_and_disabled() {
    assert!(!toggle_editor_word_wrap(true));
    assert!(toggle_editor_word_wrap(false));

    let mut settings = EditorSettings { word_wrap: true };
    settings.toggle_word_wrap();
    assert!(!settings.word_wrap);
    settings.toggle_word_wrap();
    assert!(settings.word_wrap);
}

#[test]
fn editor_font_family_falls_back_when_empty() {
    let settings = UiSettings::from_entries([
        (SETTING_EDITOR_FONT_FAMILY, "   "),
        (SETTING_EDITOR_FONT_SIZE_PT, "12"),
    ]);

    assert_eq!(settings.editor_font.family, DEFAULT_EDITOR_FONT_FAMILY);
    assert_eq!(settings.editor_font.size_pt, 12);
}

#[test]
fn editor_font_size_outside_storage_range_falls_back() {
    let too_small = UiSettings::from_entries([
        (SETTING_EDITOR_FONT_FAMILY, "Arial"),
        (SETTING_EDITOR_FONT_SIZE_PT, "5"),
    ]);
    let too_large = UiSettings::from_entries([
        (SETTING_EDITOR_FONT_FAMILY, "Arial"),
        (SETTING_EDITOR_FONT_SIZE_PT, "73"),
    ]);

    assert_eq!(too_small.editor_font.size_pt, DEFAULT_EDITOR_FONT_SIZE_PT);
    assert_eq!(too_large.editor_font.size_pt, DEFAULT_EDITOR_FONT_SIZE_PT);
}

#[test]
fn editor_font_settings_round_trip_entries() {
    let settings = UiSettings {
        editor_font: EditorFontSettings::new("Cascadia Mono", 14),
        text_encoding: TextEncodingSettings {
            import_encoding: TextEncoding::KoreanEucKr,
            export_encoding: TextEncoding::Utf16LeWithBom,
        },
        ..UiSettings::default()
    };
    let entries = settings.entries();
    let loaded =
        UiSettings::from_entries(entries.iter().map(|(key, value)| (*key, value.as_str())));

    assert_eq!(loaded.editor_font, settings.editor_font);
    assert_eq!(loaded.text_encoding, settings.text_encoding);
}

#[test]
fn text_encoding_storage_values_round_trip() {
    for encoding in TextEncoding::import_options() {
        assert_eq!(
            TextEncoding::from_import_storage_value(encoding.storage_value()),
            Some(*encoding)
        );
    }

    for encoding in TextEncoding::export_options() {
        assert_eq!(
            TextEncoding::from_export_storage_value(encoding.storage_value()),
            Some(*encoding)
        );
    }
}

#[test]
fn text_encoding_settings_fall_back_when_invalid() {
    let settings = UiSettings::from_entries([
        (SETTING_TEXT_IMPORT_ENCODING, "not-an-encoding"),
        (SETTING_TEXT_EXPORT_ENCODING, "auto"),
    ]);

    assert_eq!(
        settings.text_encoding.import_encoding,
        TextEncoding::default_import()
    );
    assert_eq!(
        settings.text_encoding.export_encoding,
        TextEncoding::default_export()
    );
}

#[test]
fn editor_font_size_invalid_string_falls_back() {
    let settings = UiSettings::from_entries([
        (SETTING_EDITOR_FONT_FAMILY, "Arial"),
        (SETTING_EDITOR_FONT_SIZE_PT, "not-a-size"),
    ]);

    assert_eq!(settings.editor_font.family, "Arial");
    assert_eq!(settings.editor_font.size_pt, DEFAULT_EDITOR_FONT_SIZE_PT);
}

#[test]
fn find_next_literal_returns_byte_range_for_utf8_text() {
    assert_eq!(
        find_next_literal("가나다 가나다", "나다", 0),
        Some(TextMatch { start: 3, end: 9 })
    );
    assert_eq!(
        find_next_literal("가나다 가나다", "나다", 9),
        Some(TextMatch { start: 13, end: 19 })
    );
    assert_eq!(find_next_literal("가나다", "", 0), None);
}

#[test]
fn find_next_literal_handles_ascii_crlf_emoji_empty_and_long_text() {
    assert_eq!(
        find_next_literal("ASCII\r\n🚀 한글\r\nASCII", "🚀 한글", 0),
        Some(TextMatch { start: 7, end: 18 })
    );
    assert_eq!(find_next_literal("", "x", 0), None);

    let prefix = "a".repeat(20_000);
    let content = format!("{prefix}\r\nneedle🚀");
    assert_eq!(
        find_next_literal(&content, "needle🚀", 0),
        Some(TextMatch {
            start: 20_002,
            end: 20_012,
        })
    );
}

#[test]
fn replace_literal_at_replaces_only_matching_position() {
    let replaced = match replace_literal_at("alpha beta alpha", "alpha", "one", 0) {
        Some(value) => value,
        None => panic!("literal should be replaced at the requested position"),
    };

    assert_eq!(replaced.content, "one beta alpha");
    assert_eq!(replaced.replaced, TextMatch { start: 0, end: 5 });
    assert!(replace_literal_at("alpha beta", "alpha", "one", 1).is_none());
}

#[test]
fn replace_literal_at_preserves_utf8_ranges_for_crlf_and_emoji() {
    let content = "ASCII\r\n🚀 한글\r\n끝";
    let start = content.find("🚀 한글").expect("needle should exist");
    let replaced = match replace_literal_at(content, "🚀 한글", "plain", start) {
        Some(value) => value,
        None => panic!("literal should be replaced at the requested position"),
    };

    assert_eq!(replaced.content, "ASCII\r\nplain\r\n끝");
    assert_eq!(
        replaced.replaced,
        TextMatch {
            start,
            end: start + 11
        }
    );
}

#[test]
fn replace_all_literal_counts_non_overlapping_replacements() {
    assert_eq!(
        replace_all_literal("aaaa", "aa", "b", REPLACE_ALL_TEST_OUTPUT_LIMIT),
        Ok(ReplaceAllResult {
            content: "bb".to_owned().into(),
            count: 2,
        })
    );
    assert_eq!(
        replace_all_literal("가나 가나", "가나", "다", REPLACE_ALL_TEST_OUTPUT_LIMIT),
        Ok(ReplaceAllResult {
            content: "다 다".to_owned().into(),
            count: 2,
        })
    );
    assert_eq!(
        replace_all_literal("same", "", "x", REPLACE_ALL_TEST_OUTPUT_LIMIT),
        Ok(ReplaceAllResult {
            content: "same".to_owned().into(),
            count: 0,
        })
    );
}

#[test]
fn replace_all_literal_handles_empty_and_very_long_text() {
    assert_eq!(
        replace_all_literal("", "x", "y", REPLACE_ALL_TEST_OUTPUT_LIMIT),
        Ok(ReplaceAllResult {
            content: String::new().into(),
            count: 0,
        })
    );

    let content = format!("{}needle\r\nneedle", "a".repeat(400));
    let result = replace_all_literal(&content, "needle", "🚀", 2_000)
        .expect("long replace result should stay under the limit");

    assert_eq!(result.count, 2);
    assert_eq!(result.content, format!("{}🚀\r\n🚀", "a".repeat(400)));
}

#[test]
fn replace_all_literal_rejects_result_larger_than_limit() {
    assert_eq!(
        replace_all_literal("aaaa", "a", "xx", 5),
        Err(ReplaceAllError::OutputTooLarge { limit: 5 })
    );
}

#[test]
fn replace_all_literal_rejects_trailing_result_larger_than_limit() {
    assert_eq!(
        replace_all_literal("x tail", "x", "", 4),
        Err(ReplaceAllError::OutputTooLarge { limit: 4 })
    );
}

#[test]
fn opening_document_creates_tab() {
    let mut tabs = OpenTabs::new();

    assert_eq!(
        tabs.open_or_activate(tab_input(10, "Draft", "content", true)),
        OpenTabResult::Opened { index: 0 }
    );

    assert_eq!(tabs.tabs().len(), 1);
    assert_eq!(tabs.active().map(|tab| tab.node_id), Some(10));
}

#[test]
fn opening_same_document_reuses_existing_tab() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "Draft", "content", true));
    let result = tabs.open_or_activate(tab_input(10, "Draft renamed", "fresh", true));

    assert_eq!(result, OpenTabResult::ActivatedExisting { index: 0 });
    assert_eq!(tabs.tabs().len(), 1);
    assert_eq!(
        tabs.active().map(|tab| tab.title.as_str()),
        Some("Draft renamed")
    );
    assert_eq!(tabs.active().map(|tab| tab.content.as_str()), Some("fresh"));
}

#[test]
fn dirty_state_is_independent_per_tab() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "First", "one", true));
    tabs.update_active_content("one changed".to_owned());
    tabs.open_or_activate(tab_input(11, "Second", "two", true));

    assert!(tabs.tabs()[0].dirty);
    assert!(!tabs.tabs()[1].dirty);

    tabs.update_active_content("two changed".to_owned());

    assert!(tabs.tabs()[0].dirty);
    assert!(tabs.tabs()[1].dirty);
}

#[test]
fn user_edit_and_save_flow_marks_and_clears_dirty_state() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "Draft", "loaded", true));

    assert!(tabs.mark_active_dirty_from_view());
    assert!(tabs.active().is_some_and(|tab| tab.dirty));
    assert!(tabs.update_active_content("loaded\r\n한글 🚀".to_owned()));

    tabs.mark_active_saved(
        "loaded\r\n한글 🚀".to_owned(),
        "2026-05-21T00:00:00Z".to_owned(),
    );

    let tab = match tabs.active() {
        Some(tab) => tab,
        None => panic!("active tab should exist"),
    };
    assert_eq!(tab.content, "loaded\r\n한글 🚀");
    assert_eq!(tab.loaded_content(), "loaded\r\n한글 🚀");
    assert_eq!(tab.loaded_updated_at, "2026-05-21T00:00:00Z");
    assert!(!tab.dirty);
    assert!(!tab.is_save_target());
}

#[test]
fn programmatic_loaded_tab_sync_does_not_mark_clean_tab_dirty() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "Draft", "loaded", true));

    let mut refreshed = tab_input(10, "Draft renamed", "programmatic\r\n한글 🚀", true);
    refreshed.loaded_updated_at = "2026-05-21T00:00:00Z".to_owned();
    tabs.sync_loaded_tab(refreshed, true);

    let tab = match tabs.active() {
        Some(tab) => tab,
        None => panic!("active tab should exist"),
    };
    assert_eq!(tab.title, "Draft renamed");
    assert_eq!(tab.content, "programmatic\r\n한글 🚀");
    assert_eq!(tab.loaded_content(), "programmatic\r\n한글 🚀");
    assert_eq!(tab.loaded_updated_at, "2026-05-21T00:00:00Z");
    assert!(!tab.dirty);
    assert!(!tab.is_save_target());
}

#[test]
fn replacing_active_tab_content_updates_body_and_dirty_state() {
    let mut tabs = OpenTabs::new();
    let content = "첫줄\r\n한글 🚀\r\n끝";

    tabs.open_or_activate(tab_input(10, "Draft", content, true));
    let found = match find_next_literal(content, "한글 🚀", 0) {
        Some(found) => found,
        None => panic!("replace target should exist"),
    };
    let replaced = match replace_literal_at(content, "한글 🚀", "plain", found.start) {
        Some(result) => result,
        None => panic!("replace target should be replaced"),
    };

    assert!(tabs.update_active_content(replaced.content));

    let tab = match tabs.active() {
        Some(tab) => tab,
        None => panic!("active tab should exist"),
    };
    assert_eq!(tab.content, "첫줄\r\nplain\r\n끝");
    assert_eq!(tab.loaded_content(), content);
    assert!(tab.dirty);
    assert!(tab.is_save_target());
}

#[test]
fn active_content_update_can_reuse_caller_buffer() {
    let mut tabs = OpenTabs::new();
    let mut content = "changed".to_owned();

    tabs.open_or_activate(tab_input(10, "Draft", "content", true));

    assert!(tabs.update_active_content_reusing(&mut content));
    assert_eq!(
        tabs.active().map(|tab| tab.content.as_str()),
        Some("changed")
    );
    assert!(content.is_empty());
    assert!(tabs.active().is_some_and(|tab| tab.dirty));
}

#[test]
fn active_tab_can_mark_dirty_from_view_without_replacing_content() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "Draft", "content", true));

    assert!(tabs.mark_active_dirty_from_view());
    assert!(!tabs.mark_active_dirty_from_view());

    let tab = match tabs.active() {
        Some(tab) => tab,
        None => panic!("active tab should exist"),
    };
    assert_eq!(tab.content, "content");
    assert!(tab.dirty);
}

#[test]
fn dirty_tab_sync_updates_save_conflict_token_when_refreshed_content_is_unchanged() {
    let mut tabs = OpenTabs::new();
    let mut initial = tab_input(10, "Draft", "content", true);
    initial.loaded_updated_at = "initial-token".to_owned();

    tabs.open_or_activate(initial);
    tabs.update_active_content("local draft".to_owned());

    let mut refreshed = tab_input(10, "Draft renamed", "content", true);
    refreshed.loaded_updated_at = "metadata-token".to_owned();
    tabs.sync_loaded_tab(refreshed, true);

    let tab = match tabs.active() {
        Some(tab) => tab,
        None => panic!("active tab should exist"),
    };
    assert_eq!(tab.title, "Draft renamed");
    assert_eq!(tab.content, "local draft");
    assert_eq!(tab.loaded_content(), "content");
    assert_eq!(tab.loaded_updated_at, "metadata-token");
    assert!(tab.dirty);
}

#[test]
fn dirty_tab_sync_preserves_save_conflict_token_when_refreshed_content_changed() {
    let mut tabs = OpenTabs::new();
    let mut initial = tab_input(10, "Draft", "content", true);
    initial.loaded_updated_at = "initial-token".to_owned();

    tabs.open_or_activate(initial);
    tabs.update_active_content("local draft".to_owned());

    let mut refreshed = tab_input(10, "Draft renamed", "external content", true);
    refreshed.loaded_updated_at = "metadata-token".to_owned();
    tabs.sync_loaded_tab(refreshed, true);

    let tab = match tabs.active() {
        Some(tab) => tab,
        None => panic!("active tab should exist"),
    };
    assert_eq!(tab.title, "Draft renamed");
    assert_eq!(tab.content, "local draft");
    assert_eq!(tab.loaded_content(), "content");
    assert_eq!(tab.loaded_updated_at, "initial-token");
    assert!(tab.dirty);
}

#[test]
fn dirty_tab_sync_preserves_save_conflict_token_when_not_requested() {
    let mut tabs = OpenTabs::new();
    let mut initial = tab_input(10, "Draft", "content", true);
    initial.loaded_updated_at = "initial-token".to_owned();

    tabs.open_or_activate(initial);
    tabs.update_active_content("local draft".to_owned());

    let mut refreshed = tab_input(10, "Draft renamed", "external content", true);
    refreshed.loaded_updated_at = "external-token".to_owned();
    tabs.sync_loaded_tab(refreshed, false);

    let tab = match tabs.active() {
        Some(tab) => tab,
        None => panic!("active tab should exist"),
    };
    assert_eq!(tab.title, "Draft renamed");
    assert_eq!(tab.content, "local draft");
    assert_eq!(tab.loaded_content(), "content");
    assert_eq!(tab.loaded_updated_at, "initial-token");
    assert!(tab.dirty);
}

#[test]
fn read_only_tab_does_not_become_dirty_or_save_target() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "Deleted", "content", false));
    tabs.mark_active_dirty_from_view();
    tabs.update_active_content("changed".to_owned());
    tabs.import_active_content("imported".to_owned());

    let tab = match tabs.active() {
        Some(tab) => tab,
        None => panic!("active tab should exist"),
    };
    assert_eq!(tab.content, "content");
    assert!(!tab.dirty);
    assert!(!tab.is_save_target());
}

#[test]
fn missing_active_document_tab_becomes_read_only_without_losing_local_content() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "Externally deleted", "content", true));
    tabs.update_active_content("local draft".to_owned());

    assert!(tabs.mark_tabs_missing_from_active_document_read_only(&[11]));

    let tab = match tabs.active() {
        Some(tab) => tab,
        None => panic!("active tab should exist"),
    };
    assert_eq!(tab.content, "local draft");
    assert!(tab.dirty);
    assert!(!tab.editable);
    assert!(!tab.is_save_target());
    assert!(tab.has_unsavable_changes());
}

#[test]
fn missing_inactive_document_tab_does_not_require_active_editor_reload() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "Externally deleted", "content", true));
    tabs.open_or_activate(tab_input(11, "Current", "current", true));

    assert!(!tabs.mark_tabs_missing_from_active_document_read_only(&[11]));

    assert!(!tabs.tabs()[0].editable);
    assert_eq!(tabs.active().map(|tab| tab.node_id), Some(11));
    assert_eq!(tabs.active().map(|tab| tab.editable), Some(true));
}

#[test]
fn importing_same_content_still_marks_editable_tab_dirty() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "Draft", "content", true));
    tabs.import_active_content("content".to_owned());

    let tab = match tabs.active() {
        Some(tab) => tab,
        None => panic!("active tab should exist"),
    };
    assert_eq!(tab.content, "content");
    assert!(tab.dirty);
    assert!(tab.is_save_target());
}

#[test]
fn active_tab_title_ignores_dirty_state() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "Draft", "content", true));
    assert_eq!(
        tabs.active().map(OpenDocumentTab::display_title),
        Some("Draft".to_owned())
    );

    tabs.update_active_content("changed".to_owned());

    assert_eq!(
        tabs.active().map(OpenDocumentTab::display_title),
        Some("Draft".to_owned())
    );
}

#[test]
fn editor_view_state_is_independent_per_tab() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "First", "one", true));
    assert!(tabs.update_active_view_state(DocumentTabViewState {
        first_visible_line: 24,
        caret_position_utf16: 5,
        selection_start_utf16: 2,
        selection_end_utf16: 5,
    }));
    tabs.open_or_activate(tab_input(11, "Second", "two", true));
    assert!(tabs.update_active_view_state(DocumentTabViewState {
        first_visible_line: 3,
        caret_position_utf16: 1,
        selection_start_utf16: 1,
        selection_end_utf16: 1,
    }));

    assert_eq!(tabs.tabs()[0].view_state.first_visible_line, 24);
    assert_eq!(tabs.tabs()[0].view_state.selection_start_utf16, 2);
    assert_eq!(tabs.tabs()[0].view_state.selection_end_utf16, 5);
    assert_eq!(tabs.tabs()[0].view_state.caret_position_utf16, 5);
    assert_eq!(tabs.tabs()[1].view_state.first_visible_line, 3);
    assert_eq!(tabs.tabs()[1].view_state.selection_start_utf16, 1);
    assert_eq!(tabs.tabs()[1].view_state.selection_end_utf16, 1);
    assert_eq!(tabs.tabs()[1].view_state.caret_position_utf16, 1);
}

#[test]
fn replacing_tab_content_resets_editor_view_state() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "Draft", "content", true));
    assert!(tabs.update_active_view_state(DocumentTabViewState {
        first_visible_line: 24,
        caret_position_utf16: 6,
        selection_start_utf16: 2,
        selection_end_utf16: 6,
    }));

    tabs.import_active_content("imported".to_owned());
    assert_eq!(tabs.tabs()[0].view_state, DocumentTabViewState::default());

    assert!(tabs.update_active_view_state(DocumentTabViewState {
        first_visible_line: 12,
        caret_position_utf16: 4,
        selection_start_utf16: 0,
        selection_end_utf16: 4,
    }));
    tabs.discard_tab_changes(0);
    assert_eq!(tabs.tabs()[0].view_state, DocumentTabViewState::default());
}

#[test]
fn document_tab_view_state_clamps_scroll_selection_and_caret() {
    let view_state = DocumentTabViewState {
        first_visible_line: 24,
        caret_position_utf16: 99,
        selection_start_utf16: 3,
        selection_end_utf16: 99,
    };

    assert_eq!(
        view_state.clamped(10, 4),
        DocumentTabViewState {
            first_visible_line: 4,
            caret_position_utf16: 10,
            selection_start_utf16: 3,
            selection_end_utf16: 10,
        }
    );
}

#[test]
fn document_tab_view_state_normalizes_reversed_selection() {
    let view_state = DocumentTabViewState {
        first_visible_line: 2,
        caret_position_utf16: 30,
        selection_start_utf16: 30,
        selection_end_utf16: 5,
    };

    assert_eq!(
        view_state.clamped(20, 10),
        DocumentTabViewState {
            first_visible_line: 2,
            caret_position_utf16: 20,
            selection_start_utf16: 5,
            selection_end_utf16: 20,
        }
    );
}

#[test]
fn document_tab_view_state_keeps_caret_inside_selection_range() {
    let view_state = DocumentTabViewState {
        first_visible_line: 2,
        caret_position_utf16: 12,
        selection_start_utf16: 3,
        selection_end_utf16: 8,
    };

    assert_eq!(
        view_state.clamped(20, 10),
        DocumentTabViewState {
            first_visible_line: 2,
            caret_position_utf16: 8,
            selection_start_utf16: 3,
            selection_end_utf16: 8,
        }
    );
}

#[test]
fn document_tab_view_state_uses_caret_for_collapsed_selection() {
    let view_state = DocumentTabViewState {
        first_visible_line: 2,
        caret_position_utf16: 7,
        selection_start_utf16: 0,
        selection_end_utf16: 0,
    };

    assert_eq!(
        view_state.clamped(20, 10),
        DocumentTabViewState {
            first_visible_line: 2,
            caret_position_utf16: 7,
            selection_start_utf16: 7,
            selection_end_utf16: 7,
        }
    );
}

#[test]
fn moving_active_tab_preserves_active_document() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "First", "one", true));
    tabs.open_or_activate(tab_input(11, "Second", "two", true));
    tabs.open_or_activate(tab_input(12, "Third", "three", true));

    assert!(tabs.move_tab(2, 0));

    let ordered_node_ids: Vec<i64> = tabs.tabs().iter().map(|tab| tab.node_id).collect();
    assert_eq!(ordered_node_ids, vec![12, 10, 11]);
    assert_eq!(tabs.active_index(), Some(0));
    assert_eq!(tabs.active().map(|tab| tab.node_id), Some(12));
}

#[test]
fn moving_inactive_tab_keeps_same_active_document() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "First", "one", true));
    tabs.open_or_activate(tab_input(11, "Second", "two", true));
    tabs.open_or_activate(tab_input(12, "Third", "three", true));
    assert!(tabs.set_active(1));

    assert!(tabs.move_tab(0, 2));

    let ordered_node_ids: Vec<i64> = tabs.tabs().iter().map(|tab| tab.node_id).collect();
    assert_eq!(ordered_node_ids, vec![11, 12, 10]);
    assert_eq!(tabs.active_index(), Some(0));
    assert_eq!(tabs.active().map(|tab| tab.node_id), Some(11));
}

#[test]
fn moving_tab_rejects_out_of_range_indices() {
    let mut tabs = OpenTabs::new();

    tabs.open_or_activate(tab_input(10, "First", "one", true));
    tabs.open_or_activate(tab_input(11, "Second", "two", true));

    assert!(!tabs.move_tab(0, 0));
    assert!(!tabs.move_tab(2, 0));
    assert!(!tabs.move_tab(0, 2));

    let ordered_node_ids: Vec<i64> = tabs.tabs().iter().map(|tab| tab.node_id).collect();
    assert_eq!(ordered_node_ids, vec![10, 11]);
    assert_eq!(tabs.active().map(|tab| tab.node_id), Some(11));
}

fn tab_input(node_id: i64, title: &str, content: &str, editable: bool) -> OpenDocumentTabInput {
    OpenDocumentTabInput {
        node_id,
        parent_id: Some(ROOT_NODE_ID),
        title: title.to_owned(),
        content: content.to_owned(),
        loaded_updated_at: "2026-04-30T00:00:00Z".to_owned(),
        editable,
        source: if editable {
            DocumentTabSource::ActiveTree
        } else {
            DocumentTabSource::Trash
        },
    }
}

fn test_timestamp() -> String {
    "2026-04-30T00:00:00Z".to_owned()
}

fn test_node(id: i64, parent_id: Option<i64>, title: &str, sort_order: i64, content: &str) -> Node {
    Node {
        id,
        parent_id,
        title: title.to_owned(),
        sort_order,
        content: content.to_owned(),
        created_at: test_timestamp(),
        updated_at: test_timestamp(),
        deleted_at: None,
    }
}

fn sibling_order_update(node_id: i64, sort_order: i64, updated_at: &str) -> NodeSiblingOrderUpdate {
    NodeSiblingOrderUpdate {
        node_id,
        parent_id: Some(ROOT_NODE_ID),
        sort_order,
        updated_at: updated_at.to_owned(),
    }
}
