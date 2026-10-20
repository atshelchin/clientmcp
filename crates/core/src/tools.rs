//! The tool set a client declares: validation (SPEC §3.2) and the
//! `x-mcp-header` bindings MCP 2026-07-28 requires a server to enforce.

use std::collections::HashMap;

use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::http::{Headers, decode_header_value};

/// Most tools one client may declare.
pub const MAX_TOOLS: usize = 1_000;
const MAX_NAME_LEN: usize = 128;

/// A validated, ordered tool set.
#[derive(Debug, Default)]
pub struct ToolSet {
    tools: Vec<Tool>,
    index: HashMap<String, usize>,
    /// The declared tools as a JSON array, ready to embed in `tools/list`.
    json: String,
}

#[derive(Debug)]
pub struct Tool {
    pub name: String,
    params: Vec<Param>,
}

/// A tool argument mirrored into an `Mcp-Param-{name}` header.
#[derive(Debug)]
struct Param {
    /// Lowercase header name, `mcp-param-{name}`.
    header: String,
    /// Property names leading from the arguments object to the value.
    path: Vec<String>,
}

impl ToolSet {
    /// Validates declared tools. The tools are kept verbatim, in order.
    pub fn parse(declared: &[Box<RawValue>]) -> Result<ToolSet, String> {
        if declared.len() > MAX_TOOLS {
            return Err(format!("more than {MAX_TOOLS} tools"));
        }
        let mut set = ToolSet {
            json: String::from("["),
            ..ToolSet::default()
        };
        for raw in declared {
            let tool = parse_tool(raw)?;
            if set
                .index
                .insert(tool.name.clone(), set.tools.len())
                .is_some()
            {
                return Err(format!("tool {:?} is declared twice", tool.name));
            }
            if !set.tools.is_empty() {
                set.json.push(',');
            }
            set.json.push_str(raw.get());
            set.tools.push(tool);
        }
        set.json.push(']');
        Ok(set)
    }

    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.index.get(name).map(|&i| &self.tools[i])
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// The tools as a JSON array.
    pub fn json(&self) -> &str {
        if self.tools.is_empty() {
            "[]"
        } else {
            &self.json
        }
    }
}

impl Tool {
    /// Checks that the `Mcp-Param-*` headers of a modern request mirror `arguments`.
    pub fn check_param_headers(
        &self,
        arguments: &RawValue,
        headers: &Headers,
    ) -> Result<(), String> {
        if self.params.is_empty() {
            return Ok(()); // arguments stay unparsed for the common tool
        }
        let arguments: Map<String, Value> = serde_json::from_str(arguments.get())
            .map_err(|_| "arguments must be an object".to_string())?;
        for param in &self.params {
            let value = lookup(&arguments, &param.path).filter(|v| !v.is_null());
            let header = headers.get(&param.header);
            let matches = match (value, header) {
                (None, None) => true,
                (None, Some(_)) | (Some(_), None) => false,
                (Some(value), Some(header)) => {
                    let header = decode_header_value(header)
                        .ok_or_else(|| format!("{} header is malformed", param.header))?;
                    match value {
                        Value::String(s) => *s == header,
                        Value::Bool(b) => header == if *b { "true" } else { "false" },
                        Value::Number(n) => header.parse::<f64>().ok() == n.as_f64(),
                        // Not a primitive: the arguments violate the schema, which the
                        // client rejects; there is no header encoding to compare against.
                        _ => true,
                    }
                }
            };
            if !matches {
                return Err(format!(
                    "{} header does not match the arguments",
                    param.header
                ));
            }
        }
        Ok(())
    }
}

fn lookup<'a>(arguments: &'a Map<String, Value>, path: &[String]) -> Option<&'a Value> {
    let (last, parents) = path.split_last()?;
    let mut object = arguments;
    for key in parents {
        object = object.get(key)?.as_object()?;
    }
    object.get(last)
}

fn parse_tool(raw: &RawValue) -> Result<Tool, String> {
    let tool: Map<String, Value> =
        serde_json::from_str(raw.get()).map_err(|_| "a tool is not a JSON object".to_string())?;
    let name = match tool.get("name") {
        Some(Value::String(name)) if is_tool_name(name) => name.clone(),
        Some(Value::String(name)) => return Err(format!("invalid tool name {name:?}")),
        _ => return Err("a tool has no name".into()),
    };
    let schema = match tool.get("inputSchema") {
        Some(Value::Object(schema)) if schema.get("type") == Some(&Value::from("object")) => schema,
        _ => {
            return Err(format!(
                "tool {name:?}: inputSchema must be an object schema"
            ));
        }
    };
    let mut params = Vec::new();
    collect_params(schema, &mut Vec::new(), true, &mut params)
        .map_err(|e| format!("tool {name:?}: {e}"))?;
    Ok(Tool { name, params })
}

/// MCP's recommended tool-name syntax, enforced: names reach HTTP headers and model APIs.
fn is_tool_name(name: &str) -> bool {
    (1..=MAX_NAME_LEN).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
}

/// Walks a JSON Schema, collecting `x-mcp-header` annotations. `reachable` is
/// true while the walk has followed only `properties` keys from the root, the
/// only place MCP allows the annotation.
fn collect_params(
    schema: &Map<String, Value>,
    path: &mut Vec<String>,
    reachable: bool,
    params: &mut Vec<Param>,
) -> Result<(), String> {
    if let Some(annotation) = schema.get("x-mcp-header") {
        if !reachable || path.is_empty() {
            return Err("x-mcp-header must be on a property reachable through `properties`".into());
        }
        let Some(name) = annotation.as_str().filter(|n| is_token(n)) else {
            return Err("x-mcp-header must be a non-empty HTTP token".into());
        };
        if !matches!(
            schema.get("type").and_then(Value::as_str),
            Some("string" | "integer" | "boolean")
        ) {
            return Err(format!(
                "x-mcp-header {name:?} must be on a string, integer or boolean"
            ));
        }
        let header = format!("mcp-param-{}", name.to_ascii_lowercase());
        if params.iter().any(|p| p.header == header) {
            return Err(format!("x-mcp-header {name:?} is not unique"));
        }
        params.push(Param {
            header,
            path: path.clone(),
        });
    }

    for (keyword, value) in schema {
        match keyword.as_str() {
            "properties" => {
                for (property, sub) in value.as_object().into_iter().flatten() {
                    path.push(property.clone());
                    let result = descend(sub, path, reachable, params);
                    path.pop();
                    result?;
                }
            }
            // Keywords holding a map of subschemas.
            "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => {
                for sub in value.as_object().into_iter().flat_map(Map::values) {
                    descend(sub, path, false, params)?;
                }
            }
            // Keywords holding one subschema, or (`items` in draft-07) a list.
            "additionalProperties"
            | "unevaluatedProperties"
            | "propertyNames"
            | "items"
            | "additionalItems"
            | "unevaluatedItems"
            | "contains"
            | "not"
            | "if"
            | "then"
            | "else"
            | "allOf"
            | "anyOf"
            | "oneOf"
            | "prefixItems" => match value {
                Value::Array(subs) => {
                    for sub in subs {
                        descend(sub, path, false, params)?;
                    }
                }
                sub => descend(sub, path, false, params)?,
            },
            _ => {}
        }
    }
    Ok(())
}

fn descend(
    sub: &Value,
    path: &mut Vec<String>,
    reachable: bool,
    params: &mut Vec<Param>,
) -> Result<(), String> {
    match sub {
        Value::Object(schema) => collect_params(schema, path, reachable, params),
        _ => Ok(()), // boolean schemas carry no annotations
    }
}

/// RFC 9110 `token`: one or more `tchar`.
fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set(tools: Value) -> Result<ToolSet, String> {
        let raw: Vec<Box<RawValue>> = serde_json::from_value(tools).unwrap();
        ToolSet::parse(&raw)
    }

    fn sql_tool() -> Value {
        json!({
            "name": "execute_sql",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "region": { "type": "string", "x-mcp-header": "Region" },
                    "opts": { "type": "object", "properties": {
                        "limit": { "type": "integer", "x-mcp-header": "Limit" }
                    }},
                    "query": { "type": "string" }
                }
            }
        })
    }

    #[test]
    fn keeps_tools_verbatim_and_in_order() {
        let tools = set(json!([
            { "name": "b", "inputSchema": { "type": "object" }, "title": "B" },
            { "name": "a", "inputSchema": { "type": "object" } }
        ]))
        .unwrap();
        assert_eq!(tools.len(), 2);
        let listed: Vec<Value> = serde_json::from_str(tools.json()).unwrap();
        assert_eq!(listed[0]["name"], "b");
        assert_eq!(listed[0]["title"], "B");
        assert!(tools.get("a").is_some() && tools.get("c").is_none());
        assert_eq!(ToolSet::default().json(), "[]");
        assert_eq!(set(json!([])).unwrap().json(), "[]");
    }

    #[test]
    fn rejects_invalid_tools() {
        let schema = json!({ "type": "object" });
        assert!(set(json!([{ "inputSchema": schema }])).is_err());
        assert!(set(json!([{ "name": "has space", "inputSchema": schema }])).is_err());
        assert!(set(json!([{ "name": "x" }])).is_err());
        assert!(set(json!([{ "name": "x", "inputSchema": { "type": "string" } }])).is_err());
        assert!(set(json!([{ "name": "x", "inputSchema": schema }, { "name": "x", "inputSchema": schema }])).is_err());
        assert!(set(json!(["x"])).is_err());
    }

    #[test]
    fn validates_header_annotations() {
        assert!(set(json!([sql_tool()])).is_ok());
        let bad = |property: Value| {
            set(json!([{ "name": "t", "inputSchema": {
                "type": "object", "properties": { "p": property }
            }}]))
        };
        assert!(bad(json!({ "type": "number", "x-mcp-header": "P" })).is_err());
        assert!(bad(json!({ "type": "string", "x-mcp-header": "" })).is_err());
        assert!(bad(json!({ "type": "string", "x-mcp-header": "a b" })).is_err());
        assert!(
            bad(json!({ "type": "array", "items": { "type": "string", "x-mcp-header": "P" } }))
                .is_err()
        );
        assert!(bad(json!({ "anyOf": [{ "type": "string", "x-mcp-header": "P" }] })).is_err());
        // A property may be *named* x-mcp-header; that is not an annotation.
        assert!(
            bad(
                json!({ "type": "object", "properties": { "x-mcp-header": { "type": "string" } } })
            )
            .is_ok()
        );
        let duplicate = set(
            json!([{ "name": "t", "inputSchema": { "type": "object", "properties": {
                "a": { "type": "string", "x-mcp-header": "Same" },
                "b": { "type": "string", "x-mcp-header": "same" }
            }}}]),
        );
        assert!(duplicate.is_err());
    }

    #[test]
    fn checks_param_headers_against_arguments() {
        let tools = set(json!([sql_tool()])).unwrap();
        let tool = tools.get("execute_sql").unwrap();
        let args = |v: Value| RawValue::from_string(v.to_string()).unwrap();
        let headers = |h: &[(&str, &str)]| h.iter().map(|(k, v)| (*k, *v)).collect::<Headers>();

        let full = args(json!({ "region": "us-west1", "opts": { "limit": 42 }, "query": "x" }));
        let ok = headers(&[
            ("Mcp-Param-Region", "us-west1"),
            ("mcp-param-limit", "42.0"),
        ]);
        assert!(tool.check_param_headers(&full, &ok).is_ok());

        let missing = headers(&[("Mcp-Param-Region", "us-west1")]);
        assert!(tool.check_param_headers(&full, &missing).is_err());
        let wrong = headers(&[("Mcp-Param-Region", "eu"), ("mcp-param-limit", "42")]);
        assert!(tool.check_param_headers(&full, &wrong).is_err());

        let unicode = args(json!({ "region": "Hello, 世界" }));
        let encoded = headers(&[("Mcp-Param-Region", "=?base64?SGVsbG8sIOS4lueVjA==?=")]);
        assert!(tool.check_param_headers(&unicode, &encoded).is_ok());

        let absent = args(json!({ "region": null }));
        assert!(tool.check_param_headers(&absent, &headers(&[])).is_ok());
        assert!(tool.check_param_headers(&absent, &missing).is_err());
    }
}
