use std::collections::HashMap;
use std::fs;

// Config loading uses expect() because configuration is a critical startup dependency
// - If config files can't be read, the application cannot function
// - This is not a recoverable runtime error but a setup/environment issue
pub fn load_config(path: &str) -> HashMap<String, String> {
    let content = fs::read_to_string(path).expect("Failed to read config file");
    let mut map = HashMap::new();
    for line in content.lines() {
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..].trim().to_string();
            map.insert(key, value);
        }
    }
    map
}