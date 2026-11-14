use clap::error::ErrorKind;

use super::command;
use brr_daemon::ops;

#[test]
fn tree_covers_registry() {
    let root = command();

    for op in ops::all().into_iter().filter(|op| !op.spec.cli.is_empty()) {
        let leaf = op
            .spec
            .cli
            .iter()
            .try_fold(&root, |parent, word| parent.find_subcommand(word))
            .unwrap_or_else(|| panic!("{} has no command {:?}", op.spec.name, op.spec.cli));

        let positionals: Vec<&str> = leaf
            .get_positionals()
            .map(|arg| arg.get_id().as_str())
            .collect();
        // A map-shaped input adds its key/value positionals after the named ones.
        assert!(
            positionals.starts_with(op.spec.cli_positional),
            "{}: {positionals:?}",
            op.spec.name
        );

        let required: Vec<bool> = leaf
            .get_positionals()
            .map(|arg| arg.is_required_set())
            .collect();
        assert!(
            required.is_sorted_by(|before, after| before >= after),
            "{}: optional positionals must follow required ones",
            op.spec.name
        );
    }
}

#[test]
fn profile_launch_help_exits_zero() {
    let error = command()
        .try_get_matches_from(["brr", "profile", "launch", "--help"])
        .expect_err("help is not a match");

    assert_eq!(error.kind(), ErrorKind::DisplayHelp);
    assert_eq!(error.exit_code(), 0);
}

#[test]
fn events_runs_alone_and_as_a_parent() {
    let alone = command()
        .try_get_matches_from(["brr", "events", "--follow", "-v"])
        .expect("events alone");
    let child = command()
        .try_get_matches_from(["brr", "events", "watch", "-vv", "--json"])
        .expect("events watch");

    assert_eq!(alone.get_count(super::VERBOSE), 1);
    assert_eq!(child.get_count(super::VERBOSE), 2);
    assert!(child.get_flag(super::JSON));
}
