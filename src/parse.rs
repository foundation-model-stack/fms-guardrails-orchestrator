// Parse chunker and detector mapping
use serde_yml::Value;

// TODO: We want this to become actual chunker and detector structs
// Debug use  let chunkers = parse::read_chunker_detector_map();
pub fn read_chunker_detector_map() -> Result<Value, config::ConfigError> {
    println!("Read getting called??");

    let base_path = std::env::current_dir().expect("Failed to determine the current directory");
    let configuration_directory = base_path.join("config");

    let configmap = config::Config::builder()
    .add_source(config::File::from(
        configuration_directory.join("configmap.yaml"),
    ))
    .build()?;

    configmap.try_deserialize::<Value>()
}
