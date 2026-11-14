use schemars::{JsonSchema, Schema, SchemaGenerator};

use crate::model::{Capability, Toolset};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Http {
    pub method: Method,
    pub path: &'static str,
}

#[derive(Debug)]
pub struct OperationSpec {
    pub name: &'static str,
    pub group: &'static str,
    pub toolset: Toolset,
    pub summary: &'static str,
    pub http: Http,
    pub cli: &'static [&'static str],
    pub cli_positional: &'static [&'static str],
    pub mcp_tool: Option<&'static str>,
    pub read_only: bool,
    pub capability: Option<Capability>,
    pub input_schema: fn(&mut SchemaGenerator) -> Schema,
    pub output_schema: fn(&mut SchemaGenerator) -> Schema,
}

pub fn schema<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<T>()
}
