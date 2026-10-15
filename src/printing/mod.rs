//! IPP printer discovery, queue selection, and print-job operations.

mod service;
mod types;

pub use service::PrintService;
pub use types::{PrintDocument, PrintJob, PrintJobState, Printer, PrinterState};
