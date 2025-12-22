

use std::sync::Arc;

use axum::http::HeaderMap;

use pyo3::prelude::*;
use pyo3::exceptions::{PyOSError, PyTypeError};
use pyo3_async_runtimes::tokio::future_into_py;

use crate::models::{
    TextContentDetectionHttpRequest,
};

use crate::orchestrator::{
    Orchestrator,
    handlers::{text_content_detection, Handle},
};
use crate::utils::trace;

use crate::config::OrchestratorConfig;

// Define orchestrator
#[pyclass]
pub struct GuardrailsOrchestrator {
    // Added under Arc to allow safe sharing from multiple threads
    // If we need modification capability later, then we might want to make it mutable
    orchestrator: Arc<Orchestrator>,
}

#[pymethods]
impl GuardrailsOrchestrator {

    #[new]
    fn new(config_path: String, start_up_health_check: bool) -> PyResult<Self> {
        Python::attach(|py| {
            pyo3_async_runtimes::tokio::run(py, async move {
            let config_result = OrchestratorConfig::load(config_path).await;

            let config = match config_result {
                Ok(config) => config,
                Err(err) => return Err(PyOSError::new_err(format!("Error loading orchestrator configuration: {}", err)))
            };

            let orchestrator_result = Orchestrator::new(config, start_up_health_check).await;

            let orchestrator = match orchestrator_result {
                Ok(orchestrator) => orchestrator,
                Err(err) => return Err(PyOSError::new_err(format!("Error creating orchestrator: {}", err)))
            };

            Ok(GuardrailsOrchestrator {
                orchestrator: Arc::new(orchestrator),
            })

        })})
    }

    pub fn content_detection<'py>(
        &self,
        py: Python<'py>,
        request: TextContentDetectionHttpRequest,
    ) -> PyResult<Bound<'py, PyAny>>   {

        let headers = HeaderMap::new();
        let trace_id = trace::current_trace_id();

        let task = text_content_detection::TextContentDetectionTask::new(trace_id, request, headers);

        let orchestrator = Arc::clone(&self.orchestrator);

        future_into_py(py, async move {
            match orchestrator.handle(task).await {
                    Ok(response) => {
                        println!("Response: {:?}", response);
                        Ok(response)
                    },
                    // TODO: Handle errors properly with correct types
                    Err(error) => Err(PyTypeError::new_err(error.to_string())),
                }
        })
    }

}