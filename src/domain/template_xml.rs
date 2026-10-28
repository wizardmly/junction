//! Live templates as IntelliJ keeps them (the `templates/*.xml` of its
//! settings folder, what Export / Import of the Live Templates page
//! write and read): a `<templateSet group="…">` of `<template>`s, each
//! with its `<variable>`s and the `<context>` options it applies in.

use super::live_templates::{Context, UserTemplate};

/// IntelliJ's context option names for Kotlin.
const KOTLIN: [(Context, &str); 5] = [
    (Context::Statement, "KOTLIN_STATEMENT"),
    (Context::Expression, "KOTLIN_EXPRESSION"),
    (Context::Class, "KOTLIN_CLASS"),
    (Context::TopLevel, "KOTLIN_TOPLEVEL"),
    (Context::Object, "KOTLIN_OBJECT_DECLARATION"),
];

/// And for Java: a class member is a declaration.
const JAVA: [(Context, &str); 3] = [(Context::Statement, "JAVA_STATEMENT"), (Context::Expression, "JAVA_EXPRESSION"), (Context::Class, "JAVA_DECLARATION")];

fn attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\t' => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
    out
}

/// A group's templates as IntelliJ writes them.
pub fn export(group: &str, templates: &[UserTemplate]) -> String {
    let mut out = format!("<templateSet group=\"{}\">\n", attr(group));
    for t in templates {
        out.push_str(&format!("  <template name=\"{}\" value=\"{}\" description=\"{}\" toReformat=\"true\" toShortenFQNames=\"true\"", attr(&t.abbreviation), attr(&t.text), attr(&t.description)));
        if !t.enabled {
            out.push_str(" deactivated=\"true\"");
        }
        out.push_str(">\n");
        for name in super::live_templates::variables(&t.text) {
            let value = |list: &[(String, String)]| list.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone()).unwrap_or_default();
            let (expression, default) = (value(&t.expressions), value(&t.defaults));
            // A default value is an expression to IntelliJ: text is quoted.
            let default = if default.is_empty() { default } else { format!("\"{}\"", default.replace('"', "\\\"")) };
            let stop = !t.skip.contains(&name);
            out.push_str(&format!("    <variable name=\"{}\" expression=\"{}\" defaultValue=\"{}\" alwaysStopAt=\"{stop}\" />\n", attr(&name), attr(&expression), attr(&default)));
        }
        out.push_str("    <context>\n");
        for (place, option) in KOTLIN {
            if t.kotlin.contains(&place) {
                out.push_str(&format!("      <option name=\"{option}\" value=\"true\" />\n"));
            }
        }
        for (place, option) in JAVA {
            if t.java.contains(&place) {
                out.push_str(&format!("      <option name=\"{option}\" value=\"true\" />\n"));
            }
        }
        out.push_str("    </context>\n  </template>\n");
    }
    out.push_str("</templateSet>\n");
    out
}

/// The templates of an IntelliJ template file, in its group. Contexts of
/// other languages are dropped; "KOTLIN" and "JAVA_CODE" (anywhere in the
/// language) tick all of that language's.
pub fn import(text: &str) -> Result<Vec<UserTemplate>, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("Not a template file: {e}"))?;
    let set = doc.root_element();
    if set.tag_name().name() != "templateSet" {
        return Err("Not a template file: no <templateSet>".into());
    }
    let group = set.attribute("group").unwrap_or_default().to_owned();
    let mut out = Vec::new();
    for node in set.children().filter(|n| n.has_tag_name("template")) {
        let Some(name) = node.attribute("name") else { continue };
        let mut t = UserTemplate {
            abbreviation: name.to_owned(),
            description: node.attribute("description").unwrap_or_default().to_owned(),
            text: node.attribute("value").unwrap_or_default().to_owned(),
            enabled: node.attribute("deactivated") != Some("true"),
            group: if group == super::live_templates::USER_GROUP { String::new() } else { group.clone() },
            ..Default::default()
        };
        for v in node.children().filter(|n| n.has_tag_name("variable")) {
            let Some(var) = v.attribute("name") else { continue };
            let mut expression = v.attribute("expression").unwrap_or_default().trim().to_owned();
            let default = v.attribute("defaultValue").unwrap_or_default().trim();
            match default.strip_prefix('"').and_then(|d| d.strip_suffix('"')) {
                Some(literal) => t.defaults.push((var.to_owned(), literal.replace("\\\"", "\""))),
                // A computed default stands in for a missing expression.
                None if !default.is_empty() && expression.is_empty() => expression = default.to_owned(),
                None => {}
            }
            if !expression.is_empty() {
                t.expressions.push((var.to_owned(), expression));
            }
            if v.attribute("alwaysStopAt") == Some("false") {
                t.skip.push(var.to_owned());
            }
        }
        for context in node.children().filter(|n| n.has_tag_name("context")) {
            for option in context.children().filter(|n| n.has_tag_name("option")) {
                let (Some(option), on) = (option.attribute("name"), option.attribute("value") == Some("true")) else { continue };
                let set = |list: &mut Vec<Context>, places: &[Context]| {
                    for place in places {
                        list.retain(|c| c != place);
                        if on {
                            list.push(*place);
                        }
                    }
                };
                match option {
                    "KOTLIN" => set(&mut t.kotlin, &KOTLIN.map(|(c, _)| c)),
                    "JAVA_CODE" => set(&mut t.java, &JAVA.map(|(c, _)| c)),
                    _ => {
                        if let Some((place, _)) = KOTLIN.iter().find(|(_, o)| *o == option) {
                            set(&mut t.kotlin, &[*place]);
                        } else if let Some((place, _)) = JAVA.iter().find(|(_, o)| *o == option) {
                            set(&mut t.java, &[*place]);
                        }
                    }
                }
            }
        }
        out.push(t);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_intellij_template_file() {
        // As Android Studio writes its AndroidLog group.
        let xml = r#"<templateSet group="AndroidLog">
  <template name="logd" value="android.util.Log.d(TAG, &quot;$METHOD_NAME$: $content$&quot;);" description="Log.d(String)" toReformat="true" toShortenFQNames="true">
    <variable name="METHOD_NAME" expression="methodName()" defaultValue="" alwaysStopAt="false" />
    <variable name="content" expression="" defaultValue="&quot;msg&quot;" alwaysStopAt="true" />
    <context>
      <option name="JAVA_STATEMENT" value="true" />
      <option name="KOTLIN" value="true" />
      <option name="KOTLIN_TOPLEVEL" value="false" />
      <option name="GROOVY" value="true" />
    </context>
  </template>
  <template name="off" value="x&#10;y" description="" deactivated="true"><context /></template>
</templateSet>"#;
        let templates = import(xml).unwrap();
        let t = &templates[0];
        assert_eq!((t.abbreviation.as_str(), t.group.as_str()), ("logd", "AndroidLog"));
        assert_eq!(t.text, "android.util.Log.d(TAG, \"$METHOD_NAME$: $content$\");");
        assert_eq!(t.expressions, vec![("METHOD_NAME".to_owned(), "methodName()".to_owned())]);
        assert_eq!(t.defaults, vec![("content".to_owned(), "msg".to_owned())]);
        assert_eq!(t.skip, vec!["METHOD_NAME".to_owned()]);
        assert_eq!(t.java, vec![Context::Statement]);
        assert_eq!(t.kotlin, vec![Context::Statement, Context::Expression, Context::Class, Context::Object]);
        assert_eq!((templates[1].text.as_str(), templates[1].enabled), ("x\ny", false));
        assert!(import("<application/>").is_err());
        assert!(import("not xml").is_err());
    }

    #[test]
    fn round_trips() {
        let t = UserTemplate {
            abbreviation: "pv".into(),
            description: "Print a \"value\"".into(),
            text: "System.out.println(\"$NAME$ = \" + $NAME$);\n$END$".into(),
            defaults: vec![("NAME".into(), "value".into())],
            expressions: vec![("NAME".into(), "suggestVariableName()".into())],
            skip: vec!["NAME".into()],
            kotlin: vec![Context::TopLevel],
            java: vec![Context::Statement, Context::Class],
            enabled: true,
            group: "mine".into(),
        };
        let xml = export("mine", std::slice::from_ref(&t));
        assert!(xml.starts_with("<templateSet group=\"mine\">"));
        assert_eq!(import(&xml).unwrap(), vec![t.clone()]);
        // The user group is the default one.
        let user = UserTemplate { group: String::new(), ..t };
        assert_eq!(import(&export("user", std::slice::from_ref(&user))).unwrap(), vec![user]);
    }
}
