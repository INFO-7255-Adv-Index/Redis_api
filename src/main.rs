use axum::{
    body::Body, extract::{Extension, Json}, http::{HeaderMap, HeaderValue, StatusCode}, response::Response, routing::{get, post,delete}, Router
};
use redis::AsyncCommands;
use serde_json::Value;
use std::{collections::HashMap, fs, net::SocketAddr};
use jsonschema::JSONSchema;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use http::header::ETAG;
// Define a type alias for the Redis connection.
type RedisPool = redis::Client;

#[tokio::main]
async fn main() {
    // Initialize the Redis client.
    let redis_client = redis::Client::open("redis://127.0.0.1/").unwrap();

    // Build our application with the route to store JSON data.
    let app = Router::new()
        .route("/v1/plan", post(store_json))
        .route("/v1/plan/getall", get(getall))
        .route("/v1/plan/getbyID/:object_id", get(getby_id))
        .route("/v1/plan/delete/:object_id", delete(delete_by_id))
        .route("/v1/plan/getall_kv", get(getall_kv))
        .layer(Extension(redis_client));

    // Run our application.
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
  
}

async fn store_json(
    Extension(redis_client): Extension<RedisPool>,
    Json(payload): Json<Value>,
) -> Result<Response, (StatusCode, String)>{
    // Load the schema and validate the JSON payload.
    let schema_json: Value = serde_json::from_str(&fs::read_to_string("schema.json").unwrap()).unwrap();
    let compiled_schema = JSONSchema::compile(&schema_json).expect("A valid schema");
    if !compiled_schema.is_valid(&payload) {
        return Err((StatusCode::BAD_REQUEST, "Invalid JSON body".to_string()));
    }

    // Serialize the JSON payload to a string.
    let json_string = match serde_json::to_string(&payload) {
        Ok(json) => json,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    // Generate an ETag based on the JSON content.
    let mut hasher = DefaultHasher::new();
    json_string.hash(&mut hasher);
    let etag_value = hasher.finish().to_string();
    // Extract the object ID from the JSON payload.
    let object_id = match payload.get("objectType") {
        Some(object_type) => {
            if object_type == "plan" {
                match payload.get("objectId") {
                    Some(id) => id.as_str().unwrap(),
                    None => return Err((StatusCode::BAD_REQUEST, "Missing objectId in plan JSON body".to_string())),
                }
            } else {
                return Err((StatusCode::BAD_REQUEST, "The provided JSON is not a plan object".to_string()));
            }
        },
        None => return Err((StatusCode::BAD_REQUEST, "Missing objectType in JSON body".to_string())),
    };
    
    // Get an asynchronous connection to Redis.
    let mut con = match redis_client.get_async_connection().await {
        Ok(con) => con,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    let exists: i32 = con.exists(&object_id).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to check key: {}", e))
    })?;
    
    if exists > 0 {
        return Err((StatusCode::CONFLICT, "Key already exists in the database".to_string()));
    }
    // Store the JSON string in Redis using the ETag as the key.
    let _: () = con.set(&object_id, &json_string).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to set key: {}", e))
    })?;

    // Prepare the response headers to include the ETag.
    let mut headers = HeaderMap::new();
    headers.insert(ETAG, HeaderValue::from_str(&etag_value).unwrap());
    let response = Response::builder()
    .status(StatusCode::CREATED)
    .header(ETAG, etag_value)
    .body(json_string.into())
    .unwrap();
    // Return the ETag as the response with the ETag header.
    Ok(response)
}
// 
async fn getall_kv(
    Extension(redis_client): Extension<RedisPool>,
) -> Result<Response, (StatusCode, String)> {
    // Get an asynchronous connection to Redis.
    let mut con = match redis_client.get_async_connection().await {
        Ok(con) => con,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    // Retrieve all keys and their corresponding values from the Redis store.
    let keys: Vec<String> = con.keys("*").await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to retrieve keys: {}", e))
    })?;

    let mut data = HashMap::new();
    for key in keys {
        let value: String = con.get(&key).await.map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to retrieve value for key {}: {}", key, e))
        })?;
        data.insert(key, value);
    }

    // Serialize the HashMap into a JSON string.
    let json_body = match serde_json::to_string(&data) {
        Ok(json) => json,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    // Prepare the JSON response.
    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(json_body.into())
        .unwrap();

    Ok(response)
}
async fn getby_id(
    Extension(redis_client): Extension<RedisPool>,
    axum::extract::Path(object_id): axum::extract::Path<String>,
    headers: HeaderMap, // Add headers parameter to access request headers
) -> Result<Response, (StatusCode, String)> {
    // Get an asynchronous connection to Redis.
    let mut con = match redis_client.get_async_connection().await {
        Ok(con) => con,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    // Retrieve the JSON string from Redis using the objectId as the key.
    let result: Option<String> = con.get(&object_id).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get value for key {}: {}", object_id, e))
    })?;

    match result {
        Some(json_string) => {
            // Calculate ETag for the retrieved JSON string
            let mut hasher = DefaultHasher::new();
            json_string.hash(&mut hasher);
            let etag_value = hasher.finish().to_string();

            // Check If-None-Match header and compare with the calculated ETag
            if let Some(if_none_match) = headers.get("If-None-Match") {
                if if_none_match == etag_value.as_str() {
                    
                    let response = Response::builder()// If ETag matches, return 304 Not Modified
                        .status(StatusCode::NOT_MODIFIED)
                        .body(Body::empty())
                        .unwrap();
                    return Ok(response);
                }
            }

            // Prepare the response with the retrieved JSON string and ETag
            let response = Response::builder()
                .status(StatusCode::OK)
                .header("ETag", etag_value)
                .body(json_string.into())
                .unwrap();
            Ok(response)
        },
        None => Err((StatusCode::NOT_FOUND, format!("No value found for key {}", object_id))),
    }
}

async fn delete_by_id(
    Extension(redis_client): Extension<RedisPool>,
    axum::extract::Path(object_id): axum::extract::Path<String>,
) -> Result<Response, (StatusCode, String)> {
    // Get an asynchronous connection to Redis.
    let mut con = match redis_client.get_async_connection().await {
        Ok(con) => con,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    // Attempt to delete the entry with the specified objectId.
    let deleted: u64 = con.del(&object_id).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete key {}: {}", object_id, e))
    })?;

    if deleted == 0 {
        // If no key was deleted, it means the key was not found.
        return Err((StatusCode::NOT_FOUND, format!("No value found for key {}", object_id)));
    }

    // If the key was successfully deleted, return a success response.
    let response = Response::builder()
        .status(StatusCode::OK)
        .body("Deleted successfully".into())
        .unwrap();

    Ok(response)
}
async fn getall(
    Extension(redis_client): Extension<RedisPool>,
) -> Result<Response, (StatusCode, String)> {
    // Get an asynchronous connection to Redis.
    let mut con = match redis_client.get_async_connection().await {
        Ok(con) => con,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    // Retrieve all keys and their corresponding values from the Redis store.
    let mut response_body = String::new();
    let keys: Vec<String> = con.keys("*").await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to retrieve keys: {}", e))
    })?;
    for key in keys {
        let value: String = con.get(&key).await.map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to retrieve value for key {}: {}", key, e))
        })?;
        response_body.push_str(&format!("Key: {}, Value: {}\n", key, value));
    }

    // Prepare the response with the retrieved keys and values.
    let response = Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(response_body))
        .unwrap();

    Ok(response)
}
