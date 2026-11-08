//! What carries an annotation in a Java or Kotlin file, for the live
//! templates' `annotated("pkg.Annotation")` (IntelliJ's AnnotatedMacro:
//! a class by its qualified name, a method or field by its name).

use super::extract_method::{children, descendants};
use super::lang::Lang;
use super::symbols;

/// The declarations in `text` annotated with `@Simple` / `@pkg.Simple`, in
/// file order: classes qualified with the file's package.
pub fn annotated_declarations(lang: Lang, text: &str, annotation: &str) -> Vec<String> {
    let simple = annotation.rsplit('.').next().unwrap_or(annotation);
    if !text.contains(&format!("@{simple}")) && !text.contains(&format!("@{annotation}")) {
        return Vec::new();
    }
    let Some(tree) = symbols::parse(lang, text) else { return Vec::new() };
    let package = crate::domain::imports::header(text).package;
    let qualified = |name: &str| if package.is_empty() { name.to_owned() } else { format!("{package}.{name}") };
    let names_it = |n: &str| {
        let n = n.trim_start_matches('@').trim();
        let n = n.split('(').next().unwrap_or(n).trim();
        n == simple || n == annotation
    };
    let mut out = Vec::new();
    for node in descendants(tree.root_node()) {
        let is_annotation = matches!(node.kind(), "marker_annotation" | "annotation");
        if !is_annotation || !names_it(&text[node.byte_range()]) {
            continue;
        }
        // annotation → modifiers → the declaration.
        let Some(declaration) = node.parent().filter(|p| p.kind() == "modifiers").and_then(|m| m.parent()) else { continue };
        let name = match declaration.kind() {
            "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration" | "annotation_type_declaration" | "object_declaration" => {
                let name = declaration.child_by_field_name("name").or_else(|| children(declaration).into_iter().find(|c| c.kind() == "type_identifier"));
                name.map(|n| qualified(&text[n.byte_range()]))
            }
            "method_declaration" | "constructor_declaration" => declaration.child_by_field_name("name").map(|n| text[n.byte_range()].to_owned()),
            "field_declaration" => declaration.child_by_field_name("declarator").and_then(|d| d.child_by_field_name("name")).map(|n| text[n.byte_range()].to_owned()),
            "function_declaration" => children(declaration).into_iter().find(|c| c.kind() == "simple_identifier").map(|n| text[n.byte_range()].to_owned()),
            "property_declaration" => children(declaration)
                .into_iter()
                .find(|c| c.kind() == "variable_declaration")
                .and_then(|v| children(v).into_iter().find(|c| c.kind() == "simple_identifier"))
                .map(|n| text[n.byte_range()].to_owned()),
            _ => None,
        };
        if let Some(name) = name
            && !out.contains(&name)
        {
            out.push(name);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_annotated_declarations() {
        let java = "package app;\n\n@Entity\npublic class User {\n    @Id long id;\n    @Test public void works() {}\n}\n@javax.persistence.Entity class Order {}\n";
        assert_eq!(annotated_declarations(Lang::Java, java, "javax.persistence.Entity"), vec!["app.User", "app.Order"]);
        assert_eq!(annotated_declarations(Lang::Java, java, "org.junit.Test"), vec!["works"]);
        let kotlin = "package app\n\n@Entity\nclass User {\n    @Test fun works() {}\n    @Id val id = 0\n}\n";
        assert_eq!(annotated_declarations(Lang::Kotlin, kotlin, "javax.persistence.Entity"), vec!["app.User"]);
        assert_eq!(annotated_declarations(Lang::Kotlin, kotlin, "org.junit.Test"), vec!["works"]);
        assert_eq!(annotated_declarations(Lang::Kotlin, kotlin, "Id"), vec!["id"]);
        assert!(annotated_declarations(Lang::Java, java, "Missing").is_empty());
    }
}
