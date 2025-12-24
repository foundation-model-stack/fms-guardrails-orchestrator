use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use pyo3::{prelude::*, types::PyDict};
use pyo3_async_runtimes::tokio::future_into_py;
use pythonize::{depythonize, pythonize};

use tokio::sync::{Mutex, mpsc};

use crate::{
    clients::{detector::ContextType, openai},
    models::{
        ContextDocsHttpRequest, DetectorParams, TextContentDetectionHttpRequest,
        TextContentDetectionResult,
    },
    orchestrator,
    orchestrator::types::Detection,
};

impl<'a, 'py> FromPyObject<'a, 'py> for DetectorParams {
    // Add an associated Error type
    type Error = PyErr;

    fn extract(obj: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        // Use depythonize to convert the Python object directly to BTreeMap
        let map: BTreeMap<String, serde_json::Value> = depythonize(&obj)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e.to_string()))?;

        Ok(DetectorParams(map))
    }
}

#[pymethods]
impl Detection {
    #[getter]
    fn metadata<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        // pythonize converts the BTreeMap and all nested Values into a Python dict
        pythonize(py, &self.metadata)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }
}

#[pymethods]
impl openai::ChatCompletionChoice {
    pub fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        pythonize(py, self).map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }
}

// Text Content Detection

#[pymethods]
impl TextContentDetectionHttpRequest {
    #[new]
    #[pyo3(signature = (content, detectors = HashMap::new()))]
    fn new(content: String, detectors: HashMap<String, DetectorParams>) -> Self {
        Self { content, detectors }
    }
}

impl<'py> IntoPyObject<'py> for TextContentDetectionResult {
    type Target = PyAny;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        let mut detections = Vec::with_capacity(self.detections.len());
        // Note below will get simplified once we move all of this declaration to Orchestrator itself.
        for detection in self.detections {
            let content_response = PyDict::new(py);
            content_response.set_item("start", detection.start)?;
            content_response.set_item("end", detection.end)?;
            content_response.set_item("text", detection.text)?;
            content_response.set_item("detection", detection.detection)?;
            content_response.set_item("detection_type", detection.detection_type)?;
            content_response.set_item("detector_id", detection.detector_id)?;
            content_response.set_item("score", detection.score)?;
            detections.push(content_response)
        }
        detections.into_pyobject(py)
    }
}

// Context Doc

#[pymethods]
impl ContextDocsHttpRequest {
    #[new]
    #[pyo3(signature = (content, context_type, context, detectors = HashMap::new()))]
    fn new(
        content: String,
        context_type: ContextType,
        context: Vec<String>,
        detectors: HashMap<String, DetectorParams>,
    ) -> Self {
        Self {
            content,
            context,
            context_type: context_type.into(),
            detectors: detectors.into(),
        }
    }
}

// Completion Detection

#[pymethods]
impl openai::Completion {
    // Implementing manual getter to override the default get behavior. Reasons:
    // 1. Default getter gives error because of serde_value (eventually)
    // 2. Adds way too much nesting to be able to get to final answer
    // 3. This provides an easy way to get the response in python dict, allowing
    //     easier access and possibly conversion in python.
    #[getter]
    fn detections<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        // This converts the Rust struct into a native Python dict automatically
        pythonize(py, &self.detections)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }
}

// Chat Completion Detection

#[pymethods]
impl openai::ChatCompletion {
    #[getter]
    fn detections<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        // This converts the Rust struct into a native Python dict automatically
        pythonize(py, &self.detections)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }
}

#[pymethods]
impl openai::ChatCompletionChunk {
    #[getter]
    fn detections<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        // This converts the Rust struct into a native Python dict automatically
        pythonize(py, &self.detections)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }
}

#[pymethods]
impl openai::Message {
    #[new]
    #[pyo3(signature = (data = None, **kwargs))]
    fn new(
        py: Python<'_>,
        data: Option<Bound<'_, PyAny>>,
        kwargs: Option<Bound<'_, pyo3::types::PyDict>>,
    ) -> PyResult<Self> {
        // Identify the source: prioritize positional 'data' then 'kwargs'
        let source = if let Some(datum) = data {
            datum
        } else if let Some(k) = kwargs {
            k.into_any()
        } else {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "Missing message data",
            ));
        };

        // Convert from Python object/dict to Rust struct via depythonize
        // This works for:
        // - A dict: {"role": "user", "content": "..."}
        let message: openai::Message = depythonize(&source).map_err(|e| {
            pyo3::exceptions::PyTypeError::new_err(format!("Invalid message format: {}", e))
        })?;

        Ok(message)
    }
}

#[pymethods]
impl openai::DetectorConfig {
    // TODO: Allow both dictionary as well as struct input
    #[new]
    #[pyo3(signature = (input = HashMap::new(), output = HashMap::new()))]
    fn new(
        input: HashMap<String, DetectorParams>,
        output: HashMap<String, DetectorParams>,
    ) -> Self {
        Self { input, output }
    }
}

#[pyclass(name = "ChatCompletionsRequest")]
#[derive(Clone)]
pub struct PyChatCompletionsRequest(pub openai::ChatCompletionsRequest);

#[pymethods]
impl PyChatCompletionsRequest {
    #[new]
    #[pyo3(signature = (model, detectors, messages, stream = None, extra = HashMap::new(), tools = None ))]
    fn new(
        py: Python<'_>,
        model: String,
        detectors: openai::DetectorConfig,
        messages: Bound<'_, PyAny>,
        stream: Option<bool>,
        extra: HashMap<String, Py<PyAny>>,
        tools: Option<Vec<openai::Tool>>,
    ) -> PyResult<Self> {
        let mut json_map = Map::new();

        for (key, py_obj) in extra {
            // Bind the Py<PyAny> to the current GIL to get a Bound<'_, PyAny>
            let bound_obj = py_obj.bind(py);

            // Convert Python object to serde_json::Value
            let value: Value = depythonize(bound_obj)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e.to_string()))?;

            json_map.insert(key, value);
        }
        let new_messages: Vec<openai::Message> = depythonize(&messages).map_err(|e| {
            pyo3::exceptions::PyTypeError::new_err(format!(
                "Argument 'messages' must be a list of valid message objects. Error: {}",
                e
            ))
        })?;

        Ok(Self(openai::ChatCompletionsRequest {
            stream,
            detectors,
            model,
            messages: new_messages,
            tools,
            extra: json_map,
        }))
    }
}

#[pyclass(name = "ChatCompletionsStream")]
pub struct PyChatCompletionsStream {
    pub receiver: Arc<
        Mutex<mpsc::Receiver<Result<Option<openai::ChatCompletionChunk>, orchestrator::Error>>>,
    >,
}

#[pymethods]
impl PyChatCompletionsStream {
    fn __aiter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let receiver = self.receiver.clone();

        future_into_py(py, async move {
            let mut receiver_stream = receiver.lock().await;

            match receiver_stream.recv().await {
                Some(Ok(Some(chunk))) => Ok(chunk),

                Some(Ok(None)) => Err(pyo3::exceptions::PyStopAsyncIteration::new_err(())),

                Some(Err(e)) => Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                    e.to_string(),
                )),

                None => Err(pyo3::exceptions::PyStopAsyncIteration::new_err(())),
            }
        })
    }
}
