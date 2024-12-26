use serde_json::json;
use tokio_stream::StreamExt;

const URI: &str = "http://localhost:8033/api/v2/text/detection/stream-content";

#[tokio::main]
async fn main() {
    println!("Hi there!");

    let request_chunks = vec![
        json!({
            "detectors": {
                "hap": {}
            },
            "content": "Message 1"
        }),
        json!(
        {
            "content": "Message 2"
        }),
    ];

    let client = reqwest::Client::new();

    for chunk in request_chunks {
        let mut response = client
            .post(URI)
            .json(&chunk)
            .send()
            .await
            .unwrap()
            .bytes_stream();

        while let Some(frame) = response.next().await {
            dbg!(frame.unwrap());
        }
    }
}
