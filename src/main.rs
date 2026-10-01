mod minimap;
mod scenario;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use scenario::{ScenarioError, ScenarioInfo};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

type ApiError = (StatusCode, Json<ErrorResponse>);
const PARSER_QUEUE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Serialize)]
struct UploadScenarioResponse {
    id: uuid::Uuid,
}

struct ScenarioUpload {
    file_name: String,
    bytes: axum::body::Bytes,
}

#[derive(Serialize)]
struct StoredScenario<'a> {
    id: uuid::Uuid,
    original_filename: &'a str,
    uploaded_at: u64,
    file_size: usize,
    parser_version: &'a str,
    scenario: &'a ScenarioInfo,
}

#[derive(Deserialize)]
struct StoredScenarioMetadata {
    original_filename: String,
}

#[derive(Serialize)]
struct ScenarioListItem {
    id: uuid::Uuid,
    original_filename: String,
    uploaded_at: u64,
    file_size: usize,
}

#[derive(Clone)]
struct AppState {
    parser_slots: Arc<Semaphore>,
    scenario_data_dir: Arc<std::path::PathBuf>,
    database: PgPool,
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}

async fn scenario_page() -> Html<&'static str> {
    Html(include_str!("../web/scenario.html"))
}

async fn healthz() -> &'static str {
    "ok"
}

fn error_response(status: StatusCode, message: &str) -> ApiError {
    (
        status,
        Json(ErrorResponse {
            error: message.to_string(),
        }),
    )
}

fn scenario_error_response(error: ScenarioError) -> ApiError {
    let (status, message) = match &error {
        ScenarioError::ParseFailed(_) => {
            tracing::warn!(
                error = %error,
                "Invalid scenario uploaded"
            );

            (
                StatusCode::BAD_REQUEST,
                "Invalid or unsupported scenario file",
            )
        }

        ScenarioError::FileNotFound
        | ScenarioError::ProcessFailed(_)
        | ScenarioError::InvalidOutput(_)
        | ScenarioError::ProcessTerminated
        | ScenarioError::ParserTimedOut => {
            tracing::error!(
                error = %error,
                "Scenario parser failed"
            );

            (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    };

    error_response(status, message)
}

fn validate_scenario_filename(file_name: Option<String>) -> Result<String, ApiError> {
    let file_name = match file_name {
        Some(file_name) => file_name,
        None => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                "Uploaded field has no filename",
            ));
        }
    };

    if std::path::Path::new(&file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("aoe2scenario")
    {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "File must be an .aoe2scenario file",
        ));
    }

    Ok(file_name)
}

fn store_scenario_tempfile(bytes: &[u8]) -> Result<tempfile::NamedTempFile, ApiError> {
    let mut temp_file = match tempfile::NamedTempFile::new() {
        Ok(file) => file,
        Err(error) => {
            tracing::error!(
                error = %error,
                "tempfile::NamedTempFile::new failed"
            );

            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to store the scenario",
            ));
        }
    };

    match temp_file.write_all(bytes) {
        Ok(()) => {}
        Err(error) => {
            tracing::error!(
                error = %error,
                "tempfile::NamedTempFile::write_all failed"
            );

            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to store the scenario",
            ));
        }
    }
    Ok(temp_file)
}

async fn parse_scenario_with_slot(
    state: &AppState,
    path: &std::path::Path,
) -> Result<ScenarioInfo, ApiError> {
    let result = {
        let _permit =
            match tokio::time::timeout(PARSER_QUEUE_TIMEOUT, state.parser_slots.acquire()).await {
                Ok(Ok(permit)) => permit,

                Ok(Err(error)) => {
                    tracing::error!(
                        error = %error,
                        "Unable to acquire parser slot"
                    );

                    return Err(error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error",
                    ));
                }

                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "Timed out waiting for parser slot"
                    );

                    return Err(error_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "Server is busy processing another scenario",
                    ));
                }
            };

        scenario::parse_scenario(path).await
    };

    match result {
        Ok(scenario) => Ok(scenario),
        Err(error) => Err(scenario_error_response(error)),
    }
}

async fn read_scenario_upload(multipart: &mut Multipart) -> Result<ScenarioUpload, ApiError> {
    match multipart.next_field().await {
        Ok(Some(field)) => {
            let field_name = field.name().map(str::to_string);
            if field_name.as_deref() != Some("scenario") {
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    "Expected multipart field named 'scenario'",
                ));
            }

            let file_name = field.file_name().map(str::to_string);
            let file_name = validate_scenario_filename(file_name)?;

            match field.bytes().await {
                Ok(bytes) => Ok(ScenarioUpload { file_name, bytes }),
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "Unable to read multipart field"
                    );

                    Err(error_response(
                        StatusCode::BAD_REQUEST,
                        "Unable to read uploaded file",
                    ))
                }
            }
        }
        Ok(None) => Err(error_response(StatusCode::BAD_REQUEST, "No field received")),
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Invalid multipart request"
            );
            Err(error_response(
                StatusCode::BAD_REQUEST,
                "Invalid upload request",
            ))
        }
    }
}

fn encode_png(image: image::DynamicImage) -> Result<Vec<u8>, ApiError> {
    let mut buffer = std::io::Cursor::new(Vec::new());

    match image.write_to(&mut buffer, image::ImageFormat::Png) {
        Ok(()) => {}
        Err(error) => {
            tracing::error!(
                error = %error,
                "image.write_to failed"
            );

            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to render the scenario",
            ));
        }
    }

    Ok(buffer.into_inner())
}

fn storage_error(error: std::io::Error, id: uuid::Uuid) -> ApiError {
    tracing::error!(
        error = %error,
        scenario_id = %id,
        "Unable to store scenario"
    );

    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unable to store the scenario",
    )
}

async fn upload_scenario(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<UploadScenarioResponse>, ApiError> {
    let upload = read_scenario_upload(&mut multipart).await?;

    let temp_file = store_scenario_tempfile(upload.bytes.as_ref())?;

    let scenario = parse_scenario_with_slot(&state, temp_file.path()).await?;

    let layers = minimap::render_isometric_minimap_layers(&scenario);
    let terrain_png = encode_png(image::DynamicImage::ImageRgb8(layers.terrain))?;

    let gaia_png = encode_png(image::DynamicImage::ImageRgba8(layers.gaia))?;

    let players_png = encode_png(image::DynamicImage::ImageRgba8(layers.players))?;

    let uploaded_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            tracing::error!(
                error = %error,
                "System clock is before Unix epoch"
            );

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to store the scenario",
            )
        })?
        .as_secs();

    let id = uuid::Uuid::new_v4();

    let scenario_dir = state.scenario_data_dir.join(id.to_string());

    let staging_dir = state.scenario_data_dir.join(format!(".tmp-{id}"));

    let staging_minimap_dir = staging_dir.join("minimap");

    let metadata = StoredScenario {
        id,
        original_filename: &upload.file_name,
        uploaded_at,
        file_size: upload.bytes.len(),
        parser_version: &scenario.parser_version,
        scenario: &scenario,
    };
    let metadata_bytes = serde_json::to_vec_pretty(&metadata).map_err(|error| {
        tracing::error!(
            error = %error,
            scenario_id = %id,
            "Unable to serialize scenario metadata"
        );

        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to store the scenario",
        )
    })?;

    let uploaded_at = i64::try_from(uploaded_at).map_err(|_| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to store the scenario",
        )
    })?;

    let file_size = i64::try_from(upload.bytes.len()).map_err(|_| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to store the scenario",
        )
    })?;

    let persist_result: Result<(), ApiError> = async {
        tokio::fs::create_dir_all(&staging_minimap_dir)
            .await
            .map_err(|error| storage_error(error, id))?;

        tokio::fs::write(staging_dir.join("original.aoe2scenario"), &upload.bytes)
            .await
            .map_err(|error| storage_error(error, id))?;

        tokio::fs::write(staging_dir.join("metadata.json"), metadata_bytes)
            .await
            .map_err(|error| storage_error(error, id))?;

        tokio::fs::write(staging_minimap_dir.join("terrain.png"), terrain_png)
            .await
            .map_err(|error| storage_error(error, id))?;

        tokio::fs::write(staging_minimap_dir.join("gaia.png"), gaia_png)
            .await
            .map_err(|error| storage_error(error, id))?;

        tokio::fs::write(staging_minimap_dir.join("players.png"), players_png)
            .await
            .map_err(|error| storage_error(error, id))?;

        tokio::fs::rename(&staging_dir, &scenario_dir)
            .await
            .map_err(|error| storage_error(error, id))?;

        Ok(())
    }
    .await;

    if let Err(error) = persist_result {
        match tokio::fs::remove_dir_all(&staging_dir).await {
            Ok(()) => {}

            Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => {}

            Err(cleanup_error) => {
                tracing::error!(
                    error = %cleanup_error,
                    scenario_id = %id,
                    "Unable to clean scenario staging directory"
                );
            }
        }

        return Err(error);
    }

    let database_result = sqlx::query(
        r#"
            INSERT INTO scenarios (
                id,
                original_filename,
                uploaded_at,
                file_size,
                parser_version,
                game_version,
                scenario_version
            )
            VALUES (
                $1,
                $2,
                TIMESTAMPTZ 'epoch' + ($3::bigint * INTERVAL '1 second'),
                $4,
                $5,
                $6,
                $7
            )
    "#,
    )
    .bind(id) // $1
    .bind(&upload.file_name) // $2
    .bind(uploaded_at) // $3
    .bind(file_size) // $4
    .bind(&scenario.parser_version) // $5
    .bind(&scenario.game_version) // $6
    .bind(&scenario.scenario_version) // $7
    .execute(&state.database)
    .await;

    if let Err(error) = database_result {
        tracing::error!(
            error = %error,
            scenario_id = %id,
            "Unable to insert scenario into database"
        );

        match tokio::fs::remove_dir_all(&scenario_dir).await {
            Ok(()) => {}

            Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => {}

            Err(cleanup_error) => {
                tracing::error!(
                    error = %cleanup_error,
                    scenario_id = %id,
                    "Unable to clean scenario directory after database failure"
                );
            }
        }

        return Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to store the scenario",
        ));
    }

    Ok(Json(UploadScenarioResponse { id }))
}

async fn delete_scenario(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, ApiError> {
    let scenario_dir = state.scenario_data_dir.join(id.to_string());

    let deleting_dir = state.scenario_data_dir.join(format!(".tmp-delete-{id}"));

    match tokio::fs::rename(&scenario_dir, &deleting_dir).await {
        Ok(()) => {}

        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(error_response(StatusCode::NOT_FOUND, "Scenario not found"));
        }

        Err(error) => {
            tracing::error!(
                error = %error,
                scenario_id = %id,
                "Unable to prepare scenario for deletion"
            );

            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to delete the scenario",
            ));
        }
    }

    let database_result = sqlx::query("DELETE FROM scenarios WHERE id = $1")
        .bind(id)
        .execute(&state.database)
        .await;

    let database_result = match database_result {
        Ok(result) => result,

        Err(error) => {
            tracing::error!(
                error = %error,
                scenario_id = %id,
                "Unable to delete scenario from database"
            );

            if let Err(rollback_error) = tokio::fs::rename(&deleting_dir, &scenario_dir).await {
                tracing::error!(
                    error = %rollback_error,
                    scenario_id = %id,
                    "Unable to restore scenario after database deletion failure"
                );
            }

            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to delete the scenario",
            ));
        }
    };

    if database_result.rows_affected() == 0 {
        if let Err(rollback_error) = tokio::fs::rename(&deleting_dir, &scenario_dir).await {
            tracing::error!(
                error = %rollback_error,
                scenario_id = %id,
                "Unable to restore scenario after missing database entry"
            );

            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to delete the scenario",
            ));
        }

        return Err(error_response(StatusCode::NOT_FOUND, "Scenario not found"));
    }

    match tokio::fs::remove_dir_all(&deleting_dir).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),

        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(StatusCode::NO_CONTENT),

        Err(error) => {
            tracing::error!(
                error = %error,
                scenario_id = %id,
                "Unable to remove deleted scenario files"
            );

            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to delete the scenario",
            ))
        }
    }
}

async fn download_scenario(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Response, ApiError> {
    let scenario_dir = state.scenario_data_dir.join(id.to_string());

    let metadata_bytes = tokio::fs::read(scenario_dir.join("metadata.json"))
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                error_response(StatusCode::NOT_FOUND, "Scenario not found")
            } else {
                storage_error(error, id)
            }
        })?;

    let metadata: StoredScenarioMetadata =
        serde_json::from_slice(&metadata_bytes).map_err(|error| {
            tracing::error!(
                error = %error,
                scenario_id = %id,
                "Unable to deserialize scenario metadata"
            );

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to read the scenario",
            )
        })?;

    let scenario_bytes = tokio::fs::read(scenario_dir.join("original.aoe2scenario"))
        .await
        .map_err(|error| storage_error(error, id))?;

    let safe_filename: String = metadata
        .original_filename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect();

    let content_disposition = format!("attachment; filename=\"{safe_filename}\"");

    let mut response = scenario_bytes.into_response();

    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );

    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition).map_err(|error| {
            tracing::error!(
                error = %error,
                scenario_id = %id,
                "Unable to create download filename header"
            );

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to download the scenario",
            )
        })?,
    );

    Ok(response)
}

async fn get_scenario(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Response, ApiError> {
    let metadata_path = state
        .scenario_data_dir
        .join(id.to_string())
        .join("metadata.json");

    let metadata = tokio::fs::read(metadata_path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            error_response(StatusCode::NOT_FOUND, "Scenario not found")
        } else {
            tracing::error!(
                error = %error,
                scenario_id = %id,
                "Unable to read scenario metadata"
            );

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to read the scenario",
            )
        }
    })?;

    Ok(([(header::CONTENT_TYPE, "application/json")], metadata).into_response())
}

async fn get_minimap_layer(
    State(state): State<AppState>,
    Path((id, layer)): Path<(uuid::Uuid, String)>,
) -> Result<Response, ApiError> {
    let file_name = match layer.as_str() {
        "terrain" => "terrain.png",
        "gaia" => "gaia.png",
        "players" => "players.png",
        _ => {
            return Err(error_response(
                StatusCode::NOT_FOUND,
                "Minimap layer not found",
            ));
        }
    };

    let path = state
        .scenario_data_dir
        .join(id.to_string())
        .join("minimap")
        .join(file_name);

    let bytes = tokio::fs::read(path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            error_response(StatusCode::NOT_FOUND, "Minimap layer not found")
        } else {
            tracing::error!(
                error = %error,
                scenario_id = %id,
                layer = %layer,
                "Unable to read minimap layer"
            );

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to read minimap layer",
            )
        }
    })?;

    Ok(([(header::CONTENT_TYPE, "image/png")], bytes).into_response())
}

async fn list_scenarios(
    State(state): State<AppState>,
) -> Result<Json<Vec<ScenarioListItem>>, ApiError> {
    let rows = sqlx::query(
        r#"
        SELECT
            id,
            original_filename,
            EXTRACT(EPOCH FROM uploaded_at)::BIGINT AS uploaded_at,
            file_size
        FROM scenarios
        ORDER BY uploaded_at DESC
        "#,
    )
    .fetch_all(&state.database)
    .await
    .map_err(|error| {
        tracing::error!(
            error = %error,
            "Unable to list scenarios from database"
        );

        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to list scenarios",
        )
    })?;

    let mut scenarios = Vec::new();

    for row in rows {
        let id: uuid::Uuid = row.try_get("id").map_err(|error| {
            tracing::error!(error = %error, "Unable to read scenario id from database");

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to list scenarios",
            )
        })?;

        let original_filename: String = row.try_get("original_filename").map_err(|error| {
            tracing::error!(error = %error, "Unable to read scenario filename from database");

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to list scenarios",
            )
        })?;

        let uploaded_at: i64 = row.try_get("uploaded_at").map_err(|error| {
            tracing::error!(error = %error, "Unable to read scenario timestamp from database");

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to list scenarios",
            )
        })?;

        let file_size: i64 = row.try_get("file_size").map_err(|error| {
            tracing::error!(error = %error, "Unable to read scenario file size from database");

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to list scenarios",
            )
        })?;

        let uploaded_at = u64::try_from(uploaded_at).map_err(|error| {
            tracing::error!(error = %error, "Invalid scenario timestamp in database");

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to list scenarios",
            )
        })?;

        let file_size = usize::try_from(file_size).map_err(|error| {
            tracing::error!(error = %error, "Invalid scenario file size in database");

            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to list scenarios",
            )
        })?;

        scenarios.push(ScenarioListItem {
            id,
            original_filename,
            uploaded_at,
            file_size,
        });
    }

    Ok(Json(scenarios))
}

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    if let Err(error) = run().await {
        tracing::error!(
            error = %error,
            "Server startup failed"
        );

        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let scenario_data_dir =
        std::env::var("SCENARIO_DATA_DIR").unwrap_or_else(|_| "data/scenarios".to_string());

    let database_url = std::env::var("DATABASE_URL")?;

    let database = PgPool::connect(&database_url).await?;

    sqlx::migrate!("./migrations")
        .run(&database)
        .await?;

    let state = AppState {
        parser_slots: Arc::new(Semaphore::new(1)),
        scenario_data_dir: Arc::new(scenario_data_dir.into()),
        database,
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/scenario/{id}", get(scenario_page))
        .route("/healthz", get(healthz))
        .route(
            "/api/scenario",
            post(upload_scenario).layer(DefaultBodyLimit::max(10 * 1024 * 1024)),
        )
        .route("/api/scenarios", get(list_scenarios))
        .route(
            "/api/scenario/{id}",
            get(get_scenario).delete(delete_scenario),
        )
        .route("/api/scenario/{id}/download", get(download_scenario))
        .route(
            "/api/scenario/{id}/minimap/{layer}",
            get(get_minimap_layer),
        )
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:8080").await?;

    axum::serve(listener, app).await?;

    Ok(())
}
