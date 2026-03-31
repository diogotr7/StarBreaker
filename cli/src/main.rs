mod chf;
mod common;
mod cryxml;
mod dcb;
mod dds;
mod entity;
mod error;
mod glb;
mod p4k;
mod skin;
mod socpak;
mod wwise;

use clap::{Parser, Subcommand};

// ── Tracking allocator ──────────────────────────────────────────────────────
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

pub struct TrackingAllocator;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static CAP: AtomicUsize = AtomicUsize::new(0); // 0 = no cap

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let cap = CAP.load(Relaxed);
        if cap > 0 {
            let current = ALLOCATED.load(Relaxed);
            if current + layout.size() > cap {
                eprintln!(
                    "\n[mem] ABORT: allocation would exceed cap ({:.0}MB). Current={:.0}MB, peak={:.0}MB",
                    cap as f64 / 1_048_576.0,
                    current as f64 / 1_048_576.0,
                    PEAK.load(Relaxed) as f64 / 1_048_576.0,
                );
                std::process::exit(137);
            }
        }
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let current = ALLOCATED.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(current, Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        ALLOCATED.fetch_sub(layout.size(), Relaxed);
    }
}

#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator;

/// Set the memory cap in bytes. 0 = no cap.
pub fn set_mem_cap(bytes: usize) {
    CAP.store(bytes, Relaxed);
}

/// Log current and peak memory usage at debug level.
pub fn log_mem_stats(label: &str) {
    let current = ALLOCATED.load(Relaxed);
    let peak = PEAK.load(Relaxed);
    log::debug!(
        "[mem] {label}: current={:.1}MB peak={:.1}MB",
        current as f64 / 1_048_576.0,
        peak as f64 / 1_048_576.0,
    );
}

/// StarBreaker — Star Citizen data extraction toolkit
#[derive(Parser)]
#[command(name = "starbreaker", version, about)]
struct Cli {
    /// Memory cap in MB — abort if exceeded (0 = unlimited)
    #[arg(long, global = true, default_value = "0")]
    mem_cap: usize,
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
    /// Wwise soundbank (BNK/WEM) operations
    Wwise {
        #[command(subcommand)]
        command: wwise::WwiseCommand,
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
    env_logger::Builder::new()
        .parse_filters(&env_filter)
        .init();

    if cli.mem_cap > 0 {
        set_mem_cap(cli.mem_cap * 1_048_576);
        log::debug!("[mem] cap set to {}MB", cli.mem_cap);
    }

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
        Command::Wwise { command } => command.run(),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
