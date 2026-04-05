use ailoy::Value;

pub fn extract_required_str(args: &Value, key: &str) -> anyhow::Result<String> {
    args.pointer(&format!("/{}", key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("missing required parameter: {}", key))
}

pub fn extract_optional_i64(args: &Value, key: &str) -> Option<i64> {
    args.pointer(&format!("/{}", key))
        .and_then(|v| v.as_integer())
}

pub fn result_to_value<T: serde::Serialize>(result: &T) -> Value {
    let json = serde_json::to_value(result).unwrap_or(serde_json::Value::Null);
    serde_json::from_value::<Value>(json).unwrap_or(Value::Null)
}
