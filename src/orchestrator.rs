use std::{collections::HashMap, usize};

use crate::{config::{ChunkerConfig, ChunkerType, DetectorConfig, DetectorMap}, models::{GuardrailsConfig, GuardrailsHttpRequest, GuardrailsTextGenerationParameters}, pb::fmaas::{generation_service_server::GenerationService, BatchedTokenizeRequest, TokenizeRequest}, ErrorResponse};
use axum::{
    response::IntoResponse,
    Json,
};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use serde::{Serialize};
use serde_json::{json, Value};
use tokio::{signal};
use tracing::info;
use std::convert::Infallible;

// ========================================== Constants and Dummy Variables ==========================================
const API_PREFIX: &'static str = r#"/api/v1/task"#;

// TODO: Dummy TGIS streaming generation response object - replace later
#[derive(Serialize)]
pub(crate) struct GenerationResponse {
    pub input_token_count: u32,
    pub generated_token_count: u32,
    pub text: String,
    // StopReason.....
}

// TODO: Dummy TGIS tokenization response object - replace later
#[derive(Serialize)]
pub(crate) struct TokenizeResponse {
    pub token_count: u32,
    // ...
}

// TODO: Dummy chunker response objects - replace later
#[derive(Clone, Serialize)]
pub(crate) struct Token {
    pub start: u32,
    pub end: u32,
    pub text: String
}
#[derive(Clone, Serialize)]
pub(crate) struct TokenizationResults {
    pub results: Vec<Token>,
}

#[derive(Clone, Serialize)]
pub(crate) struct TokenizationStreamResult {
    pub results: TokenizationResults,
    pub processed_index: u32,
    pub start_index: u32,
}

// TODO: Dummy detector response objects - replace later
#[derive(Serialize)]
pub(crate) struct DetectorResult {
    pub start: u32,
    pub end: u32,
    pub word: String,
    pub entity: String,
    pub entity_group: String,
    pub score: f32,
    pub token_count: u32,
}
#[derive(Serialize)]
pub(crate) struct DetectorResponse {
    pub results: Vec<DetectorResult>,
}

const DUMMY_RESPONSE: [&'static str; 9] = ["This", "is", "very", "good", "news,", "streaming", "is", "working", "!"];

// ========================================== Handler functions ==========================================

// This is designed to be boot-time validation and does not have to persist here
// The results should get processed before tasks are called to raise an error for config
pub fn preprocess_detector_map(detector_map: DetectorMap) -> Result<(HashMap<String, String>, HashMap<String, Result<ChunkerConfig, ErrorResponse>>), ErrorResponse> {
    // Map detectors to respective chunkers
    let chunkers: HashMap<String, ChunkerConfig> = detector_map.chunkers;
    let detectors: HashMap<String, DetectorConfig> = detector_map.detectors;

    let mut detector_chunker_map: HashMap<String, String> = HashMap::new();
    let mut chunker_map: HashMap<String, Result<ChunkerConfig, ErrorResponse>> = HashMap::new();
    for (detector_name, detector_config) in detectors.into_iter() {
        let chunker_name: String = detector_config.chunker.to_string();
        let result: Result<ChunkerConfig, ErrorResponse> = match chunkers.get(&chunker_name) {
            Some(&v) => Ok(v),
            None => Err(ErrorResponse{error: format!("Detector {detector_name} not configured correctly")})
        };
        chunker_map.insert(chunker_name, result);
        // TODO: chunker_name can't be reused
        detector_chunker_map.insert(detector_name, detector_config.chunker.to_string());
    }
    Ok((detector_chunker_map, chunker_map))
}

// ========================================== Dummy Tasks ==========================================

// API calls - do not have to actually live here

// Unary TGIS call
async fn tgis_unary_call(model_id: String, texts: Vec<String>, text_gen_params: Option<GuardrailsTextGenerationParameters>) -> GenerationResponse {
    GenerationResponse {
        input_token_count: 1,
        generated_token_count: 1, 
        text: "hi".to_string(),
    }
}

// Server streaming TGIS call
// TODO: update payload here
async fn tgis_stream_call(
    Json(tgis_payload): Json<GuardrailsHttpRequest>,
    text_gen_params: Option<GuardrailsTextGenerationParameters>,
    on_message_callback: impl Fn(GenerationResponse) -> Event,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let mut dummy_response_iterator = DUMMY_RESPONSE.iter();

    let mut input_token_count: u32 = 0;
    let stream = async_stream::stream! {
        // Server sending event stream
        while let Some(&token) = dummy_response_iterator.next() {
            let stream_token = GenerationResponse {
                input_token_count: input_token_count,
                generated_token_count: input_token_count, 
                text: token.to_string(),
                
            };
            input_token_count += 1;
            let event = on_message_callback(stream_token);
            yield Ok(event);
        }
    };
    stream
}

// Unary TGIS tokenize call
async fn tokenize_unary_call(model_id: String, texts: Vec<String>) -> TokenizeResponse {
    let mut tokenize_requests: Vec<TokenizeRequest> = vec![];
    for text in texts.iter() {
        let tokenize_request: TokenizeRequest = TokenizeRequest { text: text.to_string() };
        tokenize_requests.push(tokenize_request);
    };

    // Structs have to be filled in, so default to no truncation or extra return fields
    let request: BatchedTokenizeRequest = BatchedTokenizeRequest {
        model_id,
        requests: tokenize_requests,
        return_tokens: false,
        return_offsets: false,
        truncate_input_tokens: 0,
    };
    TokenizeResponse {
        token_count: 9
    }
}

// Unary chunker call
async fn chunker_unary_call(model_id: String, text: String) -> TokenizationResults {
    // unary under the hood
    let token_0 = Token {
        start: 0,
        end: 4,
        text: "This".to_string(),
    };
    let token_1 = Token {
        start: 5,
        end: 7,
        text: "is".to_string(),
    };
    TokenizationResults {
        results: vec![token_0, token_1]
    }
}

// Bidirectional streaming chunker call
async fn chunker_stream_call(model_id: String, texts: Vec<String>, on_message_callback: impl Fn(TokenizationStreamResult) -> Event) -> impl Stream<Item = Result<Event, Infallible>> {
    let token_0 = Token {
        start: 0,
        end: 4,
        text: "This".to_string(),
    };
    let token_1 = Token {
        start: 5,
        end: 7,
        text: "is".to_string(),
    };
    let tok_results = TokenizationResults {
        results: vec![token_0, token_1]
    };
    let mut dummy_response_iterator = DUMMY_RESPONSE.iter();

    let stream = async_stream::stream! {
        // Server sending event stream
        while let Some(&token) = dummy_response_iterator.next() {
            let stream_token = TokenizationStreamResult {
                results: tok_results.clone(),
                processed_index: 1, 
                start_index: 0 
            };
            let event = on_message_callback(stream_token);
            yield Ok(event);
        }
    };
    stream
}

// Unary detector call
// Assume processing on batch (multiple strings) can at least happen
async fn detector_call(detector_id: String, inputs: Vec<String>) -> DetectorResponse {
    // Might need some routing/extra endpoint info to begin with
    let result: DetectorResult = DetectorResult {
        start: 0,
        end: 3,
        word: "moo".to_owned(),
        entity: "cow".to_owned(),
        entity_group: "cow".to_owned(),
        token_count: 1,
        score: 0.5,
    };
    DetectorResponse {
        results: vec![result]
    }
}

// Orchestrator internal logic

fn slice_input(mut user_input: Vec<String>, payload: GuardrailsHttpRequest) -> Vec<String>{
    let input_masks = payload.guardrail_config.unwrap().input.unwrap().masks;
    if input_masks.is_some() {
        let user_input_vec = user_input[0].chars().collect::<Vec<_>>();
        // Extra work for codepoint slicing in Rust
        user_input = vec![];
        for (start, end) in input_masks.into_iter() {
            let mask_string: String = user_input_vec[start..end].iter().cloned().collect::<String>();
            user_input.push(mask_string);
        }
    }
    user_input
}

async fn input_detection(
    input_detectors_models: HashMap<String, HashMap<String, String>>,
    chunker_map: HashMap<String, String>,
    chunker_config_map: HashMap<String, Result<ChunkerConfig, ErrorResponse>>,
    texts: Vec<String>,
) -> Vec<DetectorResponse> {
    // input_detectors_models: model_name: {param: value}
    // Future - parallelize calls
    let mut detector_responses = Vec::new();
    for detector_id in input_detectors_models.keys() {
        if let Some(chunker_id) = chunker_map.get(detector_id){
            for text in texts.iter() {
                // TODO: Get config/type for chunker call
                let tokenization_results = chunker_unary_call(chunker_id.to_string(), text.to_string()).await;
                // Optimize later - chunkers would always be called even if multiple detectors had same chunker
                let mut texts: Vec<String> = Vec::new();
                for token in tokenization_results.results.iter() {
                    texts.push(token.text.to_string())
                }
                detector_responses.push(detector_call(detector_id.to_string(), texts).await)
            }
        } else {
            continue;
        }
    }
    detector_responses
}

// ========================================== Main ==========================================

// In the future probably create DAG/list of tasks to be invoked
pub async fn do_tasks(payload: GuardrailsHttpRequest, detector_hashmaps: (HashMap<String, String>, HashMap<String, Result<ChunkerConfig, ErrorResponse>>),streaming: bool) {
    // TODO: is clone() needed for every payload use? Otherwise move errors since payload has String

    // LLM / text generation model
    let model_id: String = payload.clone().model_id;

    // Original user input text, initialized as vector for type
    // consistency if masks are supplied
    let mut user_input: Vec<String> = vec![payload.clone().inputs];

    // No guardrail_config specified
    if payload.guardrail_config.is_none() {
        // TODO: Just do text gen? Error?
        // This falls through to text gen today but validation is not done
    }

    // Slice up if masks are supplied
    // Whole payload is just passed here to abstract away impl, could be separate task
    // tracked as part of DAG/list instead of function in the future
    user_input = slice_input(user_input, payload.clone());

    // Process detector hashmaps
    let chunker_map: HashMap<String, String> = detector_hashmaps.0;
    // Should ideally just be HashMap<String, ChunkerConfig> after processing
    let chunker_config_map: HashMap<String, Result<ChunkerConfig, ErrorResponse>> = detector_hashmaps.1;

    // Check for input detection
    let input_detectors: Option<HashMap<String, HashMap<String, String>>> = payload.clone().guardrail_config.unwrap().input.unwrap().models;
    let do_input_detection: bool = input_detectors.is_some();
    if do_input_detection {
        // Input detection tasks - all unary - can abstract this later
        // TODO: Confirm if tokenize should be happening on original user input
        // or spliced user input (for masks) - latter today
        // This separate call would not be necessary if generation is called, since it
        // provides input_token_count
        // Add tokenization task to count input tokens - grpc [unary] call
        let input_token_count = tokenize_unary_call(model_id.clone(), user_input.clone()).await;
        
        let input_detector_models: HashMap<String, HashMap<String, String>> = input_detectors.unwrap();
        // Add detection task for each detector - rest [unary] call
        // For each detector, add chunker task as precursor - grpc [unary] call
        let input_response = input_detection(input_detector_models, chunker_map, chunker_config_map, user_input.clone()).await;
    }

    // Response aggregation task
    // "break" if input detection - but this fn not responsible for short-circuit


    // Check for output detection
    let output_detectors: Option<HashMap<String, HashMap<String, String>>> = payload.clone().guardrail_config.unwrap().output.unwrap().models;
    let do_output_detection: bool = output_detectors.is_some();

    // ============= Unary endpoint =============
    if !streaming {
        // Add TGIS generation - grpc [unary] call
        let tgis_response = tgis_unary_call(model_id.clone(), user_input.clone(), payload.text_gen_parameters).await;
        if do_output_detection {
            let output_detector_models: HashMap<String, HashMap<String, String>> = output_detectors.unwrap();
            // Add detection task for each detector - rest [unary] call
            // For each detector, add chunker task as precursor - grpc [unary] call
        }
        // Response aggregation task

    } else { // ============= Streaming endpoint =============
        // Add TGIS generation task - grpc [server streaming] call
        let on_message_callback = |stream_token: GenerationResponse| {
            let event = Event::default();
            event.json_data(stream_token).unwrap()
        };

        // Fix payload here
        let tgis_response_stream =
            tgis_stream_call(Json(payload.clone()), payload.text_gen_parameters, on_message_callback, ).await;

        if do_output_detection {
            let output_detector_models: HashMap<String, HashMap<String, String>> = output_detectors.unwrap();
            // Add detection task for each detector - rest [unary] call
            // For each detector, add chunker task as precursor - grpc [bidi stream] call
        }
        // Response aggregation task
    }
}