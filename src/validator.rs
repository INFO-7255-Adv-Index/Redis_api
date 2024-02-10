use serde_json::Value;
use std::fs::File;
use std::io::Read;
use std::error::Error;

pub struct Validator {
    schema: Value,
}

impl Validator {
    pub fn from_file(path: &str) -> Result<Self, Box<dyn Error>> {
        let mut file = File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let schema = serde_json::from_str(&contents)?;
        Ok(Self { schema })
    }

  pub fn validate(&self, data: &Value) -> Result<(), Box<dyn Error>> {
        if let Value::Object(schema_map) = &self.schema {
            if let Some(Value::Object(properties)) = schema_map.get("properties") {
                if let Value::Object(data_map) = data {
                    for (key, schema_value) in properties {
                        if let Some(data_value) = data_map.get(key) {
                            match schema_value.get("type").and_then(Value::as_str) {
                                Some("integer") => {
                                    if data_value.as_i64().is_none() {
                                        return Err(format!("Key {} is not an integer", key).into());
                                    }
                                }
                                Some("string") => {
                                    if data_value.as_str().is_none() {
                                        return Err(format!("Key {} is not a string", key).into());
                                    }
                                }
                                Some("$ref") => {
                                  
                                    if let Some(ref_path) = schema_value.as_str() {
                                        if let Some(ref_schema) = self.get_ref_schema(ref_path) {
                                            let ref_validator: Validator = Validator { schema: ref_schema };
                                            ref_validator.validate(data_value)?;
                                        } else {
                                            return Err(format!("Invalid $ref path {}", ref_path).into());
                                        }
                                    } else {
                                        return Err("Invalid $ref value".into());
                                    }
                                }
                                _ => return Err(format!("Unsupported type for key {}", key).into()),
                            }
                        } else if schema_map.get("required").and_then(Value::as_array).map_or(false, |arr| arr.contains(&Value::String(key.clone()))) {
                            return Err(format!("Key {} is required but not found in data", key).into());
                        }
                    }
                    return Ok(());
                } else {
                    return Err("Data is not an object".into());
                }
            } else {
                return Err("Schema does not contain properties".into());
            }
        } else {
            return Err("Schema is not an object".into());
        }
    }
    

    fn get_ref_schema(&self, ref_path: &str) -> Option<Value> {
        let parts: Vec<&str> = ref_path.split('/').collect();
        if parts.len() < 2 || parts[0] != "#" {
            return None;
        }
        let mut current = &self.schema;
        for part in parts.iter().skip(1) {
            if let Some(next) = current.get(part) {
                current = next;
            } else if let Some(definitions) = current.get("definitions") {
                if let Some(next) = definitions.get(part) {
                    current = next;
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        Some(current.clone())
    }
}