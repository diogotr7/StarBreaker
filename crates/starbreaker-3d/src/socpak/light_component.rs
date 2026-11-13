//! Attribute-wise light class inheritance, shared by SOC parsing and export resolution.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use starbreaker_cryxml::{CryXml, CryXmlNode};
use starbreaker_datacore::{Database, query::value::Value};

use crate::types::LightInfo;

/// Unresolved SOC source retained until the export supplies its DataCore database.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LightComponent {
    pub class: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub nodes: BTreeSet<String>,
}

impl LightComponent {
    pub fn from_xml(xml: &CryXml, node: &CryXmlNode) -> Self {
        fn walk(xml: &CryXml, node: &CryXmlNode, path: &str, out: &mut LightComponent) {
            out.nodes.insert(path.to_owned());
            for (key, value) in xml
                .node_attributes(node)
                .filter(|(k, _)| !k.starts_with("__"))
            {
                out.attributes
                    .insert(format!("{path}{key}"), value.to_owned());
            }
            for child in xml.node_children(node) {
                walk(xml, child, &format!("{path}{}.", xml.node_tag(child)), out);
            }
        }
        let mut out = Self::default();
        walk(xml, node, "", &mut out);
        out
    }

    pub fn from_datacore(value: &Value<'_>) -> Self {
        fn walk(value: &Value<'_>, path: &str, out: &mut LightComponent) {
            let scalar = match value {
                Value::Object { fields, .. } => {
                    out.nodes.insert(path.to_owned());
                    for (key, value) in fields {
                        let next = format!("{path}{key}");
                        if matches!(value, Value::Object { .. }) {
                            walk(value, &format!("{next}."), out);
                        } else {
                            walk(value, &next, out);
                        }
                    }
                    return;
                }
                Value::Bool(v) => v.to_string(),
                Value::Float(v) => v.to_string(),
                Value::Double(v) => v.to_string(),
                Value::Int8(v) => v.to_string(),
                Value::Int16(v) => v.to_string(),
                Value::Int32(v) => v.to_string(),
                Value::Int64(v) => v.to_string(),
                Value::UInt8(v) => v.to_string(),
                Value::UInt16(v) => v.to_string(),
                Value::UInt32(v) => v.to_string(),
                Value::UInt64(v) => v.to_string(),
                Value::String(v) | Value::Enum(v) => (*v).to_owned(),
                _ => return,
            };
            out.attributes.insert(path.to_owned(), scalar);
        }
        let mut out = Self::default();
        walk(value, "", &mut out);
        out
    }

    pub fn attr(&self, path: &str) -> Option<&str> {
        self.attributes.get(path).map(String::as_str)
    }

    pub fn float(&self, path: &str) -> Option<f32> {
        self.attr(path)?.parse().ok()
    }

    pub fn boolean(&self, path: &str) -> Option<bool> {
        match self.attr(path)? {
            "1" | "true" | "True" | "TRUE" => Some(true),
            "0" | "false" | "False" | "FALSE" => Some(false),
            _ => None,
        }
    }

    pub fn inherit(&self, base: &Self) -> Self {
        let mut merged = base.clone();
        merged.class.clone_from(&self.class);
        merged.attributes.extend(self.attributes.clone());
        merged.nodes.extend(self.nodes.clone());
        merged
    }
}

/// One resolver per export, shared across root and child containers. Class lookups
/// and diagnostics are bounded independently of the number of light instances.
pub(crate) struct LightResolver<'db, 'data> {
    db: &'db Database<'data>,
    classes: HashMap<String, Option<LightComponent>>,
    importance: Option<Vec<&'db str>>,
    warned_gi: bool,
}

impl<'db, 'data> LightResolver<'db, 'data> {
    pub fn new(db: &'db Database<'data>) -> Self {
        Self {
            db,
            classes: HashMap::new(),
            importance: None,
            warned_gi: false,
        }
    }

    pub fn class_component(&mut self, class: &str) -> Option<&LightComponent> {
        self.classes
            .entry(class.to_owned())
            .or_insert_with(|| {
                let record_name = if class.starts_with("EntityClassDefinition.") {
                    class.to_owned()
                } else {
                    format!("EntityClassDefinition.{class}")
                };
                let record = self
                    .db
                    .records_of_type(self.db.struct_id("EntityClassDefinition")?)
                    .find(|r| {
                        self.db
                            .resolve_string2(r.name_offset)
                            .eq_ignore_ascii_case(&record_name)
                    })?;
                let path = self
                    .db
                    .compile_path::<Value>(record.struct_id(), "Components[LightComponentParams]")
                    .ok()?;
                let values = self.db.query::<Value>(&path, record).ok()?;
                values.first().map(LightComponent::from_datacore)
            })
            .as_ref()
    }

    pub fn resolve(&mut self, mut light: LightInfo) -> LightInfo {
        if let Some(source) = light.source_component.take() {
            let merged = source
                .class
                .as_deref()
                .and_then(|class| self.class_component(class))
                .map_or_else(|| source.clone(), |base| source.inherit(base));
            let attachment = light.attachment.take();
            light = super::build_light_info_from_fields(
                &merged,
                &light.name,
                &light.position,
                &light.rotation,
            );
            light.attachment = attachment;
            light.source_component = None;
        }
        let options = self
            .importance
            .get_or_insert_with(|| super::light_fields::importance_options(self.db));
        super::light_fields::resolve_importance(&mut light, options);
        if light.affect_gi.is_none() && light.affect_gi_mode.is_some() && !self.warned_gi {
            log::warn!(
                "Unresolved light affectGI mode {:?}; preserving mode with null affect_gi",
                light.affect_gi_mode
            );
            self.warned_gi = true;
        }
        light
    }
}
