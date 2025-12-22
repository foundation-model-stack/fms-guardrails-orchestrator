

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::exceptions::{PyOSError, PyTypeError};
use pyo3::PyErr;
use pyo3::types::PyDict;
use pyo3::conversion::{FromPyObject};

use crate::orchestrator::Orchestrator;

use crate::config::OrchestratorConfig;

// Define orchestrator
#[pyclass]
struct GuardrailsOrchestrator {
    // Added under Arc to allow safe sharing from multiple threads
    // If we need modification capability later, then we might want to make it mutable
    orchestrator: Arc<Orchestrator>,
}

#[pymethods]
impl GuardrailsOrchestrator {

    #[new]
    fn new(config_path: String) -> PyResult<Self> {
        Python::with_gil(|py| {
            pyo3_async_runtimes::tokio::run(py, async move {
            let config_result = OrchestratorConfig::load(config_path).await;

            let config = match config_result {
                Ok(config) => config,
                Err(err) => return Err(PyOSError::new_err(format!("Error loading orchestrator configuration: {}", err)))
            };

            let orchestrator_result = Orchestrator::new(config, STARTUP_HEALTHCHECK).await;

            let orchestrator = match orchestrator_result {
                Ok(orchestrator) => orchestrator,
                Err(err) => return Err(PyOSError::new_err(format!("Error creating orchestrator: {}", err)))
            };

            Ok(GuardrailsOrchestrator {
                orchestrator: Arc::new(orchestrator),
            })

        })})
    }
}