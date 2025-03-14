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

use std::{collections::HashMap, vec};

use common::{
    chunker::CHUNKER_UNARY_ENDPOINT, detectors::{
        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, TEXT_CONTENTS_DETECTOR_ENDPOINT
    }, errors::{DetectorError, OrchestratorError}, generation::{GENERATION_NLP_MODEL_ID_HEADER_NAME, GENERATION_NLP_TOKENIZATION_ENDPOINT, GENERATION_NLP_UNARY_ENDPOINT}, orchestrator::{
        TestOrchestratorServer, ORCHESTRATOR_CONFIG_FILE_PATH, ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE, ORCHESTRATOR_UNARY_ENDPOINT
    }
};
use fms_guardrails_orchestr8::{
    clients::{
        chunker::MODEL_ID_HEADER_NAME as CHUNKER_MODEL_ID_HEADER_NAME,
        detector::{ContentAnalysisRequest, ContentAnalysisResponse},
    },
    models::{ClassifiedGeneratedTextResult, DetectionWarning, DetectionWarningReason, DetectorParams, GuardrailsConfig, GuardrailsConfigInput, GuardrailsConfigOutput, GuardrailsHttpRequest, TextGenTokenClassificationResults, TokenClassificationResult},
    pb::{
        caikit::runtime::{chunkers::ChunkerTokenizationTaskRequest, nlp::{TextGenerationTaskRequest, TokenizationTaskRequest}},
        caikit_data_model::nlp::{GeneratedTextResult, Token, TokenizationResults},
    },
};
use hyper::StatusCode;
use mocktail::{prelude::*, utils::find_available_port};
use test_log::test;

pub mod common;

// Constants
const CHUNKER_NAME_SENTENCE: &str = "sentence_chunker";
const MODEL_ID: &str = "my-super-model-8B";

// Asserts that for a given request without detectors returns text generation by model
#[test(tokio::test)]
async fn whole_doc_chunker_generation_only() -> Result<(), anyhow::Error> {
    // Add generation nlp model header
    let mut headers = HeaderMap::new();
    headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add expected generated text
    let expected_response = GeneratedTextResult {
            generated_text: "I am great!".into(),
            ..Default::default()
        };

    // Add generation mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&&Method::POST, GENERATION_NLP_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(TextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb(expected_response.clone()),
        ),
    );

    // Configure mock servers
    let generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;

    // Run test orchestrator server
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(generation_server),
        None,
        None,
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: None,
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    assert!(response.json::<ClassifiedGeneratedTextResult>().await?.generated_text ==
        Some(expected_response.generated_text));

    Ok(())
}

// Asserts that for a given request with input detector using whole_doc_chunker and no detections 
// returns generated text
#[test(tokio::test)]
async fn whole_doc_chunker_input_no_detections() -> Result<(), anyhow::Error> {
    // Add generation nlp model header
    let mut headers = HeaderMap::new();
    headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add expected generated text
    let expected_response = GeneratedTextResult {
            generated_text: "I am great!".into(),
            ..Default::default()
        };

    // Add generation mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(TextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb(expected_response.clone()),
        ),
    );

    // Add detector mock
    let mut detector_mocks = MockSet::new();
    detector_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec![
                    "Hi there! How are you?".into()
                ],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![
                Vec::<ContentAnalysisResponse>::new(),
            ]),
        ),
    );

    // Configure mock servers
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detector_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: None,
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert!(results.generated_text == Some(expected_response.generated_text));
    assert!(results.warnings == None);

    Ok(())
}

// Asserts that for a given request with input detector using whole_doc_chunker and a detection 
// returns found detection and no generated text
#[test(tokio::test)]
async fn whole_doc_chunker_input_detection() -> Result<(), anyhow::Error> {
    // Add generation nlp mock
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add tokenization response mock
    let mock_tokenization_response = TokenizationResults {
        results: Vec::new(),
        token_count: 61,
    };

    // Add tokenization mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_TOKENIZATION_ENDPOINT),
        Mock::new(
            MockRequest::pb(TokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::pb(mock_tokenization_response.clone()),
        ),
    );

    // Add input detection mock response
    let expected_detection = ContentAnalysisResponse {
        start: 46,
        end: 59,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into()),
        score: 1.0,
        evidence: None,
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection. But <this one does>.".into(),
                ],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([vec![expected_detection.clone()]]),
        ),
    );

    // Configure mock servers
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Example orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This sentence does not have a detection. But <this one does>.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert!(results.generated_text == None);
    assert!(results.token_classification_results
            == TextGenTokenClassificationResults {
                input: Some(vec![TokenClassificationResult {
                    start: expected_detection.start as u32,
                    end: expected_detection.end as u32,
                    word: expected_detection.text,
                    entity: expected_detection.detection,
                    entity_group: expected_detection.detection_type,
                    detector_id: expected_detection.detector_id,
                    score: expected_detection.score,
                    token_count: None
                }]),
                output: None
            });
    assert!(results.input_token_count == mock_tokenization_response.token_count as u32);
    assert!(results.warnings == Some(vec![DetectionWarning { 
        id: Some(DetectionWarningReason::UnsuitableInput), 
        message: Some("Unsuitable input detected. Please check the detected entities on your input and try again with the unsuitable input removed.".into()) }]));

    Ok(())
}

// Asserts that for a given request with input detector using whole_doc_chunker and multiple detections
// returns found detections and no generated text
#[test(tokio::test)]
async fn whole_doc_chunker_input_multiple_detections() -> Result<(), anyhow::Error> {
    // Add generation nlp model header
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add tokenization mock response
    let mock_tokenization_response = TokenizationResults {
        results: Vec::new(),
        token_count: 84,
    };

    // Add tokenization mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_TOKENIZATION_ENDPOINT),
        Mock::new(
            MockRequest::pb(TokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::pb(mock_tokenization_response.clone()),
        ),
    );

    // Add input detection mock response
    let expected_detections = vec![ContentAnalysisResponse {
        start: 45,
        end: 58,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into()),
        score: 1.0,
        evidence: None,
    }, ContentAnalysisResponse {
        start: 71,
        end: 82,
        text: "this other one".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into()),
        score: 1.0,
        evidence: None,
    }];

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
                ],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([
                vec![expected_detections[0].clone(),
                    expected_detections[1].clone()],
            ]),
        ),
    );

    // Configure mock servers
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert!(results.generated_text == None);
    assert!(results.token_classification_results
        == TextGenTokenClassificationResults {
            input: Some(vec![TokenClassificationResult {
                start: expected_detections[0].start as u32,
                end: expected_detections[0].end as u32,
                word: expected_detections[0].text.clone(),
                entity: expected_detections[0].detection.clone(),
                entity_group: expected_detections[0].detection_type.clone(),
                detector_id: expected_detections[0].detector_id.clone(),
                score: expected_detections[0].score,
                token_count: None
            }, TokenClassificationResult {
                start: expected_detections[1].start as u32,
                end: expected_detections[1].end as u32,
                word: expected_detections[1].text.clone(),
                entity: expected_detections[1].detection.clone(),
                entity_group: expected_detections[1].detection_type.clone(),
                detector_id: expected_detections[1].detector_id.clone(),
                score: expected_detections[1].score,
                token_count: None
            }]),
            output: None,
        });
    assert!(results.input_token_count == mock_tokenization_response.token_count as u32);
    assert!(results.warnings == Some(vec![DetectionWarning { 
        id: Some(DetectionWarningReason::UnsuitableInput), 
        message: Some("Unsuitable input detected. Please check the detected entities on your input and try again with the unsuitable input removed.".into()) }]));


    Ok(())
}

// Asserts that for a given request with input detector using whole_doc_chunker returning 503 error
// returns code 503 for failed detector request
#[test(tokio::test)]
async fn whole_doc_chunker_input_detector_returns_503() -> Result<(), anyhow::Error> {
    // Add 503 expected input detection mock response
    let expected_detector_error = DetectorError {
        code: 503,
        message: "The detector service is overloaded.".into(),
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 503".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::SERVICE_UNAVAILABLE),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 503".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::SERVICE_UNAVAILABLE);
    assert!(results.details == format!("detector request failed for `{}`: {}", DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, expected_detector_error.message));

    Ok(())
}

// Asserts that for a given request with input detector using whole_doc_chunker returning 404 error
// returns code 404 for failed detector request
#[test(tokio::test)]
async fn whole_doc_chunker_input_detector_returns_404() -> Result<(), anyhow::Error> {
    // Add 404 expected input detection mock response
    let expected_detector_error = DetectorError {
        code: 404,
        message: "Not found.".into(),
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 404".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::NOT_FOUND),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 404".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::NOT_FOUND);
    assert!(results.details == format!("detector request failed for `{}`: {}", DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, expected_detector_error.message));

    Ok(())
}

// Asserts that for a given request with input detector using whole_doc_chunker returning 500 error
// returns code 500 for failed detector request
#[test(tokio::test)]
async fn whole_doc_chunker_input_detector_returns_500() -> Result<(), anyhow::Error> {
    // Add 500 expected input detection mock response
    let expected_detector_error = DetectorError {
        code: 500,
        message: "Internal detector error.".into(),
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 500".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::INTERNAL_SERVER_ERROR),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 500".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::INTERNAL_SERVER_ERROR);
    assert!(results.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

// Asserts that for a given request with input detector using whole_doc_chunker returning noncompliant response
// returns code 500 Internal Server Error
#[test(tokio::test)]
async fn whole_doc_chunker_input_detector_noncompliant_returns_500() -> Result<(), anyhow::Error> {
    // Add noncompliant expected input detection mock response
    let noncompliant_detector_response = serde_json::json!({
        "detections": true,
    });

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 500".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&noncompliant_detector_response).with_code(StatusCode::OK),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 500".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::INTERNAL_SERVER_ERROR);
    assert!(results.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

// Asserts that for a given request with output detector using whole_doc_chunker and a detection 
// returns found detection and generated text
#[test(tokio::test)]
async fn whole_doc_chunker_output_detection() -> Result<(), anyhow::Error> {
    // Add generation nlp header
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add generation mock response
    let mock_generation_response = GeneratedTextResult {
        generated_text: "This sentence does not have a detection. But <this one does>.".into(),
        ..Default::default()
    };

    // Add generation mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(TextGenerationTaskRequest {
                text: "Generate two sentences, one that does not have angle brackets detection, and another one that does have".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::pb(mock_generation_response.clone()),
        ),
    );

    // Add output detection mock response
    let expected_detection = ContentAnalysisResponse {
        start: 46,
        end: 59,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into()),
        score: 1.0,
        evidence: None,
    };

    // Add output detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This sentence does not have a detection. But <this one does>.".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([
                vec![expected_detection.clone()],
            ]),
        ),
    );

    // Configure mock servers
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "Generate two sentences, one that does not have angle brackets detection, and another one that does have".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())])
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert!(results.generated_text == Some("This sentence does not have a detection. But <this one does>.".into()));
    assert!(results.token_classification_results
            == TextGenTokenClassificationResults {
                input: None,
                output: Some(vec![TokenClassificationResult {
                    start: expected_detection.start as u32,
                    end: expected_detection.end as u32,
                    word: expected_detection.text,
                    entity: expected_detection.detection,
                    entity_group: expected_detection.detection_type,
                    detector_id: expected_detection.detector_id,
                    score: expected_detection.score,
                    token_count: None
                }])
            });

    Ok(())
}

// Asserts that for a given request with output detector using whole_doc_chunker and multiple detections
// returns found detections and generated text
#[test(tokio::test)]
async fn whole_doc_chunker_output_multiple_detections() -> Result<(), anyhow::Error> {
    // Add chunker header
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    // Add chunker tokenization for input mock
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::new(&Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "Generate three sentences, one that does not have an angle brackets detection, and another two that does have.".to_string(),
            })
            .with_headers(chunker_headers.clone()),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 109,
                        text: "Generate three sentences, one that does not have an angle brackets detection, and another two does have.".to_string(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );

    // Add chunker tokenization for generated text mock
    chunker_mocks.insert(
        MockPath::new(&Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".to_string(),
            })
            .with_headers(chunker_headers.clone()),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 84,
                        text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".to_string(),
                    }
                ],
                token_count: 0,
            }),
        ),
    );
    
    // Add generation nlp header
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add generation mock response
    let mock_generation_response = GeneratedTextResult {
        generated_text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
        ..Default::default()
    };

    // Add generation mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(TextGenerationTaskRequest {
                text: "Generate three sentences, one that does not have an angle brackets detection, and another two that does have.".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::pb(mock_generation_response.clone()),
        ),
    );

    // Add output detection mock response
    let expected_detections = vec![ContentAnalysisResponse {
        start: 46,
        end: 59,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into()),
        score: 1.0,
        evidence: None,
    }, ContentAnalysisResponse {
        start: 68,
        end: 82,
        text: "this other one".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into()),
        score: 1.0,
        evidence: None,
    }];

    // Add output detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This sentence does not have a detection. But <this one does>. Also <this other one>.".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([
                vec![expected_detections[0].clone(), 
                    expected_detections[1].clone()],
            ]),
        ),
    );

    // Configure mock servers
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let mock_chunker_server = GrpcMockServer::new(chunker_id.into(), chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        Some(vec![mock_chunker_server]),
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "Generate three sentences, one that does not have an angle brackets detection, and another two that does have.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())])
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert!(results.generated_text == Some("This sentence does not have a detection. But <this one does>. Also <this other one>.".into()));
    assert!(results.token_classification_results
            == TextGenTokenClassificationResults {
                input: None,
                output: Some(vec![TokenClassificationResult {
                    start: expected_detections[0].start as u32,
                    end: expected_detections[0].end as u32,
                    word: expected_detections[0].text.clone(),
                    entity: expected_detections[0].detection.clone(),
                    entity_group: expected_detections[0].detection_type.clone(),
                    detector_id: expected_detections[0].detector_id.clone(),
                    score: expected_detections[0].score,
                    token_count: None
                }, TokenClassificationResult {
                    start: expected_detections[1].start as u32,
                    end: expected_detections[1].end as u32,
                    word: expected_detections[1].text.clone(),
                    entity: expected_detections[1].detection.clone(),
                    entity_group: expected_detections[1].detection_type.clone(),
                    detector_id: expected_detections[1].detector_id.clone(),
                    score: expected_detections[1].score,
                    token_count: None
                }])
            });

    Ok(())
}

// Asserts that for a given request with output detector using whole_doc_chunker returning 503 error
// returns code 503 for failed detector request
#[test(tokio::test)]
async fn whole_doc_chunker_output_detector_returns_503() -> Result<(), anyhow::Error> {
    // Add 503 expected output detection mock response
    let expected_detector_error = DetectorError {
        code: 503,
        message: "The detector service is overloaded.".into(),
    };

    // Add output detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 503".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::SERVICE_UNAVAILABLE),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 503".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::SERVICE_UNAVAILABLE);
    assert!(results.details == format!("detector request failed for `{}`: {}",DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, expected_detector_error.message));

    Ok(())
}

// Asserts that for a given request with output detector using whole_doc_chunker returning 404 error
// returns code 404 for failed detector request
#[test(tokio::test)]
async fn whole_doc_chunker_output_detector_returns_404() -> Result<(), anyhow::Error> {
    // Add 404 expected output detection mock response
    let expected_detector_error = DetectorError {
        code: 404,
        message: "Not found.".into(),
    };

    // Add output detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 404".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::NOT_FOUND),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 404".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::NOT_FOUND);
    assert!(results.details == format!("detector request failed for `{}`: {}", DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, expected_detector_error.message));

    Ok(())
}

// Asserts that for a given request with output detector using whole_doc_chunker returning 500 error
// returns code 500 for failed detector request
#[test(tokio::test)]
async fn whole_doc_chunker_output_detector_returns_500() -> Result<(), anyhow::Error> {
    // Add 500 expected output detection mock response
    let expected_detector_error = DetectorError {
        code: 500,
        message: "Internal detector error.".into(),
    };

    // Add output detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 500".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::INTERNAL_SERVER_ERROR),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 500".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::INTERNAL_SERVER_ERROR);
    assert!(results.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

// Asserts that for a given request with input detector using whole_doc_chunker and generation returning 500
// returns code 500 Internal Server Error
#[test(tokio::test)]
async fn whole_doc_chunker_generation_error_returns_500() -> Result<(), anyhow::Error> {
    // Add generation nlp header
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add generation mock with internal server error response
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(TokenizationTaskRequest {
                text: "This should return a 500".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::empty().with_code(StatusCode::INTERNAL_SERVER_ERROR),
        ),
    );

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 500".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([Vec::<ContentAnalysisResponse>::new()]),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(generation_server),
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 500".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::INTERNAL_SERVER_ERROR);
    assert!(results.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

// Asserts that for a given request with output detector using whole_doc_chunker returning noncompliant response
// returns code 500 Internal Server Error
#[test(tokio::test)]
async fn whole_doc_chunker_output_detector_noncompliant_returns_500() -> Result<(), anyhow::Error> {
    // Add noncompliant expected output detection mock response
    let noncompliant_detector_response = serde_json::json!({
        "detections": true,
    });

    // Add output detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 500".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&noncompliant_detector_response).with_code(StatusCode::OK),
        ),
    );

    // Add chunker header
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    // Add chunker tokenization mock
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::new(&Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This should return a 500".to_string(),
            })
            .with_headers(chunker_headers.clone()),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 24,
                        text: "This should return a 500".to_string(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );

    // Add generation nlp header
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add generation mock response
    let mock_generation_response = GeneratedTextResult {
        generated_text: "This should return a 500".into(),
        ..Default::default()
    };

    // Add generation mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(TextGenerationTaskRequest {
                text: "This should return a 500".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::pb(mock_generation_response.clone()),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC, detection_mocks)?;
    let mock_chunker_server = GrpcMockServer::new(chunker_id.into(), chunker_mocks)?;
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        Some(vec![mock_chunker_server]),
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 500".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC.into(), DetectorParams::new())]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::INTERNAL_SERVER_ERROR);
    assert!(results.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

// Asserts that for a given request with input detector using sentence_chunker and no detections 
// returns generated text
#[test(tokio::test)]
async fn sentence_chunker_input_no_detections() -> Result<(), anyhow::Error> {
    // Add generation nlp model header
    let mut headers = HeaderMap::new();
    headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add generation mock response
    let expected_response = GeneratedTextResult {
            generated_text: "I am great!".into(),
            ..Default::default()
        };

    // Add generation mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(TextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb(expected_response.clone()),
        ),
    );

    // Add input detector mock
    let mut detector_mocks = MockSet::new();
    detector_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec![
                    "Hi there! How are you?".into()
                ],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![
                Vec::<ContentAnalysisResponse>::new(),
            ]),
        ),
    );

    // Configure mock servers
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, detector_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        None,
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: None,
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert!(results.generated_text == Some(expected_response.generated_text));
    assert!(results.warnings == None);

    Ok(())
}

// Asserts that for a given request with input detector using sentence_chunker and a detection 
// returns found detection and no generated text
#[test(tokio::test)]
async fn sentence_chunker_input_detection() -> Result<(), anyhow::Error> {

    // Add chunker header
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    // Add chunker tokenization mock
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::new(&Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".to_string(),
            })
            .with_headers(chunker_headers),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 40,
                        text: "This sentence does not have a detection.".to_string(),
                    },
                    Token {
                        start: 41,
                        end: 61,
                        text: "But <this one does>.".to_string(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );
    
    // Add generation nlp header
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add tokenization mock response
    let mock_tokenization_response = TokenizationResults {
        results: Vec::new(),
        token_count: 61,
    };

    // Add tokenization mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_TOKENIZATION_ENDPOINT),
        Mock::new(
            MockRequest::pb(TokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::pb(mock_tokenization_response.clone()),
        ),
    );

    // Add input detection mock response
    let expected_detection = ContentAnalysisResponse {
        start: 5,
        end: 18,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into()),
        score: 1.0,
        evidence: None,
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This sentence does not have a detection.".into(), 
                    "But <this one does>.".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([
                vec![],
                vec![expected_detection.clone()],
            ]),
        ),
    );

    // Configure mock servers
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let mock_chunker_server = GrpcMockServer::new(chunker_id.into(), chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        Some(vec![mock_chunker_server]),
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This sentence does not have a detection. But <this one does>.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert!(results.generated_text == None);
    assert!(results.token_classification_results
            == TextGenTokenClassificationResults {
                input: Some(vec![TokenClassificationResult {
                    start: 46 as u32,
                    end: 59 as u32,
                    word: expected_detection.text,
                    entity: expected_detection.detection,
                    entity_group: expected_detection.detection_type,
                    detector_id: expected_detection.detector_id,
                    score: expected_detection.score,
                    token_count: None
                }]),
                output: None
            });
    assert!(results.input_token_count == mock_tokenization_response.token_count as u32);
    assert!(results.warnings == Some(vec![DetectionWarning { 
        id: Some(DetectionWarningReason::UnsuitableInput), 
        message: Some("Unsuitable input detected. Please check the detected entities on your input and try again with the unsuitable input removed.".into())
    }]));

    Ok(())
}

// Asserts that for a given request with input detector using sentence_chunker and multiple detections
// returns found detections and no generated text
#[test(tokio::test)]
async fn sentence_chunker_input_multiple_detections() -> Result<(), anyhow::Error> {

    // Add chunker header
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    // Add chunker tokenization mock
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::new(&Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".to_string(),
            })
            .with_headers(chunker_headers),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 40,
                        text: "This sentence does not have a detection.".to_string(),
                    },
                    Token {
                        start: 41,
                        end: 61,
                        text: "But <this one does>.".to_string(),
                    },
                    Token {
                        start: 62,
                        end: 84,
                        text: "Also <this other one>.".to_string(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );
    
    // Add generation nlp header
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add tokenization mock response
    let mock_tokenization_response = TokenizationResults {
        results: Vec::new(),
        token_count: 84,
    };

    // Add tokenization mock 
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_TOKENIZATION_ENDPOINT),
        Mock::new(
            MockRequest::pb(TokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::pb(mock_tokenization_response.clone()),
        ),
    );

    // Add input detection mock response
    let expected_detections = vec![ContentAnalysisResponse {
        start: 5,
        end: 18,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into()),
        score: 1.0,
        evidence: None,
    }, ContentAnalysisResponse {
        start: 6,
        end: 20,
        text: "this other one".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into()),
        score: 1.0,
        evidence: None,
    }];

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This sentence does not have a detection.".into(), 
                    "But <this one does>.".into(),
                    "Also <this other one>.".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([
                vec![],
                vec![expected_detections[0].clone()],
                vec![expected_detections[1].clone()],
            ]),
        ),
    );

    // Configure mock servers
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let mock_chunker_server = GrpcMockServer::new(chunker_id.into(), chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        Some(vec![mock_chunker_server]),
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert!(results.generated_text == None);
    assert!(results.token_classification_results
        == TextGenTokenClassificationResults {
            input: Some(vec![TokenClassificationResult {
                start: 46 as u32,
                end: 59 as u32,
                word: expected_detections[0].text.clone(),
                entity: expected_detections[0].detection.clone(),
                entity_group: expected_detections[0].detection_type.clone(),
                detector_id: expected_detections[0].detector_id.clone(),
                score: expected_detections[0].score,
                token_count: None
            }, TokenClassificationResult {
                start: 68 as u32,
                end: 82 as u32,
                word: expected_detections[1].text.clone(),
                entity: expected_detections[1].detection.clone(),
                entity_group: expected_detections[1].detection_type.clone(),
                detector_id: expected_detections[1].detector_id.clone(),
                score: expected_detections[1].score,
                token_count: None
            }]),
            output: None,
        });

    Ok(())
}

// Asserts that for a given request with input detector using sentence_chunker and chunker returning 500
// returns code 500 Internal Server Error
#[test(tokio::test)]
async fn sentence_chunker_input_chunker_error_returns_500() -> Result<(), anyhow::Error> {
    // Add chunker header
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    // Add chunker tokenization mock with internal server error response
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::post(CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This should return a 500.".into(),
            })
            .with_headers(chunker_headers),
            MockResponse::empty().with_code(StatusCode::INTERNAL_SERVER_ERROR),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_chunker_server = GrpcMockServer::new(chunker_id.into(), chunker_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        None,
        Some(vec![mock_chunker_server]),
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "This should return a 500.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::INTERNAL_SERVER_ERROR);
    assert!(results.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

// Asserts that for a given request with output detector using sentence_chunker and a detection 
// returns found detection and generated text
#[test(tokio::test)]
async fn sentence_chunker_output_detection() -> Result<(), anyhow::Error> {

    // Add chunker header
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    // Add chunker tokenization for input mock 
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::new(&Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "Generate two sentences, one that does not have an angle brackets detection, and a another one that does have.".to_string(),
            })
            .with_headers(chunker_headers.clone()),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 102,
                        text: "Generate two sentences, one that does not have an angle brackets detection, and another that does have.".to_string(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );

    // Add chunker tokenization for generated text mock
    chunker_mocks.insert(
        MockPath::new(&Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".to_string(),
            })
            .with_headers(chunker_headers.clone()),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 40,
                        text: "This sentence does not have a detection.".to_string(),
                    },
                    Token {
                        start: 41,
                        end: 61,
                        text: "But <this one does>.".to_string(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );
    
    // Add generation nlp header
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add generation mock response
    let mock_generation_response = GeneratedTextResult {
        generated_text: "This sentence does not have a detection. But <this one does>.".into(),
        ..Default::default()
    };

    // Add generation mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(TextGenerationTaskRequest {
                text: "Generate two sentences, one that does not have angle brackets detection, and another one that does have.".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::pb(mock_generation_response.clone()),
        ),
    );

    // Add output detection mock response
    let expected_detection = ContentAnalysisResponse {
        start: 5,
        end: 18,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into()),
        score: 1.0,
        evidence: None,
    };

    // Add output detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This sentence does not have a detection.".into(),
                    "But <this one does>.".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([
                vec![],
                vec![expected_detection.clone()],
            ]),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let mock_chunker_server = GrpcMockServer::new(chunker_id.into(), chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        Some(vec![mock_chunker_server]),
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "Generate two sentences, one that does not have angle brackets detection, and another one that does have.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(), DetectorParams::new())])
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert!(results.generated_text == Some("This sentence does not have a detection. But <this one does>.".into()));
    assert!(results.token_classification_results
            == TextGenTokenClassificationResults {
                input: None,
                output: Some(vec![TokenClassificationResult {
                    start: 46 as u32,
                    end: 59 as u32,
                    word: expected_detection.text,
                    entity: expected_detection.detection,
                    entity_group: expected_detection.detection_type,
                    detector_id: expected_detection.detector_id,
                    score: expected_detection.score,
                    token_count: None
                }])
            });
    
    Ok(())
}

// Asserts that for a given request with output detector using sentence_chunker and multiple detections
// returns found detections and generated text
#[test(tokio::test)]
async fn sentence_chunker_output_multiple_detections() -> Result<(), anyhow::Error> {
    // Add chunker header
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    // Add chunker tokenization for input mock
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::new(&Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "Generate three sentences, one that does not have an angle brackets detection, and another two that does have.".to_string(),
            })
            .with_headers(chunker_headers.clone()),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 109,
                        text: "Generate three sentences, one that does not have an angle brackets detection, and another two does have.".to_string(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );

    // Add chunker tokenization for generated text mock
    chunker_mocks.insert(
        MockPath::new(&Method::POST, CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".to_string(),
            })
            .with_headers(chunker_headers.clone()),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 40,
                        text: "This sentence does not have a detection.".to_string(),
                    },
                    Token {
                        start: 41,
                        end: 61,
                        text: "But <this one does>.".to_string(),
                    },
                    Token {
                        start: 62,
                        end: 84,
                        text: "Also <this other one>.".to_string(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );
    
    // Add generation nlp header
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add generation mock response
    let mock_generation_response = GeneratedTextResult {
        generated_text: "This sentence does not have a detection. But <this one does>. Also <this other one>.".into(),
        ..Default::default()
    };

    // Add generation mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(TextGenerationTaskRequest {
                text: "Generate three sentences, one that does not have an angle brackets detection, and another two that does have.".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::pb(mock_generation_response.clone()),
        ),
    );

    // Add output detection mock response
    let expected_detections = vec![ContentAnalysisResponse {
        start: 5,
        end: 18,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into()),
        score: 1.0,
        evidence: None,
    }, ContentAnalysisResponse {
        start: 6,
        end: 20,
        text: "this other one".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into()),
        score: 1.0,
        evidence: None,
    }];

    // Add output detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::new(&Method::POST, TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This sentence does not have a detection.".into(),
                    "But <this one does>.".into(), "Also <this other one>.".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([
                vec![],
                vec![expected_detections[0].clone()],
                vec![expected_detections[1].clone()],
            ]),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let mock_chunker_server = GrpcMockServer::new(chunker_id.into(), chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, detection_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        Some(vec![mock_detector_server]),
        Some(vec![mock_chunker_server]),
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "Generate three sentences, one that does not have an angle brackets detection, and another two that does have.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(), DetectorParams::new())])
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::OK);
    let results = response.json::<ClassifiedGeneratedTextResult>().await?;
    assert!(results.generated_text == Some("This sentence does not have a detection. But <this one does>. Also <this other one>.".into()));
    assert!(results.token_classification_results
            == TextGenTokenClassificationResults {
                input: None,
                output: Some(vec![TokenClassificationResult {
                    start: 46 as u32,
                    end: 59 as u32,
                    word: expected_detections[0].text.clone(),
                    entity: expected_detections[0].detection.clone(),
                    entity_group: expected_detections[0].detection_type.clone(),
                    detector_id: expected_detections[0].detector_id.clone(),
                    score: expected_detections[0].score,
                    token_count: None
                }, TokenClassificationResult {
                    start: 68 as u32,
                    end: 82 as u32,
                    word: expected_detections[1].text.clone(),
                    entity: expected_detections[1].detection.clone(),
                    entity_group: expected_detections[1].detection_type.clone(),
                    detector_id: expected_detections[1].detector_id.clone(),
                    score: expected_detections[1].score,
                    token_count: None
                }])
            });

    Ok(())
}

// Asserts that for a given request with output detector using sentence_chunker and output chunker returning 500
// returns code 500 Internal Server Error
#[test(tokio::test)]
async fn sentence_chunker_output_chunker_error_returns_500() -> Result<(), anyhow::Error> {
    // Add chunker header
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    // Add chunker tokenization for input mock
    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::post(CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "Hi there! How are you?".into(),
            })
            .with_headers(chunker_headers.clone()),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 22,
                        text: "Hi there! How are you?".into(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );

    // Add chunker tokenization for generation mock with error
    chunker_mocks.insert(
        MockPath::post(CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "I am great!".into(),
            })
            .with_headers(chunker_headers.clone()),
            MockResponse::empty().with_code(StatusCode::INTERNAL_SERVER_ERROR),
        ),
    );

    // Add generation nlp header
    let mut gen_headers = HeaderMap::new();
    gen_headers.insert(GENERATION_NLP_MODEL_ID_HEADER_NAME, MODEL_ID.parse().unwrap());

    // Add generation mock response
    let mock_generation_response = GeneratedTextResult {
        generated_text: "I am great!".into(),
        ..Default::default()
    };

    // Add generation mock
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::new(&Method::POST, GENERATION_NLP_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(TextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            })
            .with_headers(gen_headers.clone()),
            MockResponse::pb(mock_generation_response.clone()),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_chunker_server = GrpcMockServer::new(chunker_id.into(), chunker_mocks)?;
    let mock_generation_server = GrpcMockServer::new("generation_server", generation_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(mock_generation_server),
        None,
        None,
        Some(vec![mock_chunker_server]),
    )
    .await?;

    // Orchestrator request with unary response
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: MODEL_ID.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: None,
                output: Some(GuardrailsConfigOutput {
                    models: HashMap::from([(DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE.into(), DetectorParams::new())]),
                }),
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Assertions
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code == StatusCode::INTERNAL_SERVER_ERROR);
    assert!(results.details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

// Asserts that for a given request with non existing fields
// returns code 422 unknown field error
#[test(tokio::test)]
async fn request_with_extra_fields_returns_422() -> Result<(), anyhow::Error> {

    // Start orchestrator server and its dependencies
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        None,
        None,
        None,
        None,
    )
    .await?;

    // Orchestrator request with non existing field
    let response = orchestrator_server
        .post(ORCHESTRATOR_UNARY_ENDPOINT)
        .json(&serde_json::json!({
            "model_id": MODEL_ID,
            "inputs": "This request does not comply with the orchestrator API",
            "guardrail_config": {},
            "text_gen_parameters": {},
            "non_existing_field": "random value"
        }))
        .send()
        .await?;

    // Assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);
    let results = response.json::<OrchestratorError>().await?;
    assert!(results.code ==StatusCode::UNPROCESSABLE_ENTITY);
    assert!(results.details.starts_with("non_existing_field: unknown field `non_existing_field`"));

    Ok(())
}
