use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, error::ErrorKind};

use chnroute::fetch::{FetchOptions, FetchStatus, fetch};

#[derive(Debug, Parser)]
#[command(
    name = "chnroute",
    version,
    about = "Fetch and generate regional and operator route tables",
    color = clap::ColorChoice::Never
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Download and validate upstream source data
    Fetch {
        /// Upstream Git branch, tag, or full commit SHA
        #[arg(long = "ref", default_value = "ip-lists")]
        requested_ref: String,

        /// Directory for downloaded source data
        #[arg(long, default_value = "data/raw")]
        output: PathBuf,

        /// Download again even when the local snapshot is current
        #[arg(long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return print_clap_error(error),
    };

    match run(cli) {
        Ok(message) => {
            print!("{}", ascii_only(&format!("{message}\n")));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprint!("{}", ascii_only(&format!("error: {error:#}\n")));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<String> {
    let message = match cli.command {
        Command::Fetch {
            requested_ref,
            output,
            force,
        } => {
            let result = fetch(FetchOptions {
                requested_ref,
                output,
                force,
            })?;

            match result.status {
                FetchStatus::Updated => format!(
                    "fetched {} files ({} prefixes) from {}@{}",
                    result.file_count,
                    result.prefix_count,
                    result.repository,
                    short_commit(&result.commit)
                ),
                FetchStatus::Unchanged => format!(
                    "already current: {}@{} ({} files)",
                    result.repository,
                    short_commit(&result.commit),
                    result.file_count
                ),
            }
        }
    };

    Ok(message)
}

fn short_commit(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

fn print_clap_error(error: clap::Error) -> ExitCode {
    let use_stdout = matches!(
        error.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    );
    let message = ascii_only(&error.to_string());
    if use_stdout {
        print!("{message}");
    } else {
        eprint!("{message}");
    }

    let code = u8::try_from(error.exit_code()).unwrap_or(1);
    ExitCode::from(code)
}

fn ascii_only(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        if character.is_ascii() {
            output.push(character);
        } else {
            output.extend(character.escape_unicode());
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn help_is_ascii() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.is_ascii(), "help contains non-ASCII text: {help:?}");
    }

    #[test]
    fn dynamic_output_is_escaped_to_ascii() {
        assert_eq!(
            ascii_only("path: /tmp/\u{8def}\u{7531}\n"),
            "path: /tmp/\\u{8def}\\u{7531}\n"
        );
    }

    #[test]
    fn clap_errors_can_be_rendered_as_ascii() {
        let error = Cli::try_parse_from(["chnroute", "\u{751f}\u{6210}"]).unwrap_err();
        assert!(ascii_only(&error.to_string()).is_ascii());
    }
}
