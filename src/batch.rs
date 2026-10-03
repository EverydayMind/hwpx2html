use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use rayon::prelude::*;
use serde::Serialize;

use crate::cli::{BatchArgs, CommonArgs, ConvertArgs, ResourceMode};
use crate::convert::{convert_input, unsupported_count, ConvertOptions, ConvertOutcome};
use crate::diagnostic::Diagnostic;
use crate::error::{ConvertError, Result};
use crate::hwpx::package::{estimate_memory_for_path, MIB};
use crate::render::RenderBundle;

#[derive(Debug, Clone, Serialize)]
pub struct ReportRecord {
    pub input: String,
    pub output: Option<String>,
    pub status: String,
    pub warnings: Vec<String>,
    pub unsupported_objects: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn run_convert(args: &ConvertArgs) -> Result<i32> {
    let result = convert_file(
        &args.input,
        &args.output,
        &args.common,
        args.force,
        args.resource_mode,
    );
    let record = match result {
        Ok(record) => record,
        Err(error) => ReportRecord {
            input: args.input.display().to_string(),
            output: Some(args.output.display().to_string()),
            status: "failed".to_owned(),
            warnings: Vec::new(),
            unsupported_objects: 0,
            error: Some(error.to_string()),
        },
    };
    if let Some(report) = args.common.report.as_deref() {
        write_report(report, std::slice::from_ref(&record))?;
    }
    emit_record(&record, args.common.report.is_some());
    Ok(i32::from(
        record.status == "failed" || (args.common.strict && record.status == "ok_with_warnings"),
    ))
}

pub fn run_batch(args: &BatchArgs) -> Result<i32> {
    if !args.input_dir.is_dir() {
        return Err(ConvertError::MissingInput(args.input_dir.clone()));
    }
    let mut inputs = Vec::new();
    if args.recursive {
        for entry in walkdir::WalkDir::new(&args.input_dir).follow_links(false) {
            let entry = entry.map_err(|error| ConvertError::InvalidValue {
                path: args.input_dir.display().to_string(),
                message: error.to_string(),
            })?;
            if entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("hwpx"))
            {
                inputs.push(entry.into_path());
            }
        }
    } else {
        for entry in std::fs::read_dir(&args.input_dir)? {
            let path = entry?.path();
            if path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("hwpx"))
            {
                inputs.push(path);
            }
        }
    }
    inputs.sort();
    std::fs::create_dir_all(&args.output_dir)?;
    let jobs = args
        .jobs
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(1)
                .min(4)
        })
        .max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .map_err(|error| ConvertError::InvalidValue {
            path: args.output_dir.display().to_string(),
            message: error.to_string(),
        })?;
    let input_dir = Arc::new(args.input_dir.clone());
    let output_dir = Arc::new(args.output_dir.clone());
    let limits = Arc::new(args.common.limits());
    let memory_budget = Arc::new(MemoryBudget::new(limits.memory_budget_bytes));
    let records = pool.install(|| {
        inputs
            .par_iter()
            .map(|input| {
                let relative = input.strip_prefix(input_dir.as_path()).unwrap_or(input);
                let mut output = output_dir.join(relative);
                output.set_extension("html");
                let record = match estimate_memory_for_path(input, &limits)
                    .and_then(|estimated| memory_budget.acquire(estimated))
                {
                    Ok(reservation) => {
                        let result = convert_file(
                            input,
                            &output,
                            &args.common,
                            args.force,
                            args.resource_mode,
                        );
                        drop(reservation);
                        result
                    }
                    Err(error) => Err(error),
                };
                match record {
                    Ok(record) => record,
                    Err(error) => ReportRecord {
                        input: input.display().to_string(),
                        output: Some(output.display().to_string()),
                        status: "failed".to_owned(),
                        warnings: Vec::new(),
                        unsupported_objects: 0,
                        error: Some(error.to_string()),
                    },
                }
            })
            .collect::<Vec<_>>()
    });
    let mut failed = false;
    if let Some(report) = args.common.report.as_deref() {
        write_report(report, &records)?;
    }
    for record in &records {
        if record.status == "failed" || (args.common.strict && record.status == "ok_with_warnings")
        {
            failed = true;
        }
        if args.verbose {
            println!("{}: {}", record.input, record.status);
        }
        emit_record(record, args.common.report.is_some());
    }
    Ok(if failed { 1 } else { 0 })
}

#[derive(Debug)]
struct MemoryBudget {
    limit: u64,
    state: Arc<(Mutex<u64>, Condvar)>,
}

impl MemoryBudget {
    fn new(limit: u64) -> Self {
        Self {
            limit,
            state: Arc::new((Mutex::new(0), Condvar::new())),
        }
    }

    fn acquire(&self, amount: u64) -> Result<MemoryReservation> {
        if amount > self.limit {
            return Err(ConvertError::MemoryBudgetExceeded {
                estimated: amount / MIB,
                budget: self.limit / MIB,
            });
        }
        let (used, signal) = &*self.state;
        let mut used = used.lock().expect("memory budget mutex poisoned");
        while used.saturating_add(amount) > self.limit {
            used = signal
                .wait(used)
                .expect("memory budget mutex poisoned while waiting");
        }
        *used += amount;
        Ok(MemoryReservation {
            budget: Arc::clone(&self.state),
            amount,
        })
    }
}

struct MemoryReservation {
    budget: Arc<(Mutex<u64>, Condvar)>,
    amount: u64,
}

impl Drop for MemoryReservation {
    fn drop(&mut self) {
        let (used, signal) = &*self.budget;
        let mut used = used.lock().expect("memory budget mutex poisoned");
        *used = used.saturating_sub(self.amount);
        signal.notify_all();
    }
}

fn convert_file(
    input: &Path,
    output: &Path,
    common: &CommonArgs,
    force: bool,
    resource_mode: ResourceMode,
) -> Result<ReportRecord> {
    if output.exists() && !force {
        return Err(ConvertError::OutputExists(output.to_path_buf()));
    }
    let directory_name = format!(
        "{}.assets",
        output.file_name().unwrap_or_default().to_string_lossy()
    );
    let options = ConvertOptions {
        render: common.render_options(),
        limits: common.limits(),
        strict: common.strict,
    };
    let bytes = crate::hwpx::package::read_input(input, &options.limits)?;
    let conversion = match convert_input(
        input,
        bytes,
        &options,
        &directory_name,
        common.no_logical_dom,
    )? {
        ConvertOutcome::Converted(conversion) => conversion,
        ConvertOutcome::Rejected {
            diagnostics,
            unsupported_objects,
        } => {
            return Ok(ReportRecord {
                input: input.display().to_string(),
                output: Some(output.display().to_string()),
                status: "failed".to_owned(),
                warnings: diagnostics
                    .iter()
                    .filter_map(Diagnostic::legacy_text)
                    .map(str::to_owned)
                    .collect(),
                unsupported_objects,
                error: Some(ConvertError::StrictUnsupported.to_string()),
            });
        }
    };
    let unsupported_objects = conversion.summary.unsupported_objects;
    let warnings = conversion
        .diagnostics
        .iter()
        .filter_map(Diagnostic::legacy_text)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let bundle = conversion.bundle;
    match resource_mode {
        ResourceMode::External => {
            write_resources(&output.with_file_name(directory_name), &bundle)?;
            atomic_write(output, bundle.html.as_bytes(), force)?;
        }
        ResourceMode::Embedded => {
            atomic_write(output, bundle.to_single_html().as_bytes(), force)?;
        }
    }
    Ok(ReportRecord {
        input: input.display().to_string(),
        output: Some(output.display().to_string()),
        status: if unsupported_objects > 0 || !warnings.is_empty() {
            "ok_with_warnings".to_owned()
        } else {
            "ok".to_owned()
        },
        warnings,
        unsupported_objects,
        error: None,
    })
}

// Content-addressed files are immutable: existing identical resources can be
// reused, but even --force must not overwrite a conflicting resource. Publish
// resources before HTML so a failed resource write leaves the old HTML intact.
// Old resources are intentionally retained when replacing an external output.
fn write_resources(directory: &Path, bundle: &RenderBundle) -> Result<()> {
    for (name, resource) in &bundle.resources {
        let path = directory.join(name);
        if path.exists() && std::fs::read(&path)? != resource.data {
            return Err(ConvertError::OutputExists(path));
        }
    }
    std::fs::create_dir_all(directory)?;
    for (name, resource) in &bundle.resources {
        let path = directory.join(name);
        if !path.exists() {
            atomic_write(&path, &resource.data, false)?;
        }
    }
    Ok(())
}

pub fn run_inspect(args: &crate::cli::InspectArgs) -> Result<i32> {
    if args.output.exists() && !args.force {
        return Err(ConvertError::OutputExists(args.output.clone()));
    }
    let document = crate::read_document(&args.input, args.common.limits())?;
    let unsupported_objects = unsupported_count(&document);
    let warnings = document.warnings.clone();
    if args.common.strict && unsupported_objects > 0 {
        let record = ReportRecord {
            input: args.input.display().to_string(),
            output: Some(args.output.display().to_string()),
            status: "failed".to_owned(),
            warnings,
            unsupported_objects,
            error: Some(ConvertError::StrictUnsupported.to_string()),
        };
        if let Some(report) = args.common.report.as_deref() {
            write_report(report, std::slice::from_ref(&record))?;
        }
        emit_record(&record, args.common.report.is_some());
        return Ok(1);
    }
    let manifest = crate::manifest::layout_manifest(&document);
    let text = serde_json::to_vec_pretty(&manifest)?;
    atomic_write(&args.output, &text, args.force)?;
    let record = ReportRecord {
        input: args.input.display().to_string(),
        output: Some(args.output.display().to_string()),
        status: if unsupported_objects > 0 || !warnings.is_empty() {
            "ok_with_warnings".to_owned()
        } else {
            "ok".to_owned()
        },
        warnings,
        unsupported_objects,
        error: None,
    };
    if let Some(report) = args.common.report.as_deref() {
        write_report(report, std::slice::from_ref(&record))?;
    }
    emit_record(&record, args.common.report.is_some());
    Ok(i32::from(record.status == "failed"))
}

fn atomic_write(path: &Path, bytes: &[u8], force: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
    }
    if force && path.exists() {
        std::fs::remove_file(path)?;
    }
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn emit_record(record: &ReportRecord, report_enabled: bool) {
    if report_enabled {
        if let Some(error) = &record.error {
            eprintln!("error: {}: {}", record.input, error);
        }
    } else if record.status == "ok_with_warnings" {
        eprintln!(
            "warning: {}: {} unsupported object(s) or fallback warning(s)",
            record.input,
            record.unsupported_objects.max(record.warnings.len())
        );
        for warning in record.warnings.iter().take(10) {
            eprintln!("  - {warning}");
        }
        if record.warnings.len() > 10 {
            eprintln!("  - ... and {} more", record.warnings.len() - 10);
        }
    } else if let Some(error) = &record.error {
        eprintln!("error: {}: {}", record.input, error);
    }
}

fn write_report(path: &Path, records: &[ReportRecord]) -> Result<()> {
    let text = records
        .iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    atomic_write(path, format!("{text}\n").as_bytes(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_budget_rejects_a_document_larger_than_the_budget() {
        let budget = MemoryBudget::new(2 * MIB);
        assert!(matches!(
            budget.acquire(3 * MIB),
            Err(ConvertError::MemoryBudgetExceeded {
                estimated: 3,
                budget: 2
            })
        ));
    }

    #[test]
    fn memory_budget_releases_a_completed_reservation() {
        let budget = MemoryBudget::new(2 * MIB);
        let reservation = budget.acquire(2 * MIB).expect("initial reservation");
        drop(reservation);
        assert!(budget.acquire(2 * MIB).is_ok());
    }
}
