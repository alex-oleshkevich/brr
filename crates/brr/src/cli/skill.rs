use std::{env, fs, path::PathBuf};

use anyhow::Context;
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};

use super::format;

pub const NAME: &str = "skill";

const SKILL: &str = include_str!("../../../../skills/brr/SKILL.md");
const INSTALL: &str = "install";
const DIR: &str = "dir";
const SKILL_DIR: &str = "brr";
const SKILL_FILE: &str = "SKILL.md";
// Claude Code reads the first; Codex and most other agents read the second.
const DEFAULT_DIRS: [&str; 2] = [".claude/skills", ".agents/skills"];

pub fn command() -> Command {
    Command::new(NAME)
        .about("Print the brr agent skill, or install it for your coding agents")
        .subcommand(
            Command::new(INSTALL)
                .about("Write the skill to ~/.claude/skills/brr and ~/.agents/skills/brr")
                .arg(
                    Arg::new(DIR)
                        .long(DIR)
                        .action(ArgAction::Append)
                        .value_parser(value_parser!(PathBuf))
                        .help("Skills folder to install into instead; repeat for several"),
                ),
        )
}

pub fn run(matches: &ArgMatches) -> anyhow::Result<()> {
    let Some((INSTALL, install)) = matches.subcommand() else {
        return Ok(format::data(SKILL)?);
    };

    let dirs: Vec<PathBuf> = match install.get_many::<PathBuf>(DIR) {
        Some(dirs) => dirs.cloned().collect(),
        None => {
            let home = env::home_dir().context("HOME is not set; pass --dir <skills folder>")?;
            DEFAULT_DIRS.iter().map(|dir| home.join(dir)).collect()
        }
    };

    for dir in dirs {
        let dir = dir.join(SKILL_DIR);
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

        let file = dir.join(SKILL_FILE);
        fs::write(&file, SKILL).with_context(|| format!("writing {}", file.display()))?;
        format::status("Installed", &file.display().to_string())?;
    }

    Ok(())
}
