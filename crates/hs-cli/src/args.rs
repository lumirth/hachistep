//! CLI declarations, defaults and help share one definition.
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "hachistep",
    version,
    about = "Deterministic Pokéwalker emulator core",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Subcommand)]
pub enum Command {
    /// Inspect input identities without advancing hardware.
    Inspect(Options),
    /// Execute to an exclusive absolute emulated horizon.
    Run(Options),
}
#[derive(Args)]
pub struct Options {
    #[arg(
        long,
        required_unless_present = "load_state",
        conflicts_with = "load_state"
    )]
    pub firmware: Option<PathBuf>,
    #[arg(
        long,
        required_unless_present = "load_state",
        conflicts_with = "load_state"
    )]
    pub eeprom: Option<PathBuf>,
    /// Restore the full session, including flash and all nonvolatile contents.
    #[arg(long, conflicts_with_all = ["sensor_nv", "status", "supply_mv", "avcc_mv", "battery_drop_mv"])]
    pub load_state: Option<PathBuf>,
    /// Write the final state to a new file. Never overwrites an existing file.
    #[arg(long)]
    pub save_state: Option<PathBuf>,
    /// Exclusive absolute horizon, in milliseconds from device time zero.
    #[arg(long, default_value_t = 1000)]
    pub milliseconds: u64,
    /// Physical input CSV, with absolute device timestamps.
    #[arg(long)]
    pub input: Option<PathBuf>,
    /// New directory for display, persistent images, state and report.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// New binary PGM screenshot.
    #[arg(long)]
    pub frame: Option<PathBuf>,
    /// New product-event trace.
    #[arg(long)]
    pub trace: Option<PathBuf>,
    /// Include bus effects; requires a build with the trace feature.
    #[arg(long)]
    pub bus_trace: bool,
    #[arg(long, default_value_t = 100000)]
    pub trace_limit: u64,
    /// Optional 19-byte sensor nonvolatile image.
    #[arg(long)]
    pub sensor_nv: Option<PathBuf>,
    /// EEPROM persistent status, decimal or 0x hexadecimal.
    #[arg(long, default_value = "0", value_parser = status)]
    pub status: u8,
    #[arg(long, default_value_t = 3000)]
    pub supply_mv: u16,
    /// External AVCC fixture; defaults to the board supply.
    #[arg(long)]
    pub avcc_mv: Option<u16>,
    #[arg(long, default_value_t = 600)]
    pub battery_drop_mv: u16,
    /// Host-call partition; does not change emulation fidelity.
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(1..))]
    pub chunk_us: u64,
    /// Side-effect-free final inspection at a hexadecimal address; repeatable.
    #[arg(long, value_parser = address)]
    pub peek: Vec<u16>,
}
fn status(text: &str) -> Result<u8, std::num::ParseIntError> {
    text.strip_prefix("0x")
        .map_or_else(|| text.parse(), |hex| u8::from_str_radix(hex, 16))
}
fn address(text: &str) -> Result<u16, std::num::ParseIntError> {
    u16::from_str_radix(text.trim_start_matches("0x"), 16)
}
