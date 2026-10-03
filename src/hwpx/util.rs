use roxmltree::Node;

use crate::error::{ConvertError, Result};

pub fn attr<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<&'a str> {
    node.attribute(name)
}

pub fn attr_string<'a, 'input>(node: Node<'a, 'input>, name: &str) -> String {
    attr(node, name).unwrap_or_default().to_owned()
}

pub fn attr_i64<'a, 'input>(node: Node<'a, 'input>, name: &str, default: i64) -> i64 {
    attr(node, name)
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(default)
}

pub fn attr_coord<'a, 'input>(node: Node<'a, 'input>, name: &str, default: i64) -> i64 {
    let value = attr_i64(node, name, default);
    // HWPML stores some signed offsets in an unsigned 32-bit attribute.
    if value > i64::from(i32::MAX) {
        value - (1_i64 << 32)
    } else {
        value
    }
}

pub fn attr_u32<'a, 'input>(node: Node<'a, 'input>, name: &str, default: u32) -> u32 {
    attr_i64(node, name, default as i64).max(0) as u32
}

pub fn attr_usize<'a, 'input>(node: Node<'a, 'input>, name: &str, default: usize) -> usize {
    attr_i64(node, name, default as i64).max(0) as usize
}

pub fn attr_bool<'a, 'input>(node: Node<'a, 'input>, name: &str) -> bool {
    matches!(
        attr(node, name),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

pub fn local_name<'a, 'input>(node: Node<'a, 'input>) -> &'a str {
    node.tag_name().name()
}

pub fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|candidate| candidate.is_element() && local_name(*candidate) == name)
}

pub fn children<'a, 'input>(
    node: Node<'a, 'input>,
    name: &str,
) -> impl Iterator<Item = Node<'a, 'input>> {
    let name = name.to_owned();
    node.children()
        .filter(move |candidate| candidate.is_element() && local_name(*candidate) == name)
}

pub fn descendants<'a, 'input>(
    node: Node<'a, 'input>,
    name: &str,
) -> impl Iterator<Item = Node<'a, 'input>> {
    let name = name.to_owned();
    node.descendants()
        .filter(move |candidate| candidate.is_element() && local_name(*candidate) == name)
}

pub fn parse_xml<'a>(bytes: &'a [u8], path: &str) -> Result<roxmltree::Document<'a>> {
    let text = std::str::from_utf8(bytes).map_err(|error| ConvertError::Xml {
        path: path.to_owned(),
        message: format!("not UTF-8: {error}"),
    })?;
    roxmltree::Document::parse(text).map_err(|error| ConvertError::Xml {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

pub fn text_content<'a, 'input>(node: Node<'a, 'input>) -> String {
    node.descendants()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect::<String>()
}
