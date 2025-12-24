use std::sync::Arc;

use axum::http::HeaderMap;
use tokio::sync::Mutex;

use pyo3::{
    exceptions::{PyOSError, PyTypeError},
    prelude::*,
};
use pyo3_async_runtimes::tokio::future_into_py;

use crate::{
    clients::openai::ChatCompletionsResponse,
    config::OrchestratorConfig,
    models::TextContentDetectionHttpRequest,
    orchestrator::{
        Orchestrator,
        handlers::{Handle, chat_completions_detection, text_content_detection},
    },
};

use crate::{
    orchestrator::python_interface::models::{PyChatCompletionsRequest, PyChatCompletionsStream},
    utils::trace,
};

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
                    Err(err) => {
                        return Err(PyOSError::new_err(format!(
                            "Error loading orchestrator configuration: {}",
                            err
                        )));
                    }
                };

                let orchestrator_result = Orchestrator::new(config, start_up_health_check).await;

                let orchestrator = match orchestrator_result {
                    Ok(orchestrator) => orchestrator,
                    Err(err) => {
                        return Err(PyOSError::new_err(format!(
                            "Error creating orchestrator: {}",
                            err
                        )));
                    }
                };

                Ok(GuardrailsOrchestrator {
                    orchestrator: Arc::new(orchestrator),
                })
            })
        })
    }

    pub fn content_detection<'py>(
        &self,
        py: Python<'py>,
        request: TextContentDetectionHttpRequest,
    ) -> PyResult<Bound<'py, PyAny>> {
        let headers = HeaderMap::new();
        let trace_id = trace::current_trace_id();

        let task =
            text_content_detection::TextContentDetectionTask::new(trace_id, request, headers);

        let orchestrator = Arc::clone(&self.orchestrator);

        future_into_py(py, async move {
            match orchestrator.handle(task).await {
                Ok(response) => {
                    println!("Response: {:?}", response);
                    Ok(response)
                }
                // TODO: Handle errors properly with correct types
                Err(error) => Err(PyTypeError::new_err(error.to_string())),
            }
        })
    }

    pub fn chat_completions_detection<'py>(
        &self,
        py: Python<'py>,
        request: PyChatCompletionsRequest,
    ) -> PyResult<Bound<'py, PyAny>> {
        let headers = HeaderMap::new();
        let trace_id = trace::current_trace_id();

        let task = chat_completions_detection::ChatCompletionsDetectionTask::new(
            trace_id, request.0, headers,
        );

        let orchestrator = Arc::clone(&self.orchestrator);

        // Convert the async move { } block to a Python awaitable
        future_into_py(py, async move {
            match orchestrator.handle(task).await {
                Ok(ChatCompletionsResponse::Unary(response)) => {
                    println!("Response: {:?}", response);
                    // response.into_pyobject(py)
                    // response.into_pyobject(py).map(|obj| obj.into())
                    // Ok(Python::attach(|py| pythonize(py, &*response)))
                    // Ok(Python::attach(|py| response.into_py_any(py)))
                    // let result_py_object: Py<PyAny> = Python::attach(|py| {
                    //     let bound_any = pythonize(py, &response)
                    //         .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

                    //     // 3. .unbind() converts Bound<'py, PyAny> to Py<PyAny> (aka PyObject)
                    //     // This "erases" the lifetime so it can safely leave the closure
                    //     Ok::<pyo3::Py<pyo3::PyAny>, PyErr>(bound_any.unbind())
                    // })?;

                    // Ok(result_py_object)
                    let py_obj = Python::attach(|py| {
                        let bound_any = Bound::new(py, *response)?.into_any();

                        Ok::<PyObject, PyErr>(bound_any.unbind())
                        // bound_any.unbind()
                        // bound_any
                    });
                    // Ok(py_obj)
                    println!("{:?}", py_obj);
                    py_obj
                }

                Ok(ChatCompletionsResponse::Streaming(receiver_stream)) => {
                    let py_stream = PyChatCompletionsStream {
                        receiver: Arc::new(Mutex::new(receiver_stream)),
                    };
                    // Ok(Python::attach(|py| pythonize(py, py_stream)))
                    // Ok(Python::attach(|py| py_stream.into_py_any(py)))
                    // py_stream.into_pyobject(py)
                    // py_stream.into_pyobject(py).map(|obj| obj.into())
                    // let result_py_object: Py<PyAny> = Python::attach(|py| {
                    //     let bound_any = pythonize(py, &py_stream)
                    //         .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

                    //     // 3. .unbind() converts Bound<'py, PyAny> to Py<PyAny> (aka PyObject)
                    //     // This "erases" the lifetime so it can safely leave the closure
                    //     Ok::<pyo3::Py<pyo3::PyAny>, PyErr>(bound_any.unbind())
                    // })?;
                    // Ok(result_py_object)

                    let py_obj = Python::attach(|py| {
                        let bound_any = Bound::new(py, py_stream)?.into_any();
                        Ok::<PyObject, PyErr>(bound_any.unbind())
                    });
                    py_obj
                }
                Err(e) => Err(PyTypeError::new_err(e.to_string())),
            }
        })
    }
}
