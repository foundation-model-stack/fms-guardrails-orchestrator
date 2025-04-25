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
    detectors::{
        ANSWER_RELEVANCE_DETECTOR, DETECTION_ON_GENERATION_DETECTOR_ENDPOINT,
        FACT_CHECKING_DETECTOR_SENTENCE, NON_EXISTING_DETECTOR,
    },
    errors::{DetectorError, OrchestratorError},
    generation::{GENERATION_NLP_MODEL_ID_HEADER_NAME, GENERATION_NLP_UNARY_ENDPOINT},
    orchestrator::{
        ORCHESTRATOR_CONFIG_FILE_PATH, ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT,
        TestOrchestratorServer,
    },
};
use fms_guardrails_orchestr8::{
    clients::detector::GenerationDetectionRequest,
    models::{
        DetectionResult, DetectorParams, GenerationWithDetectionHttpRequest,
        GenerationWithDetectionResult, Metadata,
    },
    pb::{
        caikit::runtime::nlp::TextGenerationTaskRequest,
        caikit_data_model::nlp::GeneratedTextResult,
    },
};
use http::StatusCode;
use mocktail::{MockSet, server::MockServer};
use serde_json::json;
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
        metadata: Metadata::new(),
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
        then.json([&detection]);
    });

    // Start orchestrator server and its dependencies
    let mock_generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detection_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&mock_generation_server)
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
        metadata: Metadata::new(),
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
        then.json([&detection]);
    });

    // Start orchestrator server and its dependencies
    let mock_generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detection_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&mock_generation_server)
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
/// Asserts scenarios in which errors are returned from clients.
#[test(tokio::test)]
async fn client_error() -> Result<(), anyhow::Error> {
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let generation_error_prompt = "Generation server should return an error";
    let detector_error_prompt = "Detector should return an error";
    let generated_text = "Hey, detector. Give me a 500!";
    let detector_error = DetectorError {
        code: 500,
        message: "Here's your 500 error".into(),
    };
    let orchestrator_error_500 = OrchestratorError::internal();

    // Add generation mock
    let model_id = "my-super-model-8B";
    let mut generation_mocks = MockSet::new();
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(TextGenerationTaskRequest {
                text: generation_error_prompt.into(),
                ..Default::default()
            });
        then.internal_server_error();
    });
    generation_mocks.mock(|when, then| {
        when.path(GENERATION_NLP_UNARY_ENDPOINT)
            .header(GENERATION_NLP_MODEL_ID_HEADER_NAME, model_id)
            .pb(TextGenerationTaskRequest {
                text: detector_error_prompt.into(),
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
                prompt: detector_error_prompt.into(),
                generated_text: generated_text.into(),
                detector_params: DetectorParams::new(),
            });
        then.json(&detector_error).internal_server_error();
    });

    // Start orchestrator server and its dependencies
    let mock_generation_server = MockServer::new("nlp").grpc().with_mocks(generation_mocks);
    let mock_detector_server = MockServer::new(detector_name).with_mocks(detection_mocks);
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .generation_server(&mock_generation_server)
        .detector_servers([&mock_detector_server])
        .build()
        .await?;

    // assert generation error
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&GenerationWithDetectionHttpRequest {
            model_id: model_id.into(),
            prompt: generation_error_prompt.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.json::<OrchestratorError>().await?,
        orchestrator_error_500
    );

    // assert generation error
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&GenerationWithDetectionHttpRequest {
            model_id: model_id.into(),
            prompt: detector_error_prompt.into(),
            detectors: HashMap::from([(detector_name.into(), DetectorParams::new())]),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.json::<OrchestratorError>().await?,
        orchestrator_error_500
    );

    Ok(())
}

/// Asserts detections below default threshold are not returned.
#[test(tokio::test)]
async fn orchestrator_validation_error() -> Result<(), anyhow::Error> {
    let model_id = "my-super-model-8B";
    let detector_name = ANSWER_RELEVANCE_DETECTOR;
    let prompt = "In 2014, what was the average height of men who were born in 1996?";

    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .build()
        .await?;

    // assert request extra args
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&json!({
            "model_id": model_id,
            "prompt": prompt,
            "detectors": {detector_name: {}},
            "extra_args": true
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("unknown field `extra_args`"));

    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::builder()
        .config_path(ORCHESTRATOR_CONFIG_FILE_PATH)
        .build()
        .await?;

    // assert request missing `model_id`
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&json!({
            "prompt": prompt,
            "detectors": {detector_name: {}},
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `model_id`"));

    // assert request missing `prompt`
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&json!({
            "model_id": model_id,
            "detectors": {detector_name: {}},
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `prompt`"));

    // assert request missing `detectors`
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&json!({
            "model_id": model_id,
            "prompt": prompt,
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response.code, 422);
    assert!(response.details.contains("missing field `detectors`"));

    // assert request with invalid `detectors`
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&json!({
            "model_id": model_id,
            "prompt": prompt,
            "detectors": {}
        }))
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(response, OrchestratorError::required("detectors"));

    // assert request with invalid type detectors
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&GenerationWithDetectionHttpRequest {
            model_id: model_id.into(),
            prompt: prompt.into(),
            detectors: HashMap::from([(
                FACT_CHECKING_DETECTOR_SENTENCE.into(),
                DetectorParams::new(),
            )]),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(
        response,
        OrchestratorError::detector_not_supported(FACT_CHECKING_DETECTOR_SENTENCE),
        "failed at invalid detector scenario"
    );

    // assert request with non-existing detector
    let response = orchestrator_server
        .post(ORCHESTRATOR_GENERATION_WITH_DETECTION_ENDPOINT)
        .json(&GenerationWithDetectionHttpRequest {
            model_id: model_id.into(),
            prompt: prompt.into(),
            detectors: HashMap::from([(NON_EXISTING_DETECTOR.into(), DetectorParams::new())]),
            text_gen_parameters: None,
        })
        .send()
        .await?;
    debug!("{response:#?}");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = response.json::<OrchestratorError>().await?;
    debug!("{response:#?}");
    assert_eq!(
        response,
        OrchestratorError::detector_not_found(NON_EXISTING_DETECTOR),
        "failed on non-existing detector scenario"
    );

    Ok(())
}
