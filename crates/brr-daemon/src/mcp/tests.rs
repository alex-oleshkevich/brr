use super::{Backend, CLIENT_ID_ARG, Ensure, Forwarder, McpServer, object_schema};
use crate::{ops, remote::Remote};

const NO_SOCKET: &str = "/nonexistent/brr.sock";

#[test]
fn schemas_are_inline_objects() {
    for op in ops::all() {
        for build in [op.spec.input_schema, op.spec.output_schema] {
            let schema = serde_json::to_string(&object_schema(build, true)).unwrap();

            assert!(!schema.contains("$ref"), "{}: {schema}", op.spec.name);
        }
    }
}

#[test]
fn forward_tools_have_no_client_id_property() {
    let remote = Remote::new(std::path::Path::new(NO_SOCKET)).unwrap();
    let ensure: Ensure = Box::new(|| Box::pin(async { Ok(()) }));
    let server = McpServer::new(
        Backend::Forward(Forwarder::new(remote, "test", ensure)),
        None,
    );

    let tool = server.tool(ops::find("settings.get").unwrap()).unwrap();
    let client_id = tool
        .input_schema
        .get("properties")
        .and_then(|properties| properties.get(CLIENT_ID_ARG));

    assert!(client_id.is_none());
}
