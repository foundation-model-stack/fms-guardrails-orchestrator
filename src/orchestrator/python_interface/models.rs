use std::collections::{BTreeMap, HashMap};

use pyo3::prelude::*;
use pyo3::types::PyDict;
use pythonize::{depythonize};


use crate::clients::detector::{ContextType};

use crate::models::{
    ContextDocsHttpRequest,
    DetectorParams,
    TextContentDetectionHttpRequest,
    TextContentDetectionResult,
};



impl<'py> FromPyObject<'py> for DetectorParams {
    fn extract_bound(obj: &Bound<'py, PyAny>) -> PyResult<Self> {
        println!("{:?} reached here", obj);
        // Use depythonize to convert the Python object directly to BTreeMap
        let map: BTreeMap<String, serde_json::Value> = depythonize(obj)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e.to_string()))?;

        Ok(DetectorParams(map))
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
    type Output =  Bound<'py, Self::Target>;
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
    fn new(content: String, context_type: ContextType, context: Vec<String>, detectors: HashMap<String, DetectorParams>) -> Self {
        Self {
            content,
            context,
            context_type: context_type.into(),
            detectors: detectors.into()
        }
    }
}

