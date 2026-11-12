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
    let (command, output) = match command {
        Command::List { output } => (Request::List, output),
        Command::Text { id, raw, output } => (Request::Text { id, raw }, output),
        Command::Retry {
            id,
            current_model,
            output,
        } => (Request::Retry { id, current_model }, output),
        Command::ExportAudio { id, path, output } => (Request::ExportAudio { id, path }, output),
        Command::ExportText {
            id,
            path,
            raw,
            output,
        } => (Request::ExportText { id, path, raw }, output),
        Command::Resolve { id, keep, output } => (Request::Resolve { id, keep }, output),
        Command::Delete { yes: false, .. } => {
            return super::fail("repeat with --yes to delete this recording's local safety copy");
        }
        Command::Delete {
            id,
            yes: true,
            output,
        } => (Request::Delete { id }, output),
    };
    send(ControlCommand::Recovery { operation: command }, output)
}
