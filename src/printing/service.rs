//! Printer discovery, queue selection, and document submission over IPP.

use std::time::Duration;

use ipp::{
    operation::{IppOperation, builder::IppOperationBuilder},
    prelude::{
        AsyncIppClient, DelimiterTag, IppAttribute, IppRequestResponse, IppValue, IppVersion,
        Operation, Uri,
    },
};
use tokio::{task::JoinSet, time::timeout};
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::config::Config;

use super::types::{PrintDocument, PrintJob, PrintJobState, Printer, PrinterState};

const PRINTER_ATTRIBUTES: &[&str] = &[
    "printer-name",
    "printer-uri-supported",
    "device-uri",
    "printer-location",
    "printer-info",
    "printer-make-and-model",
    "printer-state",
    "printer-state-message",
    "printer-state-reasons",
    "printer-is-accepting-jobs",
    "printer-is-shared",
    "printer-is-default",
    "queued-job-count",
    "color-supported",
    "sides-supported",
    "media-supported",
    "printer-resolution-supported",
];

/// Bound parallel IPP requests so a large queue set cannot exhaust sockets or
/// overwhelm the configured IPP server during a refresh.
const MAX_CONCURRENT_PRINTER_REQUESTS: usize = 8;

#[derive(Debug)]
pub struct PrintService {
    base_uri: Uri,
    timeout: Duration,
    pub printers: Vec<Printer>,
    pub discovered_printers: Vec<Printer>,
}

impl PrintService {
    pub fn new(config: &Config) -> Result<Self, String> {
        let scheme = if config.cups_encryption_required {
            "ipps"
        } else {
            "ipp"
        };
        let host = config.cups_server.as_deref().unwrap_or("localhost:631");
        let address = if host.contains("://") {
            host.trim_end_matches('/').to_owned()
        } else {
            format!("{scheme}://{host}/")
        };
        let base_uri: Uri = address
            .parse()
            .map_err(|error| format!("Invalid cups_server '{address}': {error}"))?;
        Ok(Self {
            base_uri,
            timeout: Duration::from_secs(10),
            printers: Vec::new(),
            discovered_printers: Vec::new(),
        })
    }

    fn client(&self, uri: Uri) -> AsyncIppClient {
        AsyncIppClient::builder(uri)
            .request_timeout(self.timeout)
            .build()
    }

    pub async fn get_printers(&mut self) -> Result<&[Printer], String> {
        let response = timeout(
            self.timeout,
            self.client(self.base_uri.clone())
                .send(IppOperationBuilder::cups().get_printers()),
        )
        .await
        .map_err(|_| "CUPS Get-Printers timed out".to_owned())?
        .map_err(|error| format!("CUPS Get-Printers failed: {error}"))?;
        ensure_success(&response, "CUPS Get-Printers")?;
        let mut printers = Vec::new();
        for group in response.attributes().groups() {
            let Some(name) = string_attribute(group, "printer-name") else {
                continue;
            };
            let mut printer = printer_from_attributes(group, name);
            if printer.uri.is_none() {
                printer.uri = Some(queue_uri(&self.base_uri, &printer.name));
            }
            printer.id = printers.len() + 1;
            printers.push(printer);
        }
        self.printers = printers;
        log::debug!("Loaded {} CUPS destination(s)", self.printers.len());
        Ok(&self.printers)
    }

    /// Ask the CUPS server to enumerate available devices. The maximum wait is
    /// five seconds, matching the C project’s discovery bound.
    pub async fn discover_printers(&mut self, wait: Duration) -> Result<&[Printer], String> {
        let wait = wait.min(Duration::from_secs(5));
        let seconds = wait.as_secs() as i32;
        let mut request = IppRequestResponse::new(
            IppVersion::v1_1(),
            Operation::CupsGetDevices,
            Some(self.base_uri.clone()),
        )
        .map_err(|error| error.to_string())?;
        request.attributes_mut().add(
            DelimiterTag::OperationAttributes,
            IppAttribute::with_name("timeout", IppValue::Integer(seconds))
                .map_err(|error| error.to_string())?,
        );
        request.attributes_mut().add(
            DelimiterTag::OperationAttributes,
            IppAttribute::with_name(
                "requested-attributes",
                IppValue::Array(vec![
                    IppValue::new_keyword("device-uri").map_err(|error| error.to_string())?,
                    IppValue::new_keyword("device-info").map_err(|error| error.to_string())?,
                    IppValue::new_keyword("device-location").map_err(|error| error.to_string())?,
                    IppValue::new_keyword("device-make-and-model")
                        .map_err(|error| error.to_string())?,
                ]),
            )
            .map_err(|error| error.to_string())?,
        );
        let response = timeout(
            wait + Duration::from_secs(1),
            self.client(self.base_uri.clone()).send(request),
        )
        .await
        .map_err(|_| "CUPS device discovery timed out".to_owned())?
        .map_err(|error| format!("CUPS device discovery failed: {error}"))?;
        ensure_success(&response, "CUPS Get-Devices")?;
        let mut printers = Vec::new();
        for group in response.attributes().groups() {
            let Some(uri) = string_attribute(group, "device-uri") else {
                continue;
            };
            let name = string_attribute(group, "device-info")
                .or_else(|| uri.rsplit('/').next().map(str::to_owned))
                .unwrap_or_else(|| uri.clone());
            printers.push(Printer {
                id: printers.len() + 1,
                name,
                uri: Some(uri),
                location: string_attribute(group, "device-location"),
                make_and_model: string_attribute(group, "device-make-and-model"),
                is_discovered: true,
                is_accepting_jobs: true,
                ..Printer::default()
            });
        }
        self.discovered_printers = printers;
        log::debug!("Discovered {} printer(s)", self.discovered_printers.len());
        Ok(&self.discovered_printers)
    }

    pub fn find_printer(&self, name: &str) -> Option<&Printer> {
        self.printers
            .iter()
            .chain(&self.discovered_printers)
            .find(|printer| printer.name == name)
    }

    pub fn default_printer(&self) -> Option<&Printer> {
        self.printers.iter().find(|printer| printer.is_default)
    }

    pub async fn refresh_printer(&self, printer: &mut Printer) -> Result<(), String> {
        *printer = refresh_printer_snapshot(self.timeout, printer.clone()).await?;
        Ok(())
    }

    pub async fn refresh_printer_jobs(&mut self) -> Result<(), String> {
        let mut first_error: Option<String> = None;
        for start in (0..self.printers.len()).step_by(MAX_CONCURRENT_PRINTER_REQUESTS) {
            let end = (start + MAX_CONCURRENT_PRINTER_REQUESTS).min(self.printers.len());
            let mut pending = JoinSet::new();
            for index in start..end {
                let printer = self.printers[index].clone();
                let request_timeout = self.timeout;
                pending.spawn(async move {
                    let result = get_printer_job_count(request_timeout, &printer).await;
                    (index, result)
                });
            }
            while let Some(result) = pending.join_next().await {
                match result {
                    Ok((index, Ok(count))) => self.printers[index].queued_jobs = count,
                    Ok((_, Err(error))) => {
                        first_error.get_or_insert(error);
                    }
                    Err(error) => {
                        first_error
                            .get_or_insert_with(|| format!("IPP refresh task failed: {error}"));
                    }
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Refreshes queue state and selects an eligible queue using active job
    /// counts, preferring idle empty queues, then the configured default and
    /// name. Requests are bounded and performed concurrently. The requested
    /// `queued-job-count` attribute avoids a separate Get-Jobs round trip.
    pub async fn select_printer(&mut self) -> Result<Option<&Printer>, String> {
        for start in (0..self.printers.len()).step_by(MAX_CONCURRENT_PRINTER_REQUESTS) {
            let end = (start + MAX_CONCURRENT_PRINTER_REQUESTS).min(self.printers.len());
            let mut pending = JoinSet::new();
            for index in start..end {
                let printer = self.printers[index].clone();
                let request_timeout = self.timeout;
                pending.spawn(async move {
                    let result = refresh_printer_snapshot(request_timeout, printer).await;
                    (index, result)
                });
            }
            while let Some(result) = pending.join_next().await {
                match result {
                    Ok((index, Ok(printer))) => self.printers[index] = printer,
                    Ok((index, Err(error))) => {
                        log::debug!(
                            "Could not refresh printer '{}': {error}",
                            self.printers[index].name
                        );
                        self.printers[index].is_accepting_jobs = false;
                    }
                    Err(error) => {
                        return Err(format!("IPP refresh task failed: {error}"));
                    }
                }
            }
        }
        let mut eligible: Vec<&Printer> = self
            .printers
            .iter()
            .filter(|printer| printer.is_accepting_jobs && printer.state != PrinterState::Stopped)
            .collect();
        eligible.sort_by(|left, right| {
            let left_idle_empty = left.state == PrinterState::Idle && left.queued_jobs == 0;
            let right_idle_empty = right.state == PrinterState::Idle && right.queued_jobs == 0;
            right_idle_empty
                .cmp(&left_idle_empty)
                .then_with(|| left.queued_jobs.cmp(&right.queued_jobs))
                .then_with(|| {
                    (right.state == PrinterState::Idle).cmp(&(left.state == PrinterState::Idle))
                })
                .then_with(|| right.is_default.cmp(&left.is_default))
                .then_with(|| left.name.cmp(&right.name))
        });
        let selected = eligible.first().copied();
        if let Some(printer) = selected {
            log::debug!(
                "Selected printer '{}' ({} active job(s), {})",
                printer.name,
                printer.queued_jobs,
                printer.state.as_str()
            );
        }
        Ok(selected)
    }

    pub async fn printer_has_active_jobs(&self, printer: &Printer) -> Result<bool, String> {
        Ok(!self.get_print_jobs(Some(printer)).await?.is_empty())
    }

    pub async fn submit_print_job(&mut self, job: &mut PrintJob) -> Result<(), String> {
        let document_path = job
            .file_path
            .as_ref()
            .ok_or("Print job has no document path")?
            .clone();
        if self.printers.is_empty() {
            self.get_printers().await?;
        }
        let printer = match job.printer_name.as_deref() {
            Some(name) => self
                .printers
                .iter()
                .find(|printer| printer.name == name)
                .cloned()
                .ok_or_else(|| format!("CUPS queue '{name}' is not configured"))?,
            None => self
                .select_printer()
                .await?
                .cloned()
                .ok_or("No eligible CUPS queue is available")?,
        };
        let document = tokio::task::spawn_blocking(move || PrintDocument::open(document_path))
            .await
            .map_err(|error| format!("Print document inspection task failed: {error}"))??;
        let uri = printer_uri(&printer)?;
        let file = tokio::fs::File::open(&document.path)
            .await
            .map_err(|error| error.to_string())?;
        let payload = ipp::payload::IppPayload::new_async(file.compat());
        let operation = IppOperationBuilder::print_job(uri.clone(), payload)
            .document_format(&document.mime_type)
            .job_title(job.title.as_deref().unwrap_or("Print Job"))
            .build()
            .map_err(|error| error.to_string())?
            .into_ipp_request();
        let response = timeout(self.timeout, self.client(uri).send(operation))
            .await
            .map_err(|_| "Print-Job timed out".to_owned())?
            .map_err(|error| format!("Print-Job failed: {error}"))?;
        ensure_success(&response, "Print-Job")?;
        if let Some(group) = first_attribute_group(&response) {
            job.id = integer_attribute(group, "job-id").unwrap_or_default();
            job.state = parse_job_state(integer_attribute(group, "job-state").unwrap_or_default());
        }
        job.printer_name = Some(printer.name);
        job.size_bytes = document.size_bytes;
        log::info!(
            "Submitted print job {} to {}",
            job.id,
            job.printer_name.as_deref().unwrap_or("unknown")
        );
        Ok(())
    }

    pub async fn get_print_jobs(&self, printer: Option<&Printer>) -> Result<Vec<PrintJob>, String> {
        let targets = match printer {
            Some(printer) => vec![(printer.name.clone(), printer_uri(printer)?)],
            None => self.configured_printer_uris().await?,
        };
        let mut jobs = Vec::new();
        for start in (0..targets.len()).step_by(MAX_CONCURRENT_PRINTER_REQUESTS) {
            let end = (start + MAX_CONCURRENT_PRINTER_REQUESTS).min(targets.len());
            let mut pending = JoinSet::new();
            for (printer_name, target_uri) in targets[start..end].iter().cloned() {
                let request_timeout = self.timeout;
                pending.spawn(async move {
                    get_jobs_for_printer(request_timeout, printer_name, target_uri).await
                });
            }
            while let Some(result) = pending.join_next().await {
                let mut printer_jobs =
                    result.map_err(|error| format!("IPP job-list task failed: {error}"))??;
                jobs.append(&mut printer_jobs);
            }
        }
        jobs.sort_by(|left, right| {
            left.printer_name
                .cmp(&right.printer_name)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(jobs)
    }

    async fn configured_printer_uris(&self) -> Result<Vec<(String, Uri)>, String> {
        let response = timeout(
            self.timeout,
            self.client(self.base_uri.clone())
                .send(IppOperationBuilder::cups().get_printers()),
        )
        .await
        .map_err(|_| "CUPS Get-Printers timed out".to_owned())?
        .map_err(|error| format!("CUPS Get-Printers failed: {error}"))?;
        ensure_success(&response, "CUPS Get-Printers")?;
        let mut printers = Vec::new();
        for group in response.attributes().groups() {
            let Some(name) = string_attribute(group, "printer-name") else {
                continue;
            };
            let address = string_attribute(group, "printer-uri-supported")
                .unwrap_or_else(|| queue_uri(&self.base_uri, &name));
            let uri = address
                .parse()
                .map_err(|error| format!("Invalid printer URI: {error}"))?;
            printers.push((name, uri));
        }
        Ok(printers)
    }

    pub async fn cancel_print_job(&self, job_id: i32) -> Result<(), String> {
        if job_id <= 0 {
            return Err("CUPS job ID must be positive".into());
        }
        for (name, uri) in self.configured_printer_uris().await? {
            let response = timeout(
                self.timeout,
                self.client(uri.clone()).send(
                    IppOperationBuilder::cancel_job(uri, job_id)
                        .build()
                        .map_err(|error| error.to_string())?,
                ),
            )
            .await;
            match response {
                Ok(Ok(response)) if response.header().status_code().is_success() => return Ok(()),
                Ok(Err(error)) => log::debug!("Job {job_id} was not canceled on {name}: {error}"),
                Err(_) => log::debug!("Cancel-Job timed out for {name}"),
                _ => {}
            }
        }
        Err(format!("CUPS did not cancel job {job_id}"))
    }
}

fn printer_uri(printer: &Printer) -> Result<Uri, String> {
    if printer.is_discovered {
        return Err("Discovered devices are not configured CUPS queues".into());
    }
    printer
        .uri
        .as_deref()
        .ok_or_else(|| format!("CUPS queue '{}' has no printer URI", printer.name))?
        .parse()
        .map_err(|error| format!("Invalid printer URI: {error}"))
}

fn first_attribute_group(
    response: &IppRequestResponse,
) -> Option<&ipp::attribute::IppAttributeGroup> {
    response
        .attributes()
        .groups()
        .iter()
        .find(|group| group.get("printer-name").is_some() || group.get("job-id").is_some())
}

fn string_attribute(group: &ipp::attribute::IppAttributeGroup, name: &str) -> Option<String> {
    let attribute = group.get(name)?;
    value_string(attribute.value())
}

fn integer_attribute(group: &ipp::attribute::IppAttributeGroup, name: &str) -> Option<i32> {
    let value = group.get(name)?.value();
    match value {
        IppValue::Integer(number) | IppValue::Enum(number) => Some(*number),
        _ => None,
    }
}

fn bool_attribute(group: &ipp::attribute::IppAttributeGroup, name: &str) -> Option<bool> {
    match group.get(name)?.value() {
        IppValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn array_strings(group: &ipp::attribute::IppAttributeGroup, name: &str) -> Vec<String> {
    let Some(attribute) = group.get(name) else {
        return Vec::new();
    };
    match attribute.value() {
        IppValue::Array(values) => values.iter().filter_map(value_string).collect(),
        value => value_string(value).into_iter().collect(),
    }
}

fn value_string(value: &IppValue) -> Option<String> {
    match value {
        IppValue::TextWithoutLanguage(value) => Some(value.to_string()),
        IppValue::NameWithoutLanguage(value) => Some(value.to_string()),
        IppValue::Keyword(value) => Some(value.to_string()),
        IppValue::Uri(value) => Some(value.to_string()),
        IppValue::UriScheme(value) => Some(value.to_string()),
        IppValue::MimeMediaType(value) => Some(value.to_string()),
        IppValue::TextWithLanguage { text, .. } => Some(text.to_string()),
        IppValue::NameWithLanguage { name, .. } => Some(name.to_string()),
        IppValue::Array(values) => values.first().and_then(value_string),
        _ => None,
    }
}

fn printer_from_attributes(group: &ipp::attribute::IppAttributeGroup, name: String) -> Printer {
    let state = match integer_attribute(group, "printer-state").unwrap_or_default() {
        3 => PrinterState::Idle,
        4 => PrinterState::Processing,
        5 => PrinterState::Stopped,
        _ => PrinterState::Unknown,
    };
    let supported = array_strings(group, "sides-supported");
    let (resolution_x, resolution_y) = group
        .get("printer-resolution-supported")
        .and_then(|attr| match attr.value() {
            IppValue::Resolution {
                cross_feed, feed, ..
            } => Some((*cross_feed, *feed)),
            IppValue::Array(values) => values.iter().find_map(|value| match value {
                IppValue::Resolution {
                    cross_feed, feed, ..
                } => Some((*cross_feed, *feed)),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default();
    Printer {
        id: 0,
        name,
        uri: string_attribute(group, "printer-uri-supported"),
        location: string_attribute(group, "printer-location"),
        description: string_attribute(group, "printer-info"),
        make_and_model: string_attribute(group, "printer-make-and-model"),
        state,
        state_message: string_attribute(group, "printer-state-message")
            .or_else(|| Some(array_strings(group, "printer-state-reasons").join(", "))),
        is_default: bool_attribute(group, "printer-is-default").unwrap_or(false),
        is_accepting_jobs: bool_attribute(group, "printer-is-accepting-jobs").unwrap_or(true),
        is_shared: bool_attribute(group, "printer-is-shared").unwrap_or(false),
        is_busy: state == PrinterState::Processing,
        queued_jobs: integer_attribute(group, "queued-job-count")
            .unwrap_or_default()
            .max(0) as usize,
        supports_color: bool_attribute(group, "color-supported").unwrap_or(false),
        supports_duplex: supported.iter().any(|value| value.starts_with("two-sided")),
        is_discovered: false,
        supported_media: array_strings(group, "media-supported"),
        resolution_x,
        resolution_y,
        instance: None,
    }
}

fn parse_job_state(state: i32) -> PrintJobState {
    match state {
        3 => PrintJobState::Pending,
        4 => PrintJobState::Held,
        5 => PrintJobState::Processing,
        9 => PrintJobState::Completed,
        7 => PrintJobState::Canceled,
        8 => PrintJobState::Aborted,
        _ => PrintJobState::Unknown,
    }
}

fn ensure_success(response: &IppRequestResponse, operation: &str) -> Result<(), String> {
    let status = response.header().status_code();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("{operation} returned IPP status {status:?}"))
    }
}

fn uri_segment(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

fn queue_uri(base_uri: &Uri, printer_name: &str) -> String {
    format!(
        "{}/printers/{}",
        base_uri.to_string().trim_end_matches('/'),
        uri_segment(printer_name)
    )
}

fn client_for(uri: Uri, request_timeout: Duration) -> AsyncIppClient {
    AsyncIppClient::builder(uri)
        .request_timeout(request_timeout)
        .build()
}

async fn refresh_printer_snapshot(
    request_timeout: Duration,
    mut printer: Printer,
) -> Result<Printer, String> {
    let uri = printer_uri(&printer)?;
    let request = IppOperationBuilder::get_printer_attributes(uri.clone())
        .attributes(PRINTER_ATTRIBUTES)
        .build()
        .map_err(|error| error.to_string())?;
    let response = timeout(
        request_timeout,
        client_for(uri, request_timeout).send(request),
    )
    .await
    .map_err(|_| format!("Get-Printer-Attributes timed out for {}", printer.name))?
    .map_err(|error| {
        format!(
            "Get-Printer-Attributes failed for {}: {error}",
            printer.name
        )
    })?;
    ensure_success(&response, "Get-Printer-Attributes")?;
    let group = first_attribute_group(&response)
        .ok_or_else(|| format!("No printer attributes returned for {}", printer.name))?;
    let id = printer.id;
    let instance = printer.instance.clone();
    printer = printer_from_attributes(group, printer.name.clone());
    printer.id = id;
    printer.instance = instance;
    Ok(printer)
}

async fn get_printer_job_count(
    request_timeout: Duration,
    printer: &Printer,
) -> Result<usize, String> {
    let uri = printer_uri(printer)?;
    let request = IppOperationBuilder::get_jobs(uri.clone())
        .build()
        .map_err(|error| error.to_string())?;
    let response = timeout(
        request_timeout,
        client_for(uri, request_timeout).send(request),
    )
    .await
    .map_err(|_| format!("Get-Jobs timed out for {}", printer.name))?
    .map_err(|error| format!("Get-Jobs failed for {}: {error}", printer.name))?;
    ensure_success(&response, "Get-Jobs")?;
    Ok(response
        .attributes()
        .groups()
        .iter()
        .filter(|group| string_attribute(group, "job-state").is_some())
        .count())
}

async fn get_jobs_for_printer(
    request_timeout: Duration,
    printer_name: String,
    uri: Uri,
) -> Result<Vec<PrintJob>, String> {
    let request = IppOperationBuilder::get_jobs(uri.clone())
        .build()
        .map_err(|error| error.to_string())?;
    let response = timeout(
        request_timeout,
        client_for(uri, request_timeout).send(request),
    )
    .await
    .map_err(|_| format!("Get-Jobs timed out for {printer_name}"))?
    .map_err(|error| format!("Get-Jobs failed for {printer_name}: {error}"))?;
    ensure_success(&response, "Get-Jobs")?;

    Ok(response
        .attributes()
        .groups()
        .iter()
        .filter_map(|group| {
            let id = integer_attribute(group, "job-id").unwrap_or_default();
            (id > 0).then(|| PrintJob {
                id,
                printer_name: string_attribute(group, "job-printer-uri")
                    .and_then(|uri| uri.rsplit('/').next().map(str::to_owned))
                    .or_else(|| Some(printer_name.clone())),
                title: string_attribute(group, "job-name"),
                owner: string_attribute(group, "job-originating-user-name"),
                state: parse_job_state(integer_attribute(group, "job-state").unwrap_or_default()),
                size_bytes: integer_attribute(group, "job-k-octets")
                    .unwrap_or_default()
                    .max(0) as u64
                    * 1024,
                impressions_completed: integer_attribute(group, "job-impressions-completed")
                    .unwrap_or_default(),
                file_path: None,
            })
        })
        .collect())
}
