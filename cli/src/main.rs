mod animation;
mod armour;
mod chf;
mod common;
mod cryxml;
mod dcb;
mod dds;
mod entity;
mod error;
mod glb;
mod nmc;
mod p4k;
mod riglogic;
mod skin;
mod socpak;
mod ui;
mod wwise;

use clap::{Parser, Subcommand};

/// Log current and peak process RSS at debug level, read from
/// `/proc/self/status` (VmRSS / VmHWM). Sampled at phase boundaries only — the
/// previous per-allocation `TrackingAllocator` taxed every alloc/free with
/// contended atomics on all rayon workers to feed this same debug line.
pub fn log_mem_stats(label: &str) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return;
    };
    let field = |key: &str| {
        status
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<f64>().ok())
            .map(|kb| kb / 1024.0)
    };
    if let (Some(rss), Some(hwm)) = (field("VmRSS:"), field("VmHWM:")) {
        log::debug!("[mem] {label}: rss={rss:.1}MB peak={hwm:.1}MB");
    }
}

/// StarBreaker — Star Citizen data extraction toolkit
#[derive(Parser)]
#[command(name = "starbreaker", version, about)]
struct Cli {
    /// Increase log verbosity (-v = debug, -vv = trace)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// P4k archive operations
    P4k {
        #[command(subcommand)]
        command: p4k::P4kCommand,
    },
    /// DataCore (DCB) operations
    Dcb {
        #[command(subcommand)]
        command: dcb::DcbCommand,
    },
    /// Entity export operations
    Entity {
        #[command(subcommand)]
        command: entity::EntityCommand,
    },
    /// Skin/mesh export operations
    Skin {
        #[command(subcommand)]
        command: skin::SkinCommand,
    },
    /// Object container (socpak) export operations
    Socpak {
        #[command(subcommand)]
        command: socpak::SocpakCommand,
    },
    /// CryXML conversion operations
    Cryxml {
        #[command(subcommand)]
        command: cryxml::CryxmlCommand,
    },
    /// DDS texture operations
    Dds {
        #[command(subcommand)]
        command: dds::DdsCommand,
    },
    /// GLB file inspection
    Glb {
        #[command(subcommand)]
        command: glb::GlbCommand,
    },
    /// Character head file operations
    Chf {
        #[command(subcommand)]
        command: chf::ChfCommand,
    },
    /// Armour and clothing export (single pieces: `entity export`)
    Armour {
        #[command(subcommand)]
        command: armour::ArmourCommand,
    },
    /// Wwise soundbank (BNK/WEM) operations
    Wwise {
        #[command(subcommand)]
        command: wwise::WwiseCommand,
    },
    /// NMC (Node Mesh Combo) chunk inspection from `.cga` / `.cgf` files
    Nmc {
        #[command(subcommand)]
        command: nmc::NmcCommand,
    },
    /// UI render replay helpers
    Ui {
        #[command(subcommand)]
        command: ui::UiCommand,
    },
    /// Animation binding diagnostics (report-only)
    Animation {
        #[command(subcommand)]
        command: animation::AnimationCommand,
    },
    /// RigLogic facial DNA inspection
    Riglogic {
        #[command(subcommand)]
        command: riglogic::RigLogicCommand,
    },
}

fn main() {
    let cli = Cli::parse();

    let env_filter = std::env::var("RUST_LOG").unwrap_or_else(|_| {
        match cli.verbose {
            0 => "warn",
            1 => "debug",
            _ => "trace",
        }
        .to_string()
    });
    let mut logger = env_logger::Builder::new();
    logger.parse_filters(&env_filter);
    // Some upstream SWF assets have header length mismatches that are benign for
    // export and currently emit noisy warnings from the dependency parser.
    // Keep CLI output actionable by suppressing that single warning source.
    if std::env::var("STARBREAKER_ALLOW_SWF_READ_WARN")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        logger.filter_module("swf::read", log::LevelFilter::Error);
    }
    if std::env::var("STARBREAKER_ALLOW_USVG_IMAGE_WARN")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        logger.filter_module("usvg::parser::image", log::LevelFilter::Error);
    }
    logger.init();

    let result = match cli.command {
        Command::P4k { command } => command.run(),
        Command::Dcb { command } => command.run(),
        Command::Entity { command } => command.run(),
        Command::Skin { command } => command.run(),
        Command::Socpak { command } => command.run(),
        Command::Cryxml { command } => command.run(),
        Command::Dds { command } => command.run(),
        Command::Glb { command } => command.run(),
        Command::Chf { command } => command.run(),
        Command::Armour { command } => command.run(),
        Command::Wwise { command } => command.run(),
        Command::Nmc { command } => command.run(),
        Command::Ui { command } => command.run(),
        Command::Animation { command } => command.run(),
        Command::Riglogic { command } => command.run(),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
