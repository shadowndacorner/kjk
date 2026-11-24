//! A standalone, native updater for published KJK Windows and Linux builds.
mod archive;
mod github;
mod profile;
mod release;

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, process::ExitCode};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Channel {
    Stable,
    Dev,
}

impl Channel {
    fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Dev => "dev",
        }
    }
}

#[derive(Parser)]
#[command(about = "Install published KJK builds from a trusted public GitHub repository")]
struct Args {
    /// Trusted public GitHub owner/repository.
    #[arg(long, default_value = "shadowndacorner/kjk")]
    repo: String,
    #[arg(long, value_enum, default_value = "stable")]
    channel: Channel,
    /// Exact source branch (main for stable, develop for dev).
    #[arg(long)]
    branch: Option<String>,
    /// Exact published release tag; permits an intentional older/divergent build.
    #[arg(long, conflicts_with = "tag_pattern")]
    tag: Option<String>,
    /// Tag glob (v* for stable, dev-* for dev).
    #[arg(long)]
    tag_pattern: Option<String>,
    /// Managed data parent, separate from existing game data.
    #[arg(long)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report the newest eligible published release without installing.
    Check,
    /// Install and select a verified release.
    Update,
    /// Run the installed version; offline unless --update.
    Launch {
        /// Require a successful update before launching.
        #[arg(long)]
        update: bool,
        /// Arguments passed directly to the game, following --.
        #[arg(last = true)]
        game_args: Vec<std::ffi::OsString>,
    },
    /// Swap current and previous binaries; preserves saves and settings.
    Rollback,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Scope {
    // Order is the original updater's sorted JSON identity.
    branch: String,
    channel: Channel,
    repository: String,
}

struct Selection {
    scope: Scope,
    tag: Option<String>,
    pattern: glob::Pattern,
}

impl Selection {
    fn from_args(args: &Args) -> Result<Self> {
        let repo = args.repo.to_ascii_lowercase();
        require(release::valid_repo(&repo), "invalid repository")?;
        let branch = args.branch.clone().unwrap_or_else(|| {
            if args.channel == Channel::Stable {
                "main"
            } else {
                "develop"
            }
            .to_string()
        });
        require(
            (1..=255).contains(&branch.chars().count())
                && branch.chars().all(|c| c >= ' ')
                && (args.channel != Channel::Stable || branch == "main"),
            "invalid branch for channel",
        )?;
        require(
            args.tag.as_deref().is_none_or(release::valid_tag),
            "invalid tag",
        )?;
        let pattern = args
            .tag_pattern
            .as_deref()
            .unwrap_or(if args.channel == Channel::Stable {
                "v*"
            } else {
                "dev-*"
            });
        Ok(Self {
            scope: Scope {
                branch,
                channel: args.channel,
                repository: repo,
            },
            tag: args.tag.clone(),
            pattern: glob::Pattern::new(pattern)?,
        })
    }
}

fn platform_name() -> Result<&'static str> {
    require(
        std::env::consts::ARCH == "x86_64",
        "only x86-64 devices are supported",
    )?;
    match std::env::consts::OS {
        "windows" => Ok("windows-x86_64"),
        "linux" => Ok("linux-x86_64"),
        _ => Err("this updater supports Windows and Linux".into()),
    }
}

fn run(args: Args) -> Result<u8> {
    let selection = Selection::from_args(&args)?;
    let system = platform_name()?;
    if matches!(args.command, Command::Check) {
        let value = release::select(&selection)?;
        println!(
            "Available {} ({}) for {} {}/{}",
            value.tag,
            value.commit,
            selection.scope.repository,
            selection.scope.channel.name(),
            selection.scope.branch
        );
        return Ok(0);
    }
    profile::run(&args, &selection, system)
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("kjk-update: {error}");
            ExitCode::FAILURE
        }
    }
}
