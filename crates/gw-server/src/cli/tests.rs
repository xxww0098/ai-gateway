use super::*;

fn parse_of(args: &[&str]) -> Result<Cli, CliError> {
    parse(args.iter().map(|a| (*a).to_owned()))
}

#[test]
fn no_arguments_keeps_the_default_config_path() {
    let cli = parse_of(&[]).expect("empty command line");
    assert_eq!(cli, Cli::default());
    // The default matters: the shipped image runs `gw-server` with no flags.
    assert_eq!(cli.config_path, "config.example.yaml");
}

#[test]
fn config_accepts_every_flag_spelling() {
    for args in [
        vec!["-config", "/etc/gw.yaml"],
        vec!["--config", "/etc/gw.yaml"],
        vec!["-config=/etc/gw.yaml"],
        vec!["--config=/etc/gw.yaml"],
    ] {
        let cli = parse_of(&args).unwrap_or_else(|err| panic!("{args:?}: {err}"));
        assert_eq!(cli.config_path, "/etc/gw.yaml", "{args:?}");
    }
}

#[test]
fn bool_flags_take_no_separate_argument() {
    assert!(parse_of(&["-version"]).expect("version").show_version);
    assert!(
        parse_of(&["--health-check"])
            .expect("health-check")
            .health_check
    );
    assert!(
        !parse_of(&["-version=false"])
            .expect("explicit false")
            .show_version
    );
    // "0"/"1" are accepted by strconv.ParseBool, so they must be accepted here.
    assert!(
        parse_of(&["-health-check=1"])
            .expect("numeric true")
            .health_check
    );
}

#[test]
fn flags_combine() {
    let cli = parse_of(&["-config", "cfg.yaml", "-health-check"]).expect("combined");
    assert_eq!(cli.config_path, "cfg.yaml");
    assert!(cli.health_check);
    assert!(!cli.show_version);
}

#[test]
fn a_value_less_config_flag_is_rejected() {
    assert_eq!(
        parse_of(&["-config"]),
        Err(CliError::MissingValue("config".to_owned()))
    );
}

#[test]
fn unknown_flags_are_rejected_rather_than_ignored() {
    assert_eq!(
        parse_of(&["-nope"]),
        Err(CliError::Unknown("nope".to_owned()))
    );
    // A typo'd config flag must NOT silently fall back to the default path:
    // booting the wrong config is worse than refusing to boot.
    assert!(parse_of(&["-configg=x.yaml"]).is_err());
}

#[test]
fn a_non_boolean_value_is_rejected() {
    assert_eq!(
        parse_of(&["-version=maybe"]),
        Err(CliError::InvalidBool {
            flag: "version".to_owned(),
            value: "maybe".to_owned(),
        })
    );
}

#[test]
fn parsing_stops_at_the_first_positional() {
    let cli = parse_of(&["-config", "a.yaml", "leftover", "-version"]).expect("positional");
    assert_eq!(cli.config_path, "a.yaml");
    assert!(!cli.show_version, "flags after a positional are not parsed");

    let cli = parse_of(&["--", "-version"]).expect("terminator");
    assert!(!cli.show_version);
}

#[test]
fn help_is_requested_by_both_spellings() {
    assert!(parse_of(&["-h"]).expect("-h").show_help);
    assert!(parse_of(&["--help"]).expect("--help").show_help);
}

#[test]
fn error_messages_match_the_flag_package() {
    assert_eq!(
        CliError::MissingValue("config".to_owned()).to_string(),
        "flag needs an argument: -config"
    );
    assert_eq!(
        CliError::Unknown("nope".to_owned()).to_string(),
        "flag provided but not defined: -nope"
    );
}
