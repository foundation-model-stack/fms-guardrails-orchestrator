/*
 Copyright FMS Guardrails Orchestrator Authors

 Licensed under the Apache License, Version 2.0 (the "License");
 you may not use this file except in compliance with the License.
 You may obtain a copy of the License at

     http://www.apache.org/licenses/LICENSE-2.0

 Unless required by applicable law or agreed to in writing, software
 distributed under the License is distributed on an "AS IS" BASIS,
 WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 See the License for the specific language governing permissions and
 limitations under the License.

*/
use std::collections::HashMap;

use common::{
    detectors::{ANSWER_RELEVANCE_DETECTOR, DETECTION_ON_GENERATION_DETECTOR_ENDPOINT},
    generation::{GENERATION_NLP_MODEL_ID_HEADER_NAME, GENERATION_NLP_UNARY_ENDPOINT},
    orchestrator::{
        TestOrchestratorServer, ORCHESTRATOR_CONFIG_FILE_PATH,
        ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT,
    },
};
use fms_guardrails_orchestr8::{
    clients::detector::GenerationDetectionRequest,
    models::{
        DetectionResult, DetectorParams, GenerationWithDetectionHttpRequest,
        GenerationWithDetectionResult,
    },
    pb::{
        caikit::runtime::nlp::TextGenerationTaskRequest,
        caikit_data_model::nlp::GeneratedTextResult,
    },
};
use http::StatusCode;
use mocktail::{server::MockServer, MockSet};
use test_log::test;
use tracing::debug;

pub mod common;

/// Asserts detections below default threshold are not returned.
#[test(tokio::test)]
async fn no_detections() -> Result<(), anyhow::Error> {
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";
    let generated_text = "The average height of women is 159cm (or 5'3'').";
    let detection = DetectionResult {
        detection_type: "relevance".into(),
        detection: "is_relevant".into(),
        detector_id: Some(detector_name.into()),
        score: 0.49,
        evidence: None,
    };

    // Add generation mock
    let model_id = "my-super-model-8B";

    let mut generation_mocks = MockSet::new();
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(TextGenerationTaskRequest {
                text: prompt.into(),
                ..Default::default()
            });
        then.pb(GeneratedTextResult {
            generated_text: generated_text.into(),
            ..Default::default()
        });
    });

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.mock(|when, then| {
        when.post()
            .path(DETECTION_ON_GENERATION_DETECTOR_ENDPOINT)
            .json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
                detector_params: DetectorParams::new(),
            });
        then.json(vec![detection.clone()]);
    });

    // Start orchestrator server and its dependencies
    let generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detection_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&generation_server)
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&GenerationWithDetectionHttpRequest {
            model_id: model_id.into(),
            prompt: prompt.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    // assertions
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<GenerationWithDetectionResult>().await?,
        GenerationWithDetectionResult {
            generated_text: generated_text.into(),
            ..Default::default()
        }
    );

    Ok(())
}

/// Asserts detections above default threshold are returned.
#[test(tokio::test)]
async fn detections() -> Result<(), anyhow::Error> {
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";
    let generated_text =
        "The average height of men who were born in 1996 was 171cm (or 5'7.5'') in 2014.";
    let detection = DetectionResult {
        detection_type: "relevance".into(),
        detection: "is_relevant".into(),
        detector_id: Some(detector_name.into()),
        score: 0.89,
        evidence: None,
    };

    // Add generation mock
    let model_id = "my-super-model-8B";

    let mut generation_mocks = MockSet::new();
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(TextGenerationTaskRequest {
                text: prompt.into(),
                ..Default::default()
            });
        then.pb(GeneratedTextResult {
            generated_text: generated_text.into(),
            ..Default::default()
        });
    });

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.mock(|when, then| {
        when.post()
            .path(DETECTION_ON_GENERATION_DETECTOR_ENDPOINT)
            .json(GenerationDetectionRequest {
                prompt: prompt.into(),
                generated_text: generated_text.into(),
                detector_params: DetectorParams::new(),
            });
        then.json(vec![detection.clone()]);
    });

    // Start orchestrator server and its dependencies
    let generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detection_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&generation_server)
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // Make orchestrator call
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&GenerationWithDetectionHttpRequest {
            model_id: model_id.into(),
            prompt: prompt.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    // assertions
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<GenerationWithDetectionResult>().await?,
        GenerationWithDetectionResult {
            generated_text: generated_text.into(),
            detections: vec![detection.clone()],
            input_token_count: 0
        }
    );

    Ok(())
}
