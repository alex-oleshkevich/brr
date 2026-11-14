use std::{fs::File, io};

use base64::{engine::general_purpose::STANDARD, read::DecoderReader, write::EncoderStringWriter};

use brr_core::{
    Backup, Error, FileOut, Http, Method, OperationSpec, TextUpload, Toolset, Upload, schema,
};
use brr_store::profile;
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, invalid, output, parse};
use crate::{
    app::Ctx,
    internal,
    services::backups::{self, Imported, Scratch, passphrase},
    wire::{BYTES_PER_MIB, MAX_INLINE_FILE_BYTES},
};

const FILE_KEY: &str = "file";

static EXPORT: OperationSpec = OperationSpec {
    name: "profile.export",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Export a stopped Persistent Profile as an age-encrypted archive, returned inline or written to the storage backend",
    http: Http {
        method: Method::Post,
        path: "/v1/profiles/{profile}/export",
    },
    cli: &["profile", "export"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_export"),
    read_only: false,
    capability: None,
    input_schema: schema::<ExportInput>,
    output_schema: schema::<ExportOutput>,
};

static IMPORT: OperationSpec = OperationSpec {
    name: "profile.import",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Import an exported archive as a new Profile; the name gets a suffix when taken. The archive travels as base64 up to 48 MiB; larger ones use POST /v1/profiles/import/raw (brr profile import does)",
    http: Http {
        method: Method::Post,
        path: "/v1/profiles/import",
    },
    cli: &["profile", "import"],
    cli_positional: &[],
    mcp_tool: Some("profile_import"),
    read_only: false,
    capability: None,
    input_schema: schema::<ImportInput>,
    output_schema: schema::<Imported>,
};

inventory::submit!(OpRegistration {
    spec: &EXPORT,
    handler: export
});
inventory::submit!(OpRegistration {
    spec: &IMPORT,
    handler: import
});

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Destination {
    Download,
    Storage,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportInput {
    profile: String,
    passphrase: TextUpload,
    destination: Option<Destination>,
}

// `file` is only the schema: serializing it would copy the base64 text, so the reply gets the
// string moved in after the rest is built.
#[derive(Serialize, JsonSchema)]
struct ExportOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<FileOut>")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup: Option<Backup>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportInput {
    #[schemars(with = "Upload")]
    file: String,
    passphrase: TextUpload,
    name: Option<String>,
}

fn export(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ExportInput = parse(input)?;
        let passphrase = passphrase(&input.passphrase.0)?;
        let found = ctx
            .app
            .store
            .run(move |c| profile::find(c, &input.profile))
            .await?;

        if input.destination == Some(Destination::Storage) {
            let created = backups::create(&ctx.app, &found, Some(passphrase)).await?;

            return output(&ExportOutput {
                file: None,
                filename: None,
                backup: Some(created),
            });
        }

        let (archive, size_bytes) = backups::export_to_file(&ctx.app, &found, passphrase).await?;
        if size_bytes > MAX_INLINE_FILE_BYTES {
            return Err(invalid(
                "destination",
                &format!(
                    "archive is larger than {} MiB, too big for a JSON reply; use destination storage, or POST /v1/profiles/<profile>/export/raw (`brr profile export`), which streams",
                    MAX_INLINE_FILE_BYTES / BYTES_PER_MIB
                ),
            ));
        }
        let path = archive.path().to_owned();
        let encoded = tokio::task::spawn_blocking(move || {
            let mut encoder = EncoderStringWriter::new(&STANDARD);
            io::copy(&mut File::open(path)?, &mut encoder)?;

            Ok::<_, io::Error>(encoder.into_inner())
        })
        .await
        .map_err(internal)?
        .map_err(internal)?;

        let mut reply = output(&ExportOutput {
            file: None,
            filename: Some(backups::archive_filename(&found)),
            backup: None,
        })?;
        reply[FILE_KEY] = Value::String(encoded);

        Ok(reply)
    }
    .boxed()
}

fn import(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ImportInput = parse(input)?;
        let passphrase = passphrase(&input.passphrase.0)?;

        let upload = Scratch::new(&ctx.app, &format!("upload-{}", brr_core::BackupId::new()))?;
        let (path, encoded) = (upload.path().to_owned(), input.file);
        tokio::task::spawn_blocking(move || {
            let mut decoder = DecoderReader::new(encoded.as_bytes(), &STANDARD);

            io::copy(&mut decoder, &mut File::create(path)?)
        })
        .await
        .map_err(internal)?
        .map_err(|e| match e.kind() {
            io::ErrorKind::InvalidData => invalid("file", &format!("not valid base64: {e}")),
            _ => internal(e),
        })?;

        output(&backups::import_from_file(&ctx.app, upload, passphrase, input.name).await?)
    }
    .boxed()
}
