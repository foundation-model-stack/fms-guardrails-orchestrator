use std::{collections::HashMap, usize};

use crate::{clients::nlp::NlpServicer, config::{ChunkerConfig, DetectorConfig, DetectorMap}, models::{ClassifiedGeneratedTextResult, FinishReason, GeneratedTextResult, GeneratedTextStreamResult, GeneratedToken, GuardrailsHttpRequest, GuardrailsTextGenerationParameters, InputWarning, InputWarningReason, TextGenTokenClassificationResults, TokenClassificationResult, TokenStreamDetails}, pb::fmaas::{BatchedTokenizeRequest, TokenizeRequest}, utils::{call_chunker, call_text_gen_tokenization}, ErrorResponse
};
use axum::Json;
use axum::response::sse::Event;
use futures::stream::Stream;
use serde::Serialize;
use std::convert::Infallible;

use crate::pb::{
    caikit_data_model::nlp::{
        Token, TokenizationStreamResult,
}};


// ========================================== Constants and Dummy Variables ==========================================
const UNSUITABLE_INPUT_MESSAGE: &'static str = "Unsuitable input detected. Please check the detected entities on your input and try again with the unsuitable input removed.";

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

// API calls - will not live here

// Unary TGIS call - first pass will be through caikit-nlp
async fn tgis_unary_call(model_id: String, text: String, text_gen_params: Option<GuardrailsTextGenerationParameters>) -> GeneratedTextResult {
    // Expect only one text here
    let token_info: GeneratedToken = GeneratedToken {
        text: "hi".to_string(),
        logprob: Some(0.53),
        rank: Some(1),
    };
    GeneratedTextResult {
        input_token_count: 1,
        generated_tokens: Some(1),
        generated_text: "hi".to_string(),
        finish_reason: Some(FinishReason::MaxTokens),
        seed: Some(42),
        tokens: Some(vec![token_info.clone()]),
        input_tokens: Some(vec![token_info.clone()]),
    }
}

// Server streaming TGIS call - first pass will be through caikit-nlp
async fn tgis_stream_call(
    Json(text): Json<String>,
    text_gen_params: Option<GuardrailsTextGenerationParameters>,
    on_message_callback: impl Fn(GeneratedTextStreamResult) -> Event,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let mut dummy_response_iterator = DUMMY_RESPONSE.iter();

    let mut input_token_count: i32 = 0;
    let token_info: GeneratedToken = GeneratedToken {
        text: "hi".to_string(),
        logprob: Some(0.53),
        rank: Some(1),
    };

    let stream = async_stream::stream! {
        // Server sending event stream
        while let Some(&token) = dummy_response_iterator.next() {
            let details: TokenStreamDetails = TokenStreamDetails {
                finish_reason:FinishReason::MaxTokens.into(),
                seed: Some(42),
                input_token_count,
                generated_tokens: Some(20),
            };
            let stream_token = GeneratedTextStreamResult {
                generated_text: token.to_string(),
                details: Some(details),
                input_tokens: Some(vec![token_info.clone()]),
                tokens: Some(vec![token_info.clone()]),
            };
            input_token_count += 1;
            let event = on_message_callback(stream_token);
            yield Ok(event);
        }
    };
    stream
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
    let token_vec = vec![token_0, token_1];
    let mut dummy_response_iterator = DUMMY_RESPONSE.iter();

    let stream = async_stream::stream! {
        // Server sending event stream
        while let Some(&token) = dummy_response_iterator.next() {
            let stream_token = TokenizationStreamResult {
                results: token_vec.clone(),
                processed_index: 1, 
                start_index: 0,
                token_count: 2,
            };
            let event = on_message_callback(stream_token);
            yield Ok(event);
        }
    };
    stream
}

// Unary detector call
// Token classification result vector used for now but expecting more generic DetectorResponse in the future
// Assume no batched calls currently
async fn detector_call(detector_id: String, input: String, offset: usize) -> Vec<TokenClassificationResult> {
    // Might need some routing/extra endpoint info to begin with
    let result: TokenClassificationResult = TokenClassificationResult {
        start: 0 + offset as i32,
        end: 3 + offset as i32,
        word: "moo".to_owned(),
        entity: "cow".to_owned(),
        entity_group: "cow".to_owned(),
        token_count: Some(1),
        score: 0.5,
    };
    vec![result]
}

// ================================ Orchestrator internal logic ==========================================

fn slice_input(mut user_input: Vec<(String, usize)>, payload: GuardrailsHttpRequest) -> Vec<(String, usize)>{
    // Slice user input based on masks if they are provided, tracking the mask start for offset
    if let Some(input_masks) = payload.guardrail_config.unwrap().input.unwrap().masks {
        if let Some((original_user_input, _offset)) = user_input.first() {
            let user_input_vec = original_user_input.chars().collect::<Vec<_>>();
            // Extra work for codepoint slicing in Rust
            user_input = vec![];
            for (start, end) in input_masks {
                let mask_string: String = user_input_vec[start..end].iter().cloned().collect::<String>();
                user_input.push((mask_string, start));
            }
        }
    }
    user_input
}


async fn unary_chunk_and_detection(
    detectors_models: HashMap<String, HashMap<String, String>>,
    chunker_map: &HashMap<String, String>,
    chunker_config_map: &HashMap<String, Result<ChunkerConfig, ErrorResponse>>,
    texts_with_offsets: Vec<(String, usize)>,
    nlp_servicer: NlpServicer,
) -> Vec<TokenClassificationResult> {
    // detectors_models: model_name: {param: value}
    // Future - parallelize calls
    let mut detector_responses = Vec::new();
    for detector_id in detectors_models.keys() {
        if let Some(chunker_id) = chunker_map.get(detector_id){
            for (text, offset) in texts_with_offsets.iter() {
                // TODO: Get config/type for chunker call
                if let Ok(tokenization_results) = call_chunker(text.to_string(), chunker_id.to_string(), nlp_servicer.clone()).await {
                    // Optimize later - chunkers would always be called even if multiple detectors had same chunker
                    let mut texts: Vec<String> = Vec::new();
                    for token in tokenization_results.results.iter() {
                        texts.push(token.text.to_string())
                    }
                    // TODO: Pass on detector params
                    for text in texts.iter() {
                        let detector_response = detector_call(detector_id.to_string(), text.to_string(), *offset).await;
                        detector_responses.extend(detector_response)
                    }
                } else {
                    // TODO: handle error - this would be an internal error if a chunker call did
                    // not succeed because end users will know nothing about chunkers
                }
            }
        } else {
            continue; // Configuration errors for missing detectors and/or associated chunkers should have
            // already been handled and surfaced at config parsing time
        }
    }
    detector_responses
}

fn aggregate_response_for_input(input_detection_response: Vec<TokenClassificationResult>, input_token_count: i32) -> ClassifiedGeneratedTextResult {
    let token_classification_results = TextGenTokenClassificationResults {
        input: Some(input_detection_response.clone()),
        output: None,
    };
    let mut warnings = None;
    // Parse input detection results if they do exist
    if !input_detection_response.clone().is_empty() {
        warnings = Some(vec![InputWarning {
            id: Some(InputWarningReason::UnsuitableInput),
            message: Some(UNSUITABLE_INPUT_MESSAGE.to_string()),
        }]);
    };
    let mut classified_result: ClassifiedGeneratedTextResult = ClassifiedGeneratedTextResult::new(token_classification_results, input_token_count);
    classified_result.warnings = warnings;
    classified_result
}

fn aggregate_response_for_output_unary(
    output_detection_response: Vec<TokenClassificationResult>,
    tgis_response: GeneratedTextResult) -> ClassifiedGeneratedTextResult{
    let output_token_classification_results = TextGenTokenClassificationResults {
        input: None,
        output: Some(output_detection_response),
    };
    ClassifiedGeneratedTextResult {
        generated_text: Some(tgis_response.generated_text),
        token_classification_results: output_token_classification_results,
        finish_reason: tgis_response.finish_reason,
        generated_token_count: tgis_response.generated_tokens,
        seed: tgis_response.seed,
        input_token_count: tgis_response.input_token_count,
        warnings: None,
        tokens: tgis_response.tokens,
        input_tokens: tgis_response.input_tokens,
    }
}

// ========================================== Main ==========================================

// In the future probably create DAG/list of tasks to be invoked
pub async fn do_tasks(payload: GuardrailsHttpRequest, 
    detector_hashmaps: (HashMap<String, String>, HashMap<String, Result<ChunkerConfig, ErrorResponse>>),
    nlp_servicer: NlpServicer,
    streaming: bool) {
    // TODO: is clone() needed for every payload use? Otherwise move errors since payload has String

    // LLM / text generation model
    let model_id: String = payload.clone().model_id;

    // No guardrail_config specified
    if payload.guardrail_config.is_none() {
        // TODO: Just do text gen? Error?
        // This falls through to text gen today but validation is not done
    }

    // Process detector hashmaps
    let chunker_map: HashMap<String, String> = detector_hashmaps.0;
    // Should ideally just be HashMap<String, ChunkerConfig> after processing
    let chunker_config_map: HashMap<String, Result<ChunkerConfig, ErrorResponse>> = detector_hashmaps.1;

    // Check for input detection
    let input_detectors: Option<HashMap<String, HashMap<String, String>>> = payload.clone().guardrail_config.unwrap().input.unwrap().models;
    let do_input_detection: bool = input_detectors.is_some();
    let mut input_detection_response: Vec<TokenClassificationResult> = Vec::new();
    let mut input_token_count = 0;
    if do_input_detection {
        // Input detection tasks - all unary - can abstract this later
        // Add tokenization task to count input tokens - grpc [unary] call
        // This separate call would not be necessary if generation is called, since it
        // provides input_token_count, whole input is used for token counts
        if let Ok(tokenization_results) = call_text_gen_tokenization(payload.clone().inputs, model_id.to_string(), nlp_servicer.clone()).await {
            input_token_count = tokenization_results.token_count;
        }

        // Initialize as vector for type consistency if masks are supplied
        let mut user_input_with_offsets: Vec<(String, usize)> = vec![(payload.clone().inputs, 0)];

        // Slice up if masks are supplied
        // Whole payload is just passed here to abstract away impl, could be separate task
        // tracked as part of DAG/list instead of function in the future
        user_input_with_offsets = slice_input(user_input_with_offsets, payload.clone());
        
        let input_detector_models: HashMap<String, HashMap<String, String>> = input_detectors.unwrap();
        // Add detection task for each detector - rest [unary] call
        // For each detector, add chunker task as precursor - grpc [unary] call
        input_detection_response = unary_chunk_and_detection(input_detector_models, &chunker_map, &chunker_config_map, user_input_with_offsets.clone(), nlp_servicer.clone()).await;
    }

    let classified_result: ClassifiedGeneratedTextResult = aggregate_response_for_input(input_detection_response, input_token_count as i32);
    // TODO: "break" if input detection - but this fn not responsible for short-circuit

    // Check for output detection
    let output_detectors: Option<HashMap<String, HashMap<String, String>>> = payload.clone().guardrail_config.unwrap().output.unwrap().models;
    let do_output_detection: bool = output_detectors.is_some();

    // ============= Unary endpoint =============
    if !streaming {
        // Add TGIS generation - grpc [unary] call
        let tgis_response = tgis_unary_call(model_id.clone(), payload.inputs, payload.text_gen_parameters).await;
        let mut output_detection_response: Vec<TokenClassificationResult> = Vec::new();
        if do_output_detection {
            let output_detector_models: HashMap<String, HashMap<String, String>> = output_detectors.unwrap();
            // Add detection task for each detector - rest [unary] call
            // For each detector, add chunker task as precursor - grpc [unary] call
            output_detection_response = unary_chunk_and_detection(output_detector_models, &chunker_map, &chunker_config_map, vec![(tgis_response.clone().generated_text, 0)], nlp_servicer.clone()).await;
        }
        // Response aggregation
        let classified_result = aggregate_response_for_output_unary(output_detection_response, tgis_response);
    } else { // ============= Streaming endpoint =============
        // Add TGIS generation task - grpc [server streaming] call
        let on_message_callback = |stream_token: GeneratedTextStreamResult| {
            let event = Event::default();
            event.json_data(stream_token).unwrap()
        };

        let tgis_response_stream =
            tgis_stream_call(Json(payload.inputs), payload.text_gen_parameters, on_message_callback, ).await;

        if do_output_detection {
            let output_detector_models: HashMap<String, HashMap<String, String>> = output_detectors.unwrap();
            // Add detection task for each detector - rest [unary] call
            // For each detector, add chunker task as precursor - grpc [bidi stream] call
        }
    // Response aggregation task
    }
}
