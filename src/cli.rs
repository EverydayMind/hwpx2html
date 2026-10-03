use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::hwpx::package::{Limits, MIB};
use crate::render::{RenderOptions, UnsupportedPolicy};

#[derive(Debug, Parser)]
#[command(
    name = "hwpx2html",
    version,
    about = "Convert HWPX documents to HTML with external or embedded resources"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Convert one HWPX file to HTML")]
    Convert(ConvertArgs),
    #[command(about = "Convert HWPX files in a directory")]
    Batch(BatchArgs),
    #[command(about = "Inspect a HWPX file and write a page manifest")]
    Inspect(InspectArgs),
}

#[derive(Debug, Clone, Args)]
pub struct CommonArgs {
    #[arg(long, value_enum, default_value_t = UnsupportedArg::Placeholder, help = "Fallback policy for unsupported objects (the page-by-page renderer only: the default writer leaves such an object out and names it in the warnings)")]
    pub on_unsupported: UnsupportedArg,
    #[arg(
        long,
        help = "Reject a document if it contains unsupported objects or parts the writer would leave out"
    )]
    pub strict: bool,
    #[arg(
        long,
        help = "Omit the inline correction script (half-em spaces, line fitting and justification)"
    )]
    pub no_adjust_letter_spacing: bool,
    #[arg(
        long,
        help = "Show every page at once instead of one page at a time with a navigation bar"
    )]
    pub no_page_navigation: bool,
    #[arg(
        long,
        help = "Do not tag headings and lists implied by paragraph markers such as 제1장, □ and ○ (tagged by default, marked data-inferred)"
    )]
    pub no_infer_structure: bool,
    #[arg(
        long,
        help = "Diagnostic: write the page-by-page renderer's output (a paragraph or table that crosses pages once per page, linked by data-hwpx-id/data-hwpx-part) instead of the default one-element-per-paragraph document"
    )]
    pub no_logical_dom: bool,
    #[arg(
        long,
        default_value_t = 64,
        help = "Maximum compressed input size in MiB"
    )]
    pub max_input_mib: u64,
    #[arg(
        long,
        default_value_t = 256,
        help = "Maximum total uncompressed ZIP size in MiB"
    )]
    pub max_unpacked_mib: u64,
    #[arg(
        long,
        default_value_t = 64,
        help = "Maximum individual ZIP entry size in MiB"
    )]
    pub max_entry_mib: u64,
    #[arg(long, default_value_t = 4096, help = "Maximum ZIP entry count")]
    pub max_entries: usize,
    #[arg(long, default_value_t = 512, help = "Shared memory budget in MiB")]
    pub memory_budget_mib: u64,
    #[arg(long, help = "Write one JSONL result record per input")]
    pub report: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct ConvertArgs {
    #[arg(long, help = "Input HWPX file")]
    pub input: PathBuf,
    #[arg(long, help = "Output HTML file")]
    pub output: PathBuf,
    #[arg(long, help = "Replace an existing output")]
    pub force: bool,
    #[arg(long, value_enum, default_value_t = ResourceMode::External, help = "External resources (comparison default) or a single embedded HTML file")]
    pub resource_mode: ResourceMode,
    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Clone, Args)]
pub struct BatchArgs {
    #[arg(long = "input-dir", help = "Input directory")]
    pub input_dir: PathBuf,
    #[arg(long = "output-dir", help = "Output directory")]
    pub output_dir: PathBuf,
    #[arg(long, help = "Scan subdirectories and preserve their structure")]
    pub recursive: bool,
    #[arg(long, help = "Maximum concurrent conversions")]
    pub jobs: Option<usize>,
    #[arg(long, help = "Replace existing outputs")]
    pub force: bool,
    #[arg(long, help = "Print per-file progress")]
    pub verbose: bool,
    #[arg(long, value_enum, default_value_t = ResourceMode::External, help = "External resources (comparison default) or a single embedded HTML file")]
    pub resource_mode: ResourceMode,
    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Clone, Args)]
pub struct InspectArgs {
    #[arg(long, help = "Input HWPX file")]
    pub input: PathBuf,
    #[arg(long, help = "Output manifest JSON")]
    pub output: PathBuf,
    #[arg(long, help = "Replace an existing manifest")]
    pub force: bool,
    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum UnsupportedArg {
    Placeholder,
    Skip,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum ResourceMode {
    External,
    Embedded,
}

impl From<UnsupportedArg> for UnsupportedPolicy {
    fn from(value: UnsupportedArg) -> Self {
        match value {
            UnsupportedArg::Placeholder => UnsupportedPolicy::Placeholder,
            UnsupportedArg::Skip => UnsupportedPolicy::Skip,
        }
    }
}

impl CommonArgs {
    pub fn limits(&self) -> Limits {
        Limits {
            max_input_bytes: self.max_input_mib.saturating_mul(MIB),
            max_unpacked_bytes: self.max_unpacked_mib.saturating_mul(MIB),
            max_entry_bytes: self.max_entry_mib.saturating_mul(MIB),
            max_entries: self.max_entries,
            memory_budget_bytes: self.memory_budget_mib.saturating_mul(MIB),
        }
    }

    pub fn render_options(&self) -> RenderOptions {
        RenderOptions {
            unsupported: self.on_unsupported.into(),
            adjust_letter_spacing: !self.no_adjust_letter_spacing,
            page_navigation: !self.no_page_navigation,
            infer_structure: !self.no_infer_structure,
            logical_dom: !self.no_logical_dom,
            source_name: None,
        }
    }
}
