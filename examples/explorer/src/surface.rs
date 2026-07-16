//! Deterministic source-to-JSON extraction for blind structural review.

use crate::view::{self, Scene};
use rinka::{ApplicationSpec, Element, MenuEntry, Props, WindowKind};
use std::fmt::Write;

/// Extracts all required states without interpreting or rewriting labels.
pub fn extract_all_scenes() -> String {
    let mut output = String::from("{\n  \"scenes\": [\n");
    for (index, scene) in Scene::all().into_iter().enumerate() {
        if index > 0 {
            output.push_str(",\n");
        }
        let application = view::application(scene);
        write!(
            &mut output,
            "    {{\"id\":\"{}\",\"application\":",
            scene.id()
        )
        .unwrap();
        write_application(&mut output, &application);
        output.push('}');
    }
    output.push_str("\n  ]\n}\n");
    output
}

fn write_application(output: &mut String, application: &ApplicationSpec) {
    write!(
        output,
        "{{\"id\":{},\"name\":{},\"windows\":[",
        json(&application.id),
        json(&application.name)
    )
    .unwrap();
    for (index, window) in application.windows.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        let kind = match window.kind {
            WindowKind::Main => "main",
            WindowKind::Preferences => "preferences",
            WindowKind::Panel(_) => "panel",
        };
        write!(
            output,
            "{{\"id\":{},\"title\":{},\"kind\":{},\"toolbar\":[",
            json(window.id.as_str()),
            json(&window.title),
            json(kind)
        )
        .unwrap();
        for (item_index, item) in window.toolbar.iter().enumerate() {
            if item_index > 0 {
                output.push(',');
            }
            write!(
                output,
                "{{\"id\":{},\"label\":{},\"representation\":{},\"help\":{},\"enabled\":{}}}",
                json(&item.id),
                json(&item.label),
                json(&format!("{:?}", item.kind)),
                json(&item.help),
                item.enabled
            )
            .unwrap();
        }
        output.push_str("],\"content\":");
        write_element(output, &window.content.snapshot());
        output.push('}');
    }
    output.push_str("]}");
}

fn write_element(output: &mut String, element: &Element) {
    write!(
        output,
        "{{\"kind\":{},\"key\":{},\"props\":",
        json(&format!("{:?}", element.kind())),
        element
            .key()
            .map_or_else(|| "null".to_owned(), |key| json(key.as_str()))
    )
    .unwrap();
    write_props(output, element.props());
    if !element.accelerator_table().is_empty() {
        output.push_str(",\"accelerators\":[");
        for (index, entry) in element.accelerator_table().iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            let description = entry.description();
            write!(
                output,
                "{{\"id\":{},\"chord\":{},\"scope\":{},\"enabled\":{},\"global\":{}}}",
                json(&description.id),
                json(&description.chord.to_string()),
                json(&format!("{:?}", description.scope)),
                description.enabled,
                description.global
            )
            .unwrap();
        }
        output.push(']');
    }
    output.push_str(",\"contextMenu\":");
    match element.context_menu_model() {
        Some(menu) => write_menu_entries(output, &menu.entries),
        None => output.push_str("null"),
    }
    output.push_str(",\"children\":[");
    for (index, child) in element.children().iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        write_element(output, child);
    }
    output.push_str("]}");
}

fn write_menu_entries(output: &mut String, entries: &[MenuEntry]) {
    output.push('[');
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        match entry {
            MenuEntry::Separator => output.push_str("{\"entry\":\"separator\"}"),
            MenuEntry::Item(item) => write!(
                output,
                "{{\"entry\":\"item\",\"id\":{},\"label\":{},\"help\":{},\"enabled\":{},\"checked\":{},\"role\":{},\"symbol\":{}}}",
                json(&item.id),
                json(&item.label),
                json(&item.help),
                item.enabled,
                item.checked,
                json(&format!("{:?}", item.role)),
                item.symbol
                    .map_or_else(|| "null".to_owned(), |value| json(&format!("{value:?}"))),
            )
            .unwrap(),
            MenuEntry::Submenu(submenu) => {
                write!(
                    output,
                    "{{\"entry\":\"submenu\",\"id\":{},\"label\":{},\"enabled\":{},\"entries\":",
                    json(&submenu.id),
                    json(&submenu.label),
                    submenu.enabled,
                )
                .unwrap();
                write_menu_entries(output, &submenu.entries);
                output.push('}');
            }
        }
    }
    output.push(']');
}

fn write_props(output: &mut String, props: &Props) {
    match props {
        Props::Label { text, role, .. } => write!(
            output,
            "{{\"text\":{},\"role\":{}}}",
            json(text),
            json(&format!("{role:?}"))
        )
        .unwrap(),
        Props::Button {
            label,
            role,
            size,
            material,
            enabled,
            tooltip,
            accessibility_label,
        } => write!(
            output,
            "{{\"label\":{},\"role\":{},\"controlSize\":{},\"material\":{},\"enabled\":{},\"tooltip\":{},\"accessibilityLabel\":{}}}",
            json(label),
            json(&format!("{role:?}")),
            json(&format!("{size:?}")),
            json(&format!("{material:?}")),
            enabled,
            tooltip.as_ref().map_or_else(|| "null".to_owned(), |value| json(value)),
            json(accessibility_label)
        )
        .unwrap(),
        Props::Input {
            value,
            placeholder,
            kind,
            enabled,
            accessibility_label,
        } => write!(
            output,
            "{{\"value\":{},\"placeholder\":{},\"inputKind\":{},\"enabled\":{},\"accessibilityLabel\":{}}}",
            json(value),
            json(placeholder),
            json(&format!("{kind:?}")),
            enabled,
            json(accessibility_label)
        )
        .unwrap(),
        Props::Toggle {
            label,
            value,
            size,
            enabled,
            accessibility_label,
        } => write!(
            output,
            "{{\"label\":{},\"value\":{},\"controlSize\":{},\"enabled\":{},\"accessibilityLabel\":{}}}",
            json(label),
            value,
            json(&format!("{size:?}")),
            enabled,
            json(accessibility_label)
        )
        .unwrap(),
        Props::Progress {
            fraction,
            accessibility_label,
        } => write!(
            output,
            "{{\"fraction\":{fraction},\"accessibilityLabel\":{}}}",
            json(accessibility_label)
        )
        .unwrap(),
        Props::ListRow {
            title,
            subtitle,
            cells,
            role,
            expanded,
            symbol,
            selected,
            disclosure,
            accessibility_label,
        } => write!(
            output,
            "{{\"title\":{},\"subtitle\":{},\"cells\":{},\"role\":{},\"expanded\":{},\"symbol\":{},\"selected\":{},\"disclosure\":{},\"accessibilityLabel\":{}}}",
            json(title),
            subtitle.as_ref().map_or_else(|| "null".to_owned(), |value| json(value)),
            json_array(cells),
            json(&format!("{role:?}")),
            expanded,
            symbol.map_or_else(|| "null".to_owned(), |value| json(&format!("{value:?}"))),
            selected,
            disclosure,
            json(accessibility_label)
        )
        .unwrap(),
        Props::Status {
            title,
            message,
            tone,
        } => write!(
            output,
            "{{\"title\":{},\"message\":{},\"tone\":{}}}",
            json(title),
            json(message),
            json(&format!("{tone:?}"))
        )
        .unwrap(),
        Props::Canvas {
            size,
            scene,
            accessibility_label,
        } => write!(
            output,
            "{{\"width\":{},\"height\":{},\"commands\":{},\"accessibilityLabel\":{}}}",
            size.width,
            size.height,
            scene.commands().len(),
            json(accessibility_label)
        )
        .unwrap(),
        Props::Image {
            content,
            scaling,
            accessibility_label,
        } => write!(
            output,
            "{{\"width\":{},\"height\":{},\"stride\":{},\"scale\":{},\"revision\":{},\"scaling\":{},\"accessibilityLabel\":{}}}",
            content.width(),
            content.height(),
            content.stride(),
            content.scale(),
            content.revision(),
            json(&format!("{scaling:?}")),
            json(accessibility_label)
        )
        .unwrap(),
        other => write!(output, "{}", json(&format!("{other:?}"))).unwrap(),
    }
}

fn json_array(values: &[String]) -> String {
    let values = values.iter().map(|value| json(value)).collect::<Vec<_>>();
    format!("[{}]", values.join(","))
}

fn json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                write!(&mut escaped, "\\u{:04x}", u32::from(character)).unwrap();
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use super::extract_all_scenes;

    #[test]
    fn extraction_contains_every_scene_and_accessible_label() {
        let output = extract_all_scenes();
        for scene in ["ready", "empty", "busy", "error", "canvas"] {
            assert!(output.contains(&format!("\"id\":\"{scene}\"")));
        }
        assert!(output.contains("accessibilityLabel"));
        assert!(output.contains("Connection Activity"));
        assert!(output.contains("Canvas test pattern"));
    }

    #[test]
    fn extraction_records_the_file_row_context_menu() {
        let output = extract_all_scenes();
        assert!(output.contains("\"entry\":\"item\",\"id\":\"rename\""));
        assert!(output.contains("\"entry\":\"submenu\",\"id\":\"open-with\""));
        assert!(output.contains("\"role\":\"Destructive\""));
        assert!(output.contains("\"entry\":\"separator\""));
    }
}
