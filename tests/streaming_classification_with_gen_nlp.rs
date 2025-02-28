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
use test_log::test;

use common::{
    chunker::{CHUNKER_MODEL_ID_HEADER_NAME, CHUNKER_NAME_SENTENCE, CHUNKER_UNARY_ENDPOINT},
    detectors::{
        DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE, DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC,
        TEXT_CONTENTS_DETECTOR_ENDPOINT,
    },
    errors::{DetectorError, OrchestratorError},
    generation::{
        GENERATION_NLP_MODEL_ID_HEADER_NAME, GENERATION_NLP_STREAMING_ENDPOINT,
        GENERATION_NLP_TOKENIZATION_ENDPOINT,
    },
    orchestrator::{
        SseStream, TestOrchestratorServer, ORCHESTRATOR_CONFIG_FILE_PATH,
        ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE, ORCHESTRATOR_STREAMING_ENDPOINT,
    },
};
use fms_guardrails_orchestr8::{
    clients::detector::{ContentAnalysisRequest, ContentAnalysisResponse},
    models::{
        ClassifiedGeneratedTextStreamResult, DetectionWarning, DetectorParams, GuardrailsConfig,
        GuardrailsConfigInput, GuardrailsHttpRequest, TextGenTokenClassificationResults,
        TokenClassificationResult,
    },
    pb::{
        caikit::runtime::{
            chunkers::ChunkerTokenizationTaskRequest,
            nlp::{ServerStreamingTextGenerationTaskRequest, TokenizationTaskRequest},
        },
        caikit_data_model::nlp::{GeneratedTextStreamResult, Token, TokenizationResults},
    },
};
use futures::StreamExt;
use mocktail::{prelude::*, utils::find_available_port};
use tracing::debug;

pub mod common;

// To troubleshoot tests with response deserialization errors, the following code
// snippet is recommended:
// // Example showing how to create an event stream from a bytes stream.
// let mut events = Vec::new();
// let mut event_stream = response.bytes_stream().eventsource();
// while let Some(event) = event_stream.next().await {
//     match event {
//         Ok(event) => {
//             if event.data == "[DONE]" {
//                 break;
//             }
//             println!("recv: {event:?}");
//             events.push(event.data);
//         }
//         Err(_) => {
//             panic!("received error from event stream");
//         }
//     }
// }
// println!("{events:?}");

/// Asserts that given a request with no detectors configured returns the text generated
/// by the model.
#[test(tokio::test)]
async fn test_no_detectors() -> Result<(), anyhow::Error> {
    // Add generation mock
    let model_id = "my-super-model-8B";
    let mut headers = HeaderMap::new();
    headers.insert(
        GENERATION_NLP_MODEL_ID_HEADER_NAME,
        model_id.parse().unwrap(),
    );

    let expected_response = vec![
        GeneratedTextStreamResult {
            generated_text: "I".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " am".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " great!".into(),
            ..Default::default()
        },
    ];

    let mut mocks = MockSet::new();
    mocks.insert(
        MockPath::post(GENERATION_NLP_STREAMING_ENDPOINT),
        Mock::new(
            MockRequest::pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb_stream(expected_response.clone()),
        ),
    );

    // Configure mock servers
    let generation_server = GrpcMockServer::new("nlp", mocks)?;

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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: None,
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Collects stream results
    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!(?messages);

    // assertions
    assert!(messages.len() == 3);
    assert!(messages[0].generated_text == Some("I".into()));
    assert!(messages[1].generated_text == Some(" am".into()));
    assert!(messages[2].generated_text == Some(" great!".into()));

    Ok(())
}

/// Asserts that the generated text is returned when an input detector configured
/// with the whole_doc_chunker finds no detections.
#[test(tokio::test)]
async fn test_input_detector_whole_doc_no_detections() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["Hi there! How are you?".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([Vec::<ContentAnalysisResponse>::new()]),
        ),
    );

    // Add generation mock
    let model_id = "my-super-model-8B";
    let mut headers = HeaderMap::new();
    headers.insert(
        GENERATION_NLP_MODEL_ID_HEADER_NAME,
        model_id.parse().unwrap(),
    );

    let expected_response = vec![
        GeneratedTextStreamResult {
            generated_text: "I".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " am".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " great!".into(),
            ..Default::default()
        },
    ];

    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::post(GENERATION_NLP_STREAMING_ENDPOINT),
        Mock::new(
            MockRequest::pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb_stream(expected_response.clone()),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
    let generation_server = GrpcMockServer::new("nlp", generation_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Test custom SseStream wrapper
    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 3);
    assert!(messages[0].generated_text == Some("I".into()));
    assert!(messages[0].token_classification_results.input == None);

    assert!(messages[1].generated_text == Some(" am".into()));
    assert!(messages[1].token_classification_results.input == None);

    assert!(messages[2].generated_text == Some(" great!".into()));
    assert!(messages[2].token_classification_results.input == None);

    Ok(())
}

/// Asserts that the generated text is returned when an input detector configured
/// with a sentence chunker finds no detections.
#[test(tokio::test)]
async fn test_input_detector_sentence_chunker_no_detections() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;

    // Add input chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::post(CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "Hi there! How are you?".into(),
            })
            .with_headers(chunker_headers),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 9,
                        text: "Hi there!".into(),
                    },
                    Token {
                        start: 10,
                        end: 22,
                        text: " How are you?".into(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["Hi there!".into(), " How are you?".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([
                Vec::<ContentAnalysisResponse>::new(),
                Vec::<ContentAnalysisResponse>::new(),
            ]),
        ),
    );

    // Add generation mock
    let model_id = "my-super-model-8B";
    let mut headers = HeaderMap::new();
    headers.insert(
        GENERATION_NLP_MODEL_ID_HEADER_NAME,
        model_id.parse().unwrap(),
    );

    let expected_response = vec![
        GeneratedTextStreamResult {
            generated_text: "I".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " am".into(),
            ..Default::default()
        },
        GeneratedTextStreamResult {
            generated_text: " great!".into(),
            ..Default::default()
        },
    ];

    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::post(GENERATION_NLP_STREAMING_ENDPOINT),
        Mock::new(
            MockRequest::pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb_stream(expected_response.clone()),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_chunker_server = GrpcMockServer::new(chunker_id, chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
    let generation_server = GrpcMockServer::new("nlp", generation_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(generation_server),
        None,
        Some(vec![mock_detector_server]),
        Some(vec![mock_chunker_server]),
    )
    .await?;

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // Test custom SseStream wrapper
    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 3);
    assert!(messages[0].generated_text == Some("I".into()));
    assert!(messages[0].token_classification_results.input == None);

    assert!(messages[1].generated_text == Some(" am".into()));
    assert!(messages[1].token_classification_results.input == None);

    assert!(messages[2].generated_text == Some(" great!".into()));
    assert!(messages[2].token_classification_results.input == None);

    Ok(())
}

/// Asserts that detections found by an input detector configured with the whole_doc_chunker
/// are returned.
#[test(tokio::test)]
async fn test_input_detector_whole_doc_with_detections() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;
    let mock_detection_response = ContentAnalysisResponse {
        start: 46,
        end: 59,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(detector_name.into()),
        score: 1.0,
        evidence: None,
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection. But <this one does>.".into(),
                ],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([vec![mock_detection_response.clone()]]),
        ),
    );

    // Add generation mock for input token count
    let model_id = "my-super-model-8B";
    let mock_tokenization_response = TokenizationResults {
        results: Vec::new(),
        token_count: 61,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        GENERATION_NLP_MODEL_ID_HEADER_NAME,
        model_id.parse().unwrap(),
    );
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::post(GENERATION_NLP_TOKENIZATION_ENDPOINT),
        Mock::new(
            MockRequest::pb(TokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb(mock_tokenization_response.clone()),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
    let generation_server = GrpcMockServer::new("nlp", generation_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This sentence does not have a detection. But <this one does>.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Test custom SseStream wrapper
    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 1);
    assert!(messages[0].generated_text == None);
    assert!(
        messages[0].token_classification_results
            == TextGenTokenClassificationResults {
                input: Some(vec![TokenClassificationResult {
                    start: mock_detection_response.start as u32,
                    end: mock_detection_response.end as u32,
                    word: mock_detection_response.text,
                    entity: mock_detection_response.detection,
                    entity_group: mock_detection_response.detection_type,
                    detector_id: mock_detection_response.detector_id,
                    score: mock_detection_response.score,
                    token_count: None
                }]),
                output: None
            }
    );
    assert!(messages[0].input_token_count == mock_tokenization_response.token_count as u32);
    assert!(messages[0].warnings == Some(vec![DetectionWarning{ id: Some(fms_guardrails_orchestr8::models::DetectionWarningReason::UnsuitableInput), message: Some("Unsuitable input detected. Please check the detected entities on your input and try again with the unsuitable input removed.".into()) }]));

    Ok(())
}

/// Asserts that detections found by an input detector configured with a sentence chunker
/// are returned.
#[test(tokio::test)]
async fn test_input_detector_sentence_chunker_with_detections() -> Result<(), anyhow::Error> {
    // Add chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::post(CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".into(),
            })
            .with_headers(chunker_headers),
            MockResponse::pb(TokenizationResults {
                results: vec![
                    Token {
                        start: 0,
                        end: 40,
                        text: "This sentence does not have a detection.".into(),
                    },
                    Token {
                        start: 41,
                        end: 61,
                        text: "But <this one does>.".into(),
                    },
                ],
                token_count: 0,
            }),
        ),
    );

    // Add input detection mock
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let mock_detection_response = ContentAnalysisResponse {
        start: 5,
        end: 18,
        text: "this one does".into(),
        detection: "has_angle_brackets".into(),
        detection_type: "angle_brackets".into(),
        detector_id: Some(detector_name.into()),
        score: 1.0,
        evidence: None,
    };
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec![
                    "This sentence does not have a detection.".into(),
                    "But <this one does>.".into(),
                ],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(vec![vec![], vec![mock_detection_response.clone()]]),
        ),
    );

    // Add generation mock for input token count
    let model_id = "my-super-model-8B";
    let mock_tokenization_response = TokenizationResults {
        results: Vec::new(),
        token_count: 61,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        GENERATION_NLP_MODEL_ID_HEADER_NAME,
        model_id.parse().unwrap(),
    );
    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::post(GENERATION_NLP_TOKENIZATION_ENDPOINT),
        Mock::new(
            MockRequest::pb(TokenizationTaskRequest {
                text: "This sentence does not have a detection. But <this one does>.".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::pb(mock_tokenization_response.clone()),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_chunker_server = GrpcMockServer::new(chunker_id, chunker_mocks)?;
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
    let generation_server = GrpcMockServer::new("nlp", generation_mocks)?;
    let orchestrator_server = TestOrchestratorServer::run(
        ORCHESTRATOR_CONFIG_FILE_PATH,
        find_available_port().unwrap(),
        find_available_port().unwrap(),
        Some(generation_server),
        None,
        Some(vec![mock_detector_server]),
        Some(vec![mock_chunker_server]),
    )
    .await?;

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This sentence does not have a detection. But <this one does>.".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Test custom SseStream wrapper
    let sse_stream: SseStream<ClassifiedGeneratedTextStreamResult> =
        SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 1);
    assert!(messages[0].generated_text == None);
    assert!(
        messages[0].token_classification_results
            == TextGenTokenClassificationResults {
                input: Some(vec![TokenClassificationResult {
                    start: 46 as u32, // index of first token of detected text, relative to the `inputs` string sent in the orchestrator request.
                    end: 59 as u32, // index of last token (+1) of detected text, relative to the `inputs` string sent in the orchestrator request.
                    word: mock_detection_response.text,
                    entity: mock_detection_response.detection,
                    entity_group: mock_detection_response.detection_type,
                    detector_id: mock_detection_response.detector_id,
                    score: mock_detection_response.score,
                    token_count: None
                }]),
                output: None
            }
    );
    assert!(messages[0].input_token_count == mock_tokenization_response.token_count as u32);
    assert!(messages[0].warnings == Some(vec![DetectionWarning{ id: Some(fms_guardrails_orchestr8::models::DetectionWarningReason::UnsuitableInput), message: Some("Unsuitable input detected. Please check the detected entities on your input and try again with the unsuitable input removed.".into()) }]));

    Ok(())
}

/// Asserts that 503 errors returned from detectors are correctly propagated.
#[test(tokio::test)]
async fn test_input_detector_returns_503() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;
    let model_id = "my-super-model-8B";
    let expected_detector_error = DetectorError {
        code: 503,
        message: "The detector service is overloaded.".into(),
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 503".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::SERVICE_UNAVAILABLE),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This should return a 503".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    debug!(?response, "RESPONSE RECEIVED FROM ORCHESTRATOR");

    // Test custom SseStream wrapper
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 1);
    assert!(messages[0].code == 503);
    assert!(
        messages[0].details
            == format!(
                "detector request failed for `{}`: {}",
                detector_name, expected_detector_error.message
            )
    );

    Ok(())
}

/// Asserts that 404 errors returned from detectors are correctly propagated.
#[test(tokio::test)]
async fn test_input_detector_returns_404() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;
    let model_id = "my-super-model-8B";
    let expected_detector_error = DetectorError {
        code: 404,
        message: "Not found.".into(),
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 404".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::NOT_FOUND),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This should return a 404".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    debug!(?response, "RESPONSE RECEIVED FROM ORCHESTRATOR");

    // Test custom SseStream wrapper
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 1);
    assert!(messages[0].code == 404);
    assert!(
        messages[0].details
            == format!(
                "detector request failed for `{}`: {}",
                detector_name, expected_detector_error.message
            )
    );

    Ok(())
}

/// Asserts that 500 errors returned from detectors are correctly propagated.
#[test(tokio::test)]
async fn test_input_detector_returns_500() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;
    let model_id = "my-super-model-8B";
    let expected_detector_error = DetectorError {
        code: 500,
        message: "Internal detector error.".into(),
    };

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["This should return a 500".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&expected_detector_error).with_code(StatusCode::NOT_FOUND),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "This should return a 500".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    debug!(?response, "RESPONSE RECEIVED FROM ORCHESTRATOR");

    // Test custom SseStream wrapper
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 1);
    assert!(messages[0].code == 500);
    assert!(messages[0].details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

/// Asserts error 500 is returned when a detector returns a message that does not comply
/// with the detector API.
#[test(tokio::test)]
async fn test_input_detector_returns_invalid_message() -> Result<(), anyhow::Error> {
    // ensure_global_rustls_state();
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;
    let model_id = "my-super-model-8B";
    let non_compliant_detector_response = serde_json::json!({
        "detections": true,
    });

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec![
                    "The detector will return a message non compliant with the API".into(),
                ],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json(&non_compliant_detector_response).with_code(StatusCode::OK),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "The detector will return a message non compliant with the API".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    debug!(?response, "RESPONSE RECEIVED FROM ORCHESTRATOR");

    // Test custom SseStream wrapper
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 1);
    assert!(messages[0].code == 500);
    assert!(messages[0].details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

/// Asserts error 500 is returned when an input chunker returns an error.
#[test(tokio::test)]
async fn test_input_chunker_returns_an_error() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_SENTENCE;
    let model_id = "my-super-model-8B";

    // Add input chunker mock
    let chunker_id = CHUNKER_NAME_SENTENCE;
    let mut chunker_headers = HeaderMap::new();
    chunker_headers.insert(CHUNKER_MODEL_ID_HEADER_NAME, chunker_id.parse()?);

    let mut chunker_mocks = MockSet::new();
    chunker_mocks.insert(
        MockPath::post(CHUNKER_UNARY_ENDPOINT),
        Mock::new(
            MockRequest::pb(ChunkerTokenizationTaskRequest {
                text: "Hi there! How are you?".into(),
            })
            .with_headers(chunker_headers),
            MockResponse::empty().with_code(StatusCode::INTERNAL_SERVER_ERROR),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_chunker_server = GrpcMockServer::new(chunker_id, chunker_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    debug!("{response:#?}");

    // Test custom SseStream wrapper
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 1);
    assert!(messages[0].code == StatusCode::INTERNAL_SERVER_ERROR);
    assert!(messages[0].details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

// Asserts error 500 is returned when generation server returns an error.
#[test(tokio::test)]
async fn test_generation_server_returns_an_error() -> Result<(), anyhow::Error> {
    let detector_name = DETECTOR_NAME_ANGLE_BRACKETS_WHOLE_DOC;

    // Add input detection mock
    let mut detection_mocks = MockSet::new();
    detection_mocks.insert(
        MockPath::post(TEXT_CONTENTS_DETECTOR_ENDPOINT),
        Mock::new(
            MockRequest::json(ContentAnalysisRequest {
                contents: vec!["Hi there! How are you?".into()],
                detector_params: DetectorParams::new(),
            }),
            MockResponse::json([Vec::<ContentAnalysisResponse>::new()]),
        ),
    );

    // Add generation mock
    let model_id = "my-super-model-8B";
    let mut headers = HeaderMap::new();
    headers.insert(
        GENERATION_NLP_MODEL_ID_HEADER_NAME,
        model_id.parse().unwrap(),
    );

    let mut generation_mocks = MockSet::new();
    generation_mocks.insert(
        MockPath::post(GENERATION_NLP_STREAMING_ENDPOINT),
        Mock::new(
            MockRequest::pb(ServerStreamingTextGenerationTaskRequest {
                text: "Hi there! How are you?".into(),
                ..Default::default()
            })
            .with_headers(headers.clone()),
            MockResponse::empty().with_code(StatusCode::INTERNAL_SERVER_ERROR),
        ),
    );

    // Start orchestrator server and its dependencies
    let mock_detector_server = HttpMockServer::new(detector_name, detection_mocks)?;
    let generation_server = GrpcMockServer::new("nlp", generation_mocks)?;
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&GuardrailsHttpRequest {
            model_id: model_id.into(),
            inputs: "Hi there! How are you?".into(),
            guardrail_config: Some(GuardrailsConfig {
                input: Some(GuardrailsConfigInput {
                    models: HashMap::from([(detector_name.into(), DetectorParams::new())]),
                    masks: None,
                }),
                output: None,
            }),
            text_gen_parameters: None,
        })
        .send()
        .await?;

    // Test custom SseStream wrapper
    let sse_stream: SseStream<OrchestratorError> = SseStream::new(response.bytes_stream());
    let messages = sse_stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    debug!("{messages:#?}");

    // assertions
    assert!(messages.len() == 1);
    assert!(messages[0].code == StatusCode::INTERNAL_SERVER_ERROR);
    assert!(messages[0].details == ORCHESTRATOR_INTERNAL_SERVER_ERROR_MESSAGE);

    Ok(())
}

/// Asserts error 422 is returned when the orchestrator request has extra fields.
#[test(tokio::test)]
async fn test_request_with_extra_fields_returns_422() -> Result<(), anyhow::Error> {
    let model_id = "my-super-model-8B";

    // Run test orchestrator server
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

    // Example orchestrator request with streaming response
    let response = orchestrator_server
        .post(ORCHESTRATOR_STREAMING_ENDPOINT)
        .json(&serde_json::json!({
            "model_id": model_id,
            "inputs": "This request does not comply with the orchestrator API",
            "guardrail_config": {
                "inputs": {},
                "outputs": {}
            },
            "non_existing_field": "random value"
        }))
        .send()
        .await?;

    debug!(?response);

    // assertions
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY);

    let response_body = response.json::<OrchestratorError>().await?;
    assert!(response_body.code == StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response_body
        .details
        .starts_with("non_existing_field: unknown field `non_existing_field`"));

    Ok(())
}
