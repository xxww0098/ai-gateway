//! Command-line flags.
//!
//! Hand-rolled rather than `clap`: three flags do not justify a dependency, and
//! the exact spellings (`-config x`, `-config=x`, `--config=x`, bare
//! `-version`) are what the shipped Dockerfile / systemd units already use.

use gw_config::parse_loose_bool;

/// The application version string.
pub const APP_VERSION: &str = "0.1.0";

/// The default config path.
pub const DEFAULT_CONFIG_PATH: &str = "config.example.yaml";

/// Flag documentation, printed on `-h` or a malformed command line.
pub const USAGE: &str = "\
Usage of gw-server:
  -config string
    \tpath to YAML config file (default \"config.example.yaml\")
  -health-check
    \tprobe /api/health/ready on localhost and exit 0 (ready) / 1 (not); for
    \tcontainer HEALTHCHECK on the shell-less distroless image
  -version
    \tprint version and exit";

/// The parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub config_path: String,
    pub show_version: bool,
    pub health_check: bool,
    pub show_help: bool,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            config_path: DEFAULT_CONFIG_PATH.to_owned(),
            show_version: false,
            health_check: false,
            show_help: false,
        }
    }
}

/// Why a command line was rejected. Messages keep the familiar flag-package
/// spellings so operators see the same text they see today.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CliError {
    #[error("flag needs an argument: -{0}")]
    MissingValue(String),
    #[error("flag provided but not defined: -{0}")]
    Unknown(String),
    #[error("invalid boolean value {value:?} for -{flag}")]
    InvalidBool { flag: String, value: String },
}

/// Parse arguments **without** the program name (`std::env::args().skip(1)`).
///
/// The first non-flag argument ends flag parsing and the remainder is
/// ignored.
pub fn parse<I>(args: I) -> Result<Cli, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut cli = Cli::default();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        // "--" terminates flags; a bare "-" or any positional is not a flag.
        if arg == "--" {
            break;
        }
        let Some(rest) = arg.strip_prefix('-') else {
            break;
        };
        let rest = rest.strip_prefix('-').unwrap_or(rest);
        if rest.is_empty() {
            break;
        }

        let (name, value) = match rest.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (rest, None),
        };

        match name {
            "config" => {
                cli.config_path = match value {
                    Some(value) => value,
                    None => args
                        .next()
                        .ok_or_else(|| CliError::MissingValue(name.to_owned()))?,
                };
            }
            "version" => cli.show_version = bool_flag(name, value)?,
            "health-check" => cli.health_check = bool_flag(name, value)?,
            "h" | "help" => cli.show_help = true,
            other => return Err(CliError::Unknown(other.to_owned())),
        }
    }

    Ok(cli)
}

/// Bool flags take no separate argument: `-version` is true, `-version=0`
/// is false, and `-version false` leaves `false` as a positional.
fn bool_flag(name: &str, value: Option<String>) -> Result<bool, CliError> {
    match value {
        None => Ok(true),
        Some(value) => parse_loose_bool(&value).ok_or(CliError::InvalidBool {
            flag: name.to_owned(),
            value,
        }),
    }
}

#[cfg(test)]
mod tests;
