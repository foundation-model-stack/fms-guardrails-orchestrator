use std::{collections::{hash_map::Entry, HashMap}, usize};

use crate::{config::{ChunkerConfig, ChunkerType, DetectorConfig, DetectorMap}, models::{GuardrailsConfig, GuardrailsHttpRequest}, pb::fmaas::{generation_service_server::GenerationService, BatchedTokenizeRequest, TokenizeRequest}, ErrorResponse};
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


// pub fn parse_detector_map(detector_map: DetectorMap) -> (HashMap<std::string::String, std::string::String>, HashMap<std::string::String, Vec<std::string::String>>) {
//     let chunkers: HashMap<String, ChunkerConfig> = detector_map.chunkers;
//     let detectors:HashMap<String, DetectorConfig> = detector_map.detectors;

//     let mut detectors_to_chunkers = HashMap::with_capacity(detectors.len());
//     // This could be more intelligently replaced with a DAG but non-optimized for now
//     // Map of each chunker to list of detectors they support to optimize
//     let mut chunkers_to_detectors: HashMap<String, Vec<String>> = HashMap::with_capacity(chunkers.len());
//     let mut detector_info: HashMap<String, DetectorConfig> = HashMap::with_capacity(detectors.len());

//     for (detector_name, detector_config) in detectors.into_iter() {

//         // Track detectors for each chunker
//         let chunker_id: String = detector_config.chunker;
//         match chunkers_to_detectors.entry(chunker_id) {
//             Entry::Vacant(e) => { e.insert(vec![detector_name]); },
//             Entry::Occupied(mut e) => { e.get_mut().push(detector_name); }
//         }
//         detectors_to_chunkers.insert(detector_name, chunker_id);
//         detector_info.insert(detector_name, detector_config);
//     }
//     // At the end of this we should know which chunkers actually could
//     // be invoked based on the chunkers_to_detectors map. Extra chunkers
//     // don't need tracking because should not be invoked independently
//     // of a detector.

//     // Based on the user request, the detector request list will be formed,
//     // with chunker requests as prerequisites.

//     // for (key, value) in chunkers_to_detectors.into_iter() {
//     //     println!("{} / {:?}", key, value);
//     // }
//     (detectors_to_chunkers, chunkers_to_detectors)

// }

// fn get_chunker_for_detector(detector_to_find: String, detector_map: DetectorMap) -> Result<ChunkerConfig, ErrorResponse> {
//     let chunkers: HashMap<String, ChunkerConfig> = detector_map.chunkers;
//     let detectors:HashMap<String, DetectorConfig> = detector_map.detectors;
//     for (detector_name, detector_config) in detectors.into_iter() {
//         if detector_to_find == detector_name {
//             let chunker_name: String = detector_config.chunker;
//             return Ok(Some(&chunker_name)) // FIX
//         }
//     }
//     ErrorResponse("Detector not configured correctly");
// }

// ========================================== Dummy Tasks ==========================================

// API calls

// Server streaming TGIS call
async fn tgis_call(
    Json(tgis_payload): Json<GuardrailsHttpRequest>,
    on_message_callback: impl Fn(GenerationResponse) -> Event,
) -> impl Stream<Item = Result<Event, Infallible>> {

    const DUMMY_RESPONSE: [&'static str; 9] = ["This", "is", "very", "good", "news,", "streaming", "is", "working", "!"];

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
async fn tokenize_call(model_id: String, texts: Vec<String>) -> TokenizeResponse {
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

// Unary detector call
// Assume processing on batch (multiple strings) can at least happen
async fn detector_call(model_id: String, inputs: Vec<String>) -> DetectorResponse {
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

async fn input_detection(input_detectors_models: HashMap<String, HashMap<String, String>>) {
    // TODO
}

// ========================================== Main ==========================================

pub async fn create_tasks(payload: GuardrailsHttpRequest) {
    
    // LLM / text generation model
    let model_id: String = payload.model_id;

    // Original user input text, initialized as vector for type
    // consistency if masks are supplied
    let mut user_input: Vec<String> = vec![payload.inputs];

    // No guardrail_config specified
    if payload.guardrail_config.is_none() {
        // TODO: Just do text gen? Error?
        // This falls through to text gen today but validation is not done
    }

    // Slice up if masks are supplied
    // Whole payload is just passed here to abstract away impl, could be separate task
    // tracked as part of DAG/list instead of function in the future
    user_input = slice_input(user_input, payload);

    // Check for input detection
    let input_detectors: Option<HashMap<String, HashMap<String, String>>> = payload.guardrail_config.unwrap().input.unwrap().models;
    let do_input_detection: bool = input_detectors.is_some();
    if do_input_detection {
        // Input detection tasks - all unary - can abstract this later
        // TODO: Confirm if tokenize should be happening on original user input
        // or spliced user input (for masks) - latter today
        // This separate call would not be necessary if generation is called, since it
        // provides input_token_count
        let input_token_count = tokenize_call(model_id, user_input);
        
        let input_detector_models: HashMap<String, HashMap<String, String>> = input_detectors.unwrap();
        //let input_response = input_detection(input_detectors_models);
    }
    // Add tokenization task to count input tokens - grpc [unary] call
    // Get any detectors from payload.guardrail_config.input.models
    // Add detection task for each detector - rest [unary] call
    // For each detector, add chunker task as precursor - grpc [unary] call

    // Response aggregation task
    // "break" if input detection - but this fn not responsible for short-circuit

    // payload.text_gen_parameters - extra TGIS generation parameters


    // Add TGIS generation task - grpc [server streaming] call
    let on_message_callback = |stream_token: GenerationResponse| {
        let event = Event::default();
        event.json_data(stream_token).unwrap()
    };

    let tgis_response_stream =
        tgis_call(Json(payload), on_message_callback).await;


    // If output detection
    // Get any detectors from payload.guardrail_config.output.models
    // Add detection task for each detector - rest [unary] call
    // For each detector, add chunker task as precursor - grpc [bidi stream] call
    // Response aggregation task

    // Eventually make this into a DAG/list of tasks that will be invoked for
    // abstraction purposes instead of calling functions directly here
}