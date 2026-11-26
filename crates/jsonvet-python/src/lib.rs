//! In-memory checks exposed to Python.

use jsonvet_core::{Config, Dialect, UnknownDialect};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// One finding from check(), with a 1-based position and a half-open UTF-8 byte span.
#[pyclass(frozen, get_all, module = "jsonvet")]
struct Diagnostic {
    severity: &'static str,
    rule: &'static str,
    message: String,
    line: usize,
    column: usize,
    byte_offset: usize,
    byte_end: usize,
}

/// Check a string using the dialect's default rules. Returns findings in document order.
#[pyfunction]
#[pyo3(signature = (text, *, dialect = "json"))]
fn check(py: Python<'_>, text: &str, dialect: &str) -> PyResult<Vec<Diagnostic>> {
    let dialect = dialect
        .parse::<Dialect>()
        .map_err(|error: UnknownDialect| PyValueError::new_err(error.to_string()))?;
    Ok(py.detach(|| {
        let report = jsonvet_core::check(text, &Config::new(dialect));
        report
            .problems()
            .iter()
            .map(|problem| {
                let location = report.location(problem.offset());
                Diagnostic {
                    severity: problem.severity().name(),
                    rule: problem.rule().id(),
                    message: problem.to_string(),
                    line: location.line,
                    column: location.column,
                    byte_offset: problem.offset(),
                    byte_end: problem.end(),
                }
            })
            .collect()
    }))
}

/// Check JSON-family text in memory.
#[pymodule]
fn jsonvet(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Diagnostic>()?;
    module.add_function(wrap_pyfunction!(check, module)?)?;
    Ok(())
}
