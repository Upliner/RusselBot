//! A small value model shared by LLSD XML and XML-RPC, with parsers and writers.

use roxmltree::{Document, Node};
use std::collections::BTreeMap;
use std::fmt;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Undef,
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(String),
    Uuid(Uuid),
    Binary(Vec<u8>),
    Array(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

#[derive(Debug, PartialEq)]
pub struct XmlError(pub String);

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, XmlError> {
    Err(XmlError(msg.into()))
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(map) => map.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Integers, and strings holding an integer (grids are not consistent).
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_uuid(&self) -> Option<Uuid> {
        match self {
            Value::Uuid(u) => Some(*u),
            Value::Str(s) => Uuid::parse_str(s.trim()).ok(),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Binary(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub fn str_field(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }
}

fn elements<'a, 'i>(node: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(Node::is_element)
}

fn text<'a>(node: Node<'a, '_>) -> &'a str {
    node.text().unwrap_or("")
}

fn decode_base64(input: &str) -> Result<Vec<u8>, XmlError> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in input.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return err("invalid base64 data"),
        };
        acc = acc << 6 | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

fn parse_document(input: &str) -> Result<Document<'_>, XmlError> {
    Document::parse(input).map_err(|e| XmlError(format!("malformed XML: {e}")))
}

// ---------------------------------------------------------------- LLSD XML

pub fn parse_llsd(input: &str) -> Result<Value, XmlError> {
    let doc = parse_document(input)?;
    let root = doc.root_element();
    if root.tag_name().name() != "llsd" {
        return err("not an LLSD document");
    }
    let first = elements(root).next();
    first.map_or(Ok(Value::Undef), llsd_value)
}

fn llsd_value(node: Node) -> Result<Value, XmlError> {
    let body = text(node);
    Ok(match node.tag_name().name() {
        "undef" => Value::Undef,
        "boolean" => Value::Bool(matches!(body.trim(), "1" | "true")),
        "integer" => Value::Int(body.trim().parse().unwrap_or(0)),
        "real" => Value::Real(body.trim().parse().unwrap_or(0.0)),
        "string" | "uri" | "date" => Value::Str(body.to_string()),
        "uuid" => Value::Uuid(Uuid::parse_str(body.trim()).unwrap_or(Uuid::nil())),
        "binary" => Value::Binary(decode_base64(body)?),
        "array" => Value::Array(elements(node).map(llsd_value).collect::<Result<_, _>>()?),
        "map" => {
            let mut map = BTreeMap::new();
            let mut children = elements(node);
            while let Some(key) = children.next() {
                if key.tag_name().name() != "key" {
                    return err("LLSD map entry without <key>");
                }
                let value = children.next().ok_or(XmlError("LLSD map key without value".into()))?;
                map.insert(text(key).to_string(), llsd_value(value)?);
            }
            Value::Map(map)
        }
        other => return err(format!("unknown LLSD element <{other}>")),
    })
}

pub fn write_llsd(value: &Value) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><llsd>");
    llsd_element(value, &mut out);
    out.push_str("</llsd>");
    out
}

fn llsd_element(value: &Value, out: &mut String) {
    match value {
        Value::Undef => out.push_str("<undef />"),
        Value::Bool(b) => out.push_str(&format!("<boolean>{}</boolean>", u8::from(*b))),
        Value::Int(i) => out.push_str(&format!("<integer>{i}</integer>")),
        Value::Real(r) => out.push_str(&format!("<real>{r}</real>")),
        Value::Str(s) => out.push_str(&format!("<string>{}</string>", escape(s))),
        Value::Uuid(u) => out.push_str(&format!("<uuid>{u}</uuid>")),
        // The bot never sends binary data.
        Value::Binary(_) => out.push_str("<binary />"),
        Value::Array(items) => {
            out.push_str("<array>");
            items.iter().for_each(|item| llsd_element(item, out));
            out.push_str("</array>");
        }
        Value::Map(map) => {
            out.push_str("<map>");
            for (key, item) in map {
                out.push_str(&format!("<key>{}</key>", escape(key)));
                llsd_element(item, out);
            }
            out.push_str("</map>");
        }
    }
}

// ----------------------------------------------------------------- XML-RPC

/// Parses an XML-RPC `methodResponse` and returns its single parameter.
pub fn parse_xmlrpc_response(input: &str) -> Result<Value, XmlError> {
    let doc = parse_document(input)?;
    let root = doc.root_element();
    if root.tag_name().name() != "methodResponse" {
        return err("not an XML-RPC response");
    }
    let first = elements(root).next().ok_or(XmlError("empty XML-RPC response".into()))?;
    match first.tag_name().name() {
        "fault" => {
            let fault = elements(first).next().map(xmlrpc_value).transpose()?;
            let reason = fault.as_ref().and_then(|f| f.str_field("faultString")).unwrap_or("unknown fault");
            err(format!("XML-RPC fault: {reason}"))
        }
        "params" => {
            let value = elements(first)
                .next()
                .and_then(|param| elements(param).next())
                .ok_or(XmlError("XML-RPC response without a value".into()))?;
            xmlrpc_value(value)
        }
        other => err(format!("unexpected XML-RPC element <{other}>")),
    }
}

/// Parses a `<value>` element.
fn xmlrpc_value(node: Node) -> Result<Value, XmlError> {
    if node.tag_name().name() != "value" {
        return err("expected XML-RPC <value>");
    }
    // A <value> with no type element is a string.
    let Some(typed) = elements(node).next() else {
        return Ok(Value::Str(text(node).to_string()));
    };
    let body = text(typed);
    Ok(match typed.tag_name().name() {
        "string" => Value::Str(body.to_string()),
        "int" | "i4" | "i8" => Value::Int(body.trim().parse().unwrap_or(0)),
        "boolean" => Value::Bool(matches!(body.trim(), "1" | "true")),
        "double" => Value::Real(body.trim().parse().unwrap_or(0.0)),
        "base64" => Value::Binary(decode_base64(body)?),
        "nil" => Value::Undef,
        "array" => {
            let mut items = Vec::new();
            for data in elements(typed) {
                for item in elements(data) {
                    items.push(xmlrpc_value(item)?);
                }
            }
            Value::Array(items)
        }
        "struct" => {
            let mut map = BTreeMap::new();
            for member in elements(typed) {
                let mut name = None;
                let mut value = Value::Undef;
                for part in elements(member) {
                    match part.tag_name().name() {
                        "name" => name = Some(text(part).to_string()),
                        "value" => value = xmlrpc_value(part)?,
                        _ => {}
                    }
                }
                map.insert(name.ok_or(XmlError("XML-RPC member without <name>".into()))?, value);
            }
            Value::Map(map)
        }
        other => return err(format!("unknown XML-RPC type <{other}>")),
    })
}

/// Writes an XML-RPC call whose single parameter is a struct of `members`.
pub fn write_xmlrpc_call(method: &str, members: &[(&str, Value)]) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\"?><methodCall><methodName>{}</methodName><params><param><value><struct>",
        escape(method)
    );
    for (name, value) in members {
        out.push_str(&format!("<member><name>{}</name>", escape(name)));
        xmlrpc_element(value, &mut out);
        out.push_str("</member>");
    }
    out.push_str("</struct></value></param></params></methodCall>");
    out
}

fn xmlrpc_element(value: &Value, out: &mut String) {
    out.push_str("<value>");
    match value {
        Value::Undef | Value::Binary(_) => out.push_str("<string></string>"),
        Value::Bool(b) => out.push_str(&format!("<boolean>{}</boolean>", u8::from(*b))),
        Value::Int(i) => out.push_str(&format!("<int>{i}</int>")),
        Value::Real(r) => out.push_str(&format!("<double>{r}</double>")),
        Value::Str(s) => out.push_str(&format!("<string>{}</string>", escape(s))),
        Value::Uuid(u) => out.push_str(&format!("<string>{u}</string>")),
        Value::Array(items) => {
            out.push_str("<array><data>");
            items.iter().for_each(|item| xmlrpc_element(item, out));
            out.push_str("</data></array>");
        }
        Value::Map(map) => {
            out.push_str("<struct>");
            for (name, item) in map {
                out.push_str(&format!("<member><name>{}</name>", escape(name)));
                xmlrpc_element(item, out);
                out.push_str("</member>");
            }
            out.push_str("</struct>");
        }
    }
    out.push_str("</value>");
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llsd_nested_maps_and_arrays() {
        let doc = "<?xml version=\"1.0\"?><llsd><map>\
            <key>events</key><array><map>\
              <key>message</key><string>TeleportFinish</string>\
              <key>body</key><map><key>Info</key><array><map>\
                <key>SimPort</key><integer>9001</integer>\
                <key>SimIP</key><binary encoding=\"base64\">fwAAAQ==</binary>\
                <key>AgentID</key><uuid>11111111-2222-3333-4444-555555555555</uuid>\
              </map></array></map>\
            </map></array>\
            <key>id</key><integer>-7</integer>\
            <key>flag</key><boolean>true</boolean>\
            <key>ratio</key><real>1.5</real>\
            <key>none</key><undef />\
          </map></llsd>";
        let value = parse_llsd(doc).unwrap();
        assert_eq!(value.get("id").unwrap().as_i64(), Some(-7));
        assert_eq!(value.get("flag"), Some(&Value::Bool(true)));
        assert_eq!(value.get("ratio"), Some(&Value::Real(1.5)));
        assert_eq!(value.get("none"), Some(&Value::Undef));
        let event = &value.get("events").unwrap().as_array().unwrap()[0];
        assert_eq!(event.str_field("message"), Some("TeleportFinish"));
        let info = &event.get("body").unwrap().get("Info").unwrap().as_array().unwrap()[0];
        assert_eq!(info.get("SimPort").unwrap().as_i64(), Some(9001));
        assert_eq!(info.get("SimIP").unwrap().as_bytes(), Some(&[127u8, 0, 0, 1][..]));
        assert_eq!(
            info.get("AgentID").unwrap().as_uuid(),
            Uuid::parse_str("11111111-2222-3333-4444-555555555555").ok()
        );
    }

    #[test]
    fn llsd_escaped_text_round_trips() {
        let value = Value::Array(vec![Value::Str("a<b & \"c\">".into()), Value::Int(3)]);
        assert_eq!(parse_llsd(&write_llsd(&value)).unwrap(), value);
    }

    #[test]
    fn llsd_rejects_garbage() {
        assert!(parse_llsd("<llsd><map><string>x</string></map></llsd>").is_err());
        assert!(parse_llsd("not xml").is_err());
        assert!(parse_llsd("<html></html>").is_err());
    }

    #[test]
    fn xmlrpc_response_struct() {
        let doc = "<?xml version=\"1.0\" encoding=\"utf-8\"?><methodResponse><params><param><value><struct>\
            <member><name>login</name><value><string>true</string></value></member>\
            <member><name>sim_port</name><value><int>9000</int></value></member>\
            <member><name>bare</name><value>plain &amp; simple</value></member>\
            <member><name>list</name><value><array><data>\
              <value><i4>1</i4></value><value><boolean>1</boolean></value>\
            </data></array></value></member>\
          </struct></value></param></params></methodResponse>";
        let value = parse_xmlrpc_response(doc).unwrap();
        assert_eq!(value.str_field("login"), Some("true"));
        assert_eq!(value.get("sim_port").unwrap().as_i64(), Some(9000));
        assert_eq!(value.str_field("bare"), Some("plain & simple"));
        assert_eq!(value.get("list").unwrap().as_array(), Some(&[Value::Int(1), Value::Bool(true)][..]));
    }

    #[test]
    fn xmlrpc_fault_is_an_error() {
        let doc = "<methodResponse><fault><value><struct>\
            <member><name>faultCode</name><value><int>4</int></value></member>\
            <member><name>faultString</name><value><string>Too busy</string></value></member>\
          </struct></value></fault></methodResponse>";
        assert!(parse_xmlrpc_response(doc).unwrap_err().0.contains("Too busy"));
    }

    #[test]
    fn xmlrpc_call_is_well_formed_and_escaped() {
        let call = write_xmlrpc_call(
            "login_to_simulator",
            &[
                ("first", Value::Str("A&B".into())),
                ("agree_to_tos", Value::Bool(true)),
                ("options", Value::Array(vec![])),
            ],
        );
        let doc = Document::parse(&call).unwrap();
        let names: Vec<&str> = doc
            .descendants()
            .filter(|n| n.tag_name().name() == "name")
            .map(|n| n.text().unwrap())
            .collect();
        assert_eq!(names, ["first", "agree_to_tos", "options"]);
        assert!(call.contains("<string>A&amp;B</string>"));
        assert!(call.contains("<methodName>login_to_simulator</methodName>"));
    }

    #[test]
    fn base64_with_and_without_padding() {
        assert_eq!(decode_base64("AAAD6AAAA+g=").unwrap(), [0, 0, 3, 232, 0, 0, 3, 232]);
        assert_eq!(decode_base64("Zm9v").unwrap(), b"foo");
        assert!(decode_base64("Zm9v!").is_err());
    }
}
