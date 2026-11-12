//! CLI recovery operations use the existing authenticated control transport.
use super::{Output, send};
use crate::control::{ControlCommand, recovery::RecoveryCommand as Request};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum Command {
    List {
        #[command(flatten)]
        output: Output,
    },
    Text {
        id: String,
        #[arg(long)]
        raw: bool,
        #[command(flatten)]
        output: Output,
    },
    Retry {
        id: String,
        #[arg(long)]
        current_model: bool,
        #[command(flatten)]
        output: Output,
    },
    ExportAudio {
        id: String,
        path: std::path::PathBuf,
        #[command(flatten)]
        output: Output,
    },
    ExportText {
        id: String,
        path: std::path::PathBuf,
        #[arg(long)]
        raw: bool,
        #[command(flatten)]
        output: Output,
    },
    Resolve {
        id: String,
        #[arg(long)]
        keep: bool,
        #[command(flatten)]
        output: Output,
    },
    Delete {
        id: String,
        /// Required confirmation: permanently delete this safety copy.
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        output: Output,
    },
}
pub fn run(command: Command) -> u8 {
    match request(command) {
        Ok((operation, output)) => send(ControlCommand::Recovery { operation }, output),
        Err(error) => super::fail(&error),
    }
}

fn request(command: Command) -> Result<(Request, Output), String> {
    let absolute = |path: std::path::PathBuf| {
        std::path::absolute(path).map_err(|error| format!("could not resolve export path: {error}"))
    };
    Ok(match command {
        Command::List { output } => (Request::List, output),
        Command::Text { id, raw, output } => (Request::Text { id, raw }, output),
        Command::Retry {
            id,
            current_model,
            output,
        } => (Request::Retry { id, current_model }, output),
        Command::ExportAudio { id, path, output } => (
            Request::ExportAudio {
                id,
                path: absolute(path)?,
            },
            output,
        ),
        Command::ExportText {
            id,
            path,
            raw,
            output,
        } => (
            Request::ExportText {
                id,
                path: absolute(path)?,
                raw,
            },
            output,
        ),
        Command::Resolve { id, keep, output } => (Request::Resolve { id, keep }, output),
        Command::Delete { yes: false, .. } => {
            return Err("repeat with --yes to delete this recording's local safety copy".into());
        }
        Command::Delete {
            id,
            yes: true,
            output,
        } => (Request::Delete { id }, output),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_exports_send_paths_relative_to_the_invoking_shell() {
        let relative = std::path::PathBuf::from("./not-created/saved.wav");
        let expected = std::env::current_dir()
            .unwrap()
            .join("not-created/saved.wav");
        for command in [
            Command::ExportAudio {
                id: "synthetic".into(),
                path: relative.clone(),
                output: Output::default(),
            },
            Command::ExportText {
                id: "synthetic".into(),
                path: relative.clone(),
                raw: true,
                output: Output::default(),
            },
        ] {
            let path = match request(command).unwrap().0 {
                Request::ExportAudio { path, .. } | Request::ExportText { path, .. } => path,
                _ => unreachable!(),
            };
            assert!(path.is_absolute());
            assert_eq!(path, expected);
        }
    }

    #[test]
    fn absolute_export_destinations_are_preserved() {
        let path = std::env::temp_dir().join("hex-synthetic-export.txt");
        let (Request::ExportText { path: sent, .. }, _) = request(Command::ExportText {
            id: "synthetic".into(),
            path: path.clone(),
            raw: false,
            output: Output::default(),
        })
        .unwrap() else {
            unreachable!()
        };
        assert_eq!(sent, path);
    }

    #[test]
    fn deletion_still_requires_explicit_confirmation_before_transport() {
        assert!(
            request(Command::Delete {
                id: "synthetic".into(),
                yes: false,
                output: Output::default()
            })
            .is_err()
        );
    }
}
