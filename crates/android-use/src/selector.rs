use serde::{Deserialize, Serialize};

use crate::error::{AuError, Result};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Node {
    pub id: u64,
    pub text: String,
    pub description: String,
    pub resource_id: String,
    pub class_name: String,
    pub package_name: String,
    pub clickable: bool,
    pub enabled: bool,
    pub scrollable: bool,
    pub checked: bool,
    pub bounds: [i32; 4],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Operator {
    Equals,
    Contains,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Term {
    pub field: String,
    pub operator: Operator,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selector {
    pub terms: Vec<Term>,
    pub occurrence: usize,
}

impl Selector {
    pub fn parse(input: &str) -> Result<Self> {
        let (body, occurrence) = split_occurrence(input)?;
        if body.is_empty() {
            return Err(AuError::code("E_SELECTOR", "selector has no terms"));
        }
        let terms = split_escaped(body, ',')
            .into_iter()
            .map(|term| parse_term(&term))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { terms, occurrence })
    }

    pub fn matches(&self, node: &Node) -> bool {
        self.terms.iter().all(|term| match term.field.as_str() {
            "text" => string_match(&node.text, term),
            "desc" => string_match(&node.description, term),
            "id" => string_match(&node.resource_id, term),
            "class" => string_match(&node.class_name, term),
            "pkg" => string_match(&node.package_name, term),
            "clickable" => bool_match(node.clickable, term),
            "enabled" => bool_match(node.enabled, term),
            "scrollable" => bool_match(node.scrollable, term),
            "checked" => bool_match(node.checked, term),
            "bounds" => {
                term.operator == Operator::Equals && term.value == format_bounds(node.bounds)
            }
            _ => false,
        })
    }
}

fn split_occurrence(input: &str) -> Result<(&str, usize)> {
    let mut escaped = false;
    let mut index = None;
    for (position, character) in input.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '#' {
            index = Some(position);
        }
    }
    match index {
        None => Ok((input, 0)),
        Some(position) => {
            let suffix = &input[position + 1..];
            if suffix.is_empty() || !suffix.chars().all(|character| character.is_ascii_digit()) {
                return Err(AuError::code(
                    "E_SELECTOR",
                    "occurrence must be a non-negative integer",
                ));
            }
            Ok((&input[..position], suffix.parse()?))
        }
    }
}

fn parse_term(input: &str) -> Result<Term> {
    let input = input.trim();
    let (field, operator, value) = if let Some((field, value)) = split_first_unescaped(input, '~') {
        (field, Operator::Contains, value)
    } else if let Some((field, value)) = split_first_unescaped(input, '=') {
        (field, Operator::Equals, value)
    } else {
        return Err(AuError::code(
            "E_SELECTOR",
            format!("missing = or ~ in {input}"),
        ));
    };
    let field = field.trim().to_ascii_lowercase();
    if !matches!(
        field.as_str(),
        "text"
            | "desc"
            | "id"
            | "class"
            | "pkg"
            | "clickable"
            | "enabled"
            | "scrollable"
            | "checked"
            | "bounds"
    ) {
        return Err(AuError::code(
            "E_SELECTOR",
            format!("unsupported selector field {field}"),
        ));
    }
    if matches!(
        field.as_str(),
        "clickable" | "enabled" | "scrollable" | "checked" | "bounds"
    ) && operator != Operator::Equals
    {
        return Err(AuError::code(
            "E_SELECTOR",
            format!("{field} only accepts ="),
        ));
    }
    Ok(Term {
        field,
        operator,
        value: unescape(value.trim()),
    })
}

fn split_escaped(input: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for character in input.chars() {
        if escaped {
            current.push('\\');
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == delimiter {
            parts.push(current);
            current = String::new();
        } else {
            current.push(character);
        }
    }
    if escaped {
        current.push('\\');
    }
    parts.push(current);
    parts
}

fn split_first_unescaped(input: &str, delimiter: char) -> Option<(&str, &str)> {
    let mut escaped = false;
    for (position, character) in input.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == delimiter {
            return Some((
                &input[..position],
                &input[position + character.len_utf8()..],
            ));
        }
    }
    None
}

fn unescape(input: &str) -> String {
    let mut value = String::new();
    let mut escaped = false;
    for character in input.chars() {
        if escaped {
            value.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            value.push(character);
        }
    }
    if escaped {
        value.push('\\');
    }
    value
}

fn string_match(value: &str, term: &Term) -> bool {
    match term.operator {
        Operator::Equals => value == term.value,
        Operator::Contains => value.contains(&term.value),
    }
}

fn bool_match(value: bool, term: &Term) -> bool {
    matches!(
        (value, term.value.as_str()),
        (true, "true") | (false, "false")
    )
}

fn format_bounds(bounds: [i32; 4]) -> String {
    format!("{},{},{},{}", bounds[0], bounds[1], bounds[2], bounds[3])
}

#[cfg(test)]
mod tests {
    use super::{Node, Selector};

    fn allow_node() -> Node {
        Node {
            id: 7,
            text: "Allow while using the app".into(),
            description: String::new(),
            resource_id: "android:id/button1".into(),
            class_name: "android.widget.Button".into(),
            package_name: "android".into(),
            clickable: true,
            enabled: true,
            scrollable: false,
            checked: false,
            bounds: [1, 2, 3, 4],
        }
    }

    #[test]
    fn selector_accepts_documented_example() {
        let selector = Selector::parse("text~Allow,clickable=true#0").expect("selector");
        assert!(selector.matches(&allow_node()));
    }

    #[test]
    fn selector_unescapes_comma() {
        let selector = Selector::parse(r"text~a\,b#2").expect("selector");
        assert_eq!(selector.occurrence, 2);
        assert_eq!(selector.terms[0].value, "a,b");
    }
}
