use std::path::PathBuf;

use anyhow::Context;
use brr_core::{Error, OperationSpec};
use brr_daemon::Boot;
use clap::ArgMatches;
use serde_json::json;

use super::{args, as_client, daemon, files, render};

const PROFILE_EXPORT: &str = "profile.export";
const PROFILE_IMPORT: &str = "profile.import";
const STORAGE: &str = "storage";
const PROFILE: &str = "profile";
const PASSPHRASE: &str = "passphrase";
const DESTINATION: &str = "destination";
const NAME: &str = "name";
const FILE: &str = "file";

// Archives are streamed over the raw routes: the JSON ops would hold the whole file as base64
// and stop at 48 MiB. None when the call is not an archive transfer.
pub async fn run(
    boot: &Boot,
    argv0: &str,
    spec: &OperationSpec,
    matches: &ArgMatches,
    json: bool,
) -> anyhow::Result<Option<()>> {
    let to_storage = matches
        .try_get_one::<String>(DESTINATION)
        .ok()
        .flatten()
        .map(String::as_str)
        == Some(STORAGE);
    if spec.name != PROFILE_IMPORT && (spec.name != PROFILE_EXPORT || to_storage) {
        return Ok(None);
    }

    // Binary output has nowhere to go without -o, so this runs before anything is read or started.
    if spec.name == PROFILE_EXPORT && matches.get_one::<PathBuf>(args::OUTPUT).is_none() {
        return Err(Error::InvalidArgument {
            field: "output".to_string(),
            message: "binary output; pass -o <path>".to_string(),
        }
        .into());
    }

    let passphrase_file = matches
        .get_one::<PathBuf>(PASSPHRASE)
        .context("pass --passphrase <file>")?;
    let passphrase = files::read_input(passphrase_file, true)?;
    let passphrase = passphrase
        .as_str()
        .context("the passphrase file is not text")?;
    let remote = daemon::ensure_daemon(&boot.paths, &boot.settings).await?;
    let label = format!("{argv0} {}", spec.cli.join(" "));

    let output = if spec.name == PROFILE_IMPORT {
        let archive = matches
            .get_one::<PathBuf>(FILE)
            .context("pass --file <archive>")?;
        let name = matches.get_one::<String>(NAME).map(String::as_str);

        as_client(&remote, &label, async |client_id| {
            remote
                .import_archive(archive, passphrase, name, client_id)
                .await
        })
        .await?
    } else {
        let profile = matches
            .get_one::<String>(PROFILE)
            .context("pass a profile")?;
        let out = matches
            .get_one::<PathBuf>(args::OUTPUT)
            .context("pass -o <path>")?;
        let bytes = as_client(&remote, &label, async |client_id| {
            remote
                .export_archive(profile, passphrase, out, client_id)
                .await
        })
        .await?;

        json!({ "path": out, "bytes": bytes })
    };

    render::output(&output, json, None, None)?;

    Ok(Some(()))
}
