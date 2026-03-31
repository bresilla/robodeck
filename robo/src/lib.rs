use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_big_array::BigArray;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Cursor, net::SocketAddr, path::Path, sync::Arc};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};
use tower_http::services::{ServeDir, ServeFile};

const APP_ADDR: &str = "0.0.0.0:38080";
const DIST_DIR: &str = "dist";

#[derive(Clone)]
struct AppState {
    zenoh: Arc<Mutex<ZenohManager>>,
}

#[derive(Default)]
struct ZenohManager {
    session: Option<zenoh::Session>,
    robot_watches: Vec<JoinHandle<()>>,
    robots: BTreeMap<String, RobotSummary>,
    status: ZenohStatusResponse,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ZenohConnectionType {
    Ws,
    #[default]
    Tcp,
    Udp,
    Quic,
}

impl ZenohConnectionType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ws => "ws",
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Quic => "quic",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ZenohConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZenohConnectRequest {
    pub endpoint: String,
    pub connection_type: ZenohConnectionType,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ZenohStatusResponse {
    pub state: ZenohConnectionState,
    pub endpoint: String,
    pub connection_type: ZenohConnectionType,
    pub status: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RobotSummary {
    pub name: String,
    pub odom_key: String,
    pub gnss_keys: Vec<String>,
    pub gnss_lat: Option<f64>,
    pub gnss_lon: Option<f64>,
    pub odom_x: Option<f64>,
    pub odom_y: Option<f64>,
    pub yaw_rad: Option<f64>,
    pub last_seen_ms: u64,
}

impl ZenohStatusResponse {
    fn disconnected() -> Self {
        Self {
            state: ZenohConnectionState::Disconnected,
            endpoint: String::new(),
            connection_type: ZenohConnectionType::Tcp,
            status: "Not connected.".into(),
        }
    }
}

pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    ensure_frontend_dist_exists()?;

    let state = AppState {
        zenoh: Arc::new(Mutex::new(ZenohManager {
            session: None,
            robot_watches: Vec::new(),
            robots: BTreeMap::new(),
            status: ZenohStatusResponse::disconnected(),
        })),
    };

    let static_files = ServeDir::new(DIST_DIR).not_found_service(ServeFile::new("dist/index.html"));
    let app = Router::new()
        .route("/api/zenoh/status", get(zenoh_status))
        .route("/api/zenoh/connect", post(zenoh_connect))
        .route("/api/zenoh/disconnect", post(zenoh_disconnect))
        .route("/api/robots", get(robots))
        .fallback_service(static_files)
        .with_state(state.clone());

    let addr: SocketAddr = APP_ADDR.parse()?;
    let listener = TcpListener::bind(addr).await?;

    println!("uimap: serving UI and zenoh api on http://127.0.0.1:38080");
    println!("uimap: open http://127.0.0.1:38080 in your browser");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(state))
        .await?;

    Ok(())
}

async fn zenoh_status(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.zenoh.lock().await.status.clone())
}

async fn robots(State(state): State<AppState>) -> impl IntoResponse {
    let robots = state
        .zenoh
        .lock()
        .await
        .robots
        .values()
        .cloned()
        .collect::<Vec<_>>();
    Json(robots)
}

async fn zenoh_connect(
    State(state): State<AppState>,
    Json(request): Json<ZenohConnectRequest>,
) -> impl IntoResponse {
    let endpoint = request.endpoint.trim().to_string();
    if endpoint.is_empty() {
        let response = ZenohStatusResponse {
            state: ZenohConnectionState::Error,
            endpoint,
            connection_type: request.connection_type,
            status: "Enter a Zenoh endpoint first.".into(),
        };
        state.zenoh.lock().await.status = response.clone();
        return (StatusCode::BAD_REQUEST, Json(response));
    }

    disconnect_existing_session(&state).await;

    {
        let mut manager = state.zenoh.lock().await;
        manager.status = ZenohStatusResponse {
            state: ZenohConnectionState::Connecting,
            endpoint: endpoint.clone(),
            connection_type: request.connection_type,
            status: format!(
                "Connecting to {} via {}...",
                endpoint,
                request.connection_type.as_str()
            ),
        };
    }

    match open_zenoh_session(&request).await {
        Ok(session) => {
            let odom_watch = tokio::spawn(watch_robot_topics(
                state.clone(),
                session.clone(),
                "**/odom",
                TopicKind::Odom,
            ));
            let gnss_watch = tokio::spawn(watch_robot_topics(
                state.clone(),
                session.clone(),
                "**/gnss/**",
                TopicKind::Gnss,
            ));
            let gnss_root_watch = tokio::spawn(watch_robot_topics(
                state.clone(),
                session.clone(),
                "**/gnss",
                TopicKind::Gnss,
            ));
            let activity_watch = tokio::spawn(watch_robot_activity(state.clone(), session.clone()));
            let response = ZenohStatusResponse {
                state: ZenohConnectionState::Connected,
                endpoint: endpoint.clone(),
                connection_type: request.connection_type,
                status: format!(
                    "Connected to {} via {}.",
                    endpoint,
                    request.connection_type.as_str()
                ),
            };
            let mut manager = state.zenoh.lock().await;
            manager.session = Some(session);
            manager.robot_watches = vec![odom_watch, gnss_watch, gnss_root_watch, activity_watch];
            manager.status = response.clone();
            (StatusCode::OK, Json(response))
        }
        Err(error) => {
            let response = ZenohStatusResponse {
                state: ZenohConnectionState::Error,
                endpoint,
                connection_type: request.connection_type,
                status: format!("Zenoh connect failed: {error}"),
            };
            state.zenoh.lock().await.status = response.clone();
            (StatusCode::BAD_GATEWAY, Json(response))
        }
    }
}

async fn zenoh_disconnect(State(state): State<AppState>) -> impl IntoResponse {
    disconnect_existing_session(&state).await;
    let response = ZenohStatusResponse::disconnected();
    state.zenoh.lock().await.status = response.clone();
    (StatusCode::OK, Json(response))
}

async fn disconnect_existing_session(state: &AppState) {
    let (session, watches) = {
        let mut manager = state.zenoh.lock().await;
        manager.robots.clear();
        (manager.session.take(), std::mem::take(&mut manager.robot_watches))
    };

    for watch in watches {
        watch.abort();
    }

    if let Some(session) = session {
        let _ = session.close().await;
    }
}

async fn shutdown_signal(state: AppState) {
    let _ = tokio::signal::ctrl_c().await;
    println!("uimap: shutting down server");
    disconnect_existing_session(&state).await;
}

fn ensure_frontend_dist_exists() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if Path::new(DIST_DIR).join("index.html").exists() {
        Ok(())
    } else {
        Err("frontend assets are missing; run `trunk build` first so `dist/index.html` exists".into())
    }
}

async fn open_zenoh_session(
    request: &ZenohConnectRequest,
) -> Result<zenoh::Session, Box<dyn std::error::Error + Send + Sync>> {
    let locator = normalize_locator(request.connection_type, &request.endpoint)?;
    let mut config = zenoh::Config::default();
    config.insert_json5("mode", "\"client\"")?;
    config.insert_json5("scouting/multicast/enabled", "false")?;
    config.insert_json5("connect/endpoints", &serde_json::to_string(&vec![locator])?)?;
    let session = zenoh::open(config).await?;
    Ok(session)
}

fn normalize_locator(
    connection_type: ZenohConnectionType,
    endpoint: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return Err("empty endpoint".into());
    }

    if trimmed.contains('/') && !trimmed.contains("://") {
        return Ok(trimmed.to_string());
    }

    let without_scheme = trimmed
        .strip_prefix("tcp://")
        .or_else(|| trimmed.strip_prefix("udp://"))
        .or_else(|| trimmed.strip_prefix("quic://"))
        .or_else(|| trimmed.strip_prefix("ws://"))
        .unwrap_or(trimmed);

    Ok(format!("{}/{}", connection_type.as_str(), without_scheme))
}

#[derive(Clone, Copy)]
enum TopicKind {
    Odom,
    Gnss,
}

async fn watch_robot_topics(
    state: AppState,
    session: zenoh::Session,
    key_expr: &'static str,
    topic_kind: TopicKind,
) {
    let subscriber = match session.declare_subscriber(key_expr).await {
        Ok(subscriber) => subscriber,
        Err(error) => {
            state.zenoh.lock().await.status = ZenohStatusResponse {
                state: ZenohConnectionState::Error,
                endpoint: String::new(),
                connection_type: ZenohConnectionType::Tcp,
                status: format!("Zenoh robot watcher failed for {key_expr}: {error}"),
            };
            return;
        }
    };

    while let Ok(sample) = subscriber.recv_async().await {
        let key = sample.key_expr().as_str();
        let Some(robot_name) = robot_name_from_key(key, topic_kind) else {
            continue;
        };

        let mut manager = state.zenoh.lock().await;
        let robot = manager
            .robots
            .entry(robot_name.clone())
            .or_insert_with(|| RobotSummary {
                name: robot_name,
                odom_key: String::new(),
                gnss_keys: Vec::new(),
                gnss_lat: None,
                gnss_lon: None,
                odom_x: None,
                odom_y: None,
                yaw_rad: None,
                last_seen_ms: 0,
            });

        robot.last_seen_ms = now_ms();

        match topic_kind {
            TopicKind::Odom => {
                robot.odom_key = key.to_string();
                if let Ok(odom) = decode_ros_message::<RosOdometry>(sample.payload().to_bytes().as_ref()) {
                    robot.odom_x = Some(odom.pose.pose.position.x);
                    robot.odom_y = Some(odom.pose.pose.position.y);
                    robot.yaw_rad = Some(quaternion_to_yaw(&odom.pose.pose.orientation));
                }
            }
            TopicKind::Gnss => {
                let next = key.to_string();
                if !robot.gnss_keys.contains(&next) {
                    robot.gnss_keys.push(next);
                    robot.gnss_keys.sort();
                }
                if let Ok(fix) = decode_ros_message::<RosNavSatFix>(sample.payload().to_bytes().as_ref()) {
                    robot.gnss_lat = Some(fix.latitude);
                    robot.gnss_lon = Some(fix.longitude);
                }
            }
        }
    }
}

async fn watch_robot_activity(state: AppState, session: zenoh::Session) {
    let subscriber = match session.declare_subscriber("**").await {
        Ok(subscriber) => subscriber,
        Err(error) => {
            state.zenoh.lock().await.status = ZenohStatusResponse {
                state: ZenohConnectionState::Error,
                endpoint: String::new(),
                connection_type: ZenohConnectionType::Tcp,
                status: format!("Zenoh activity watcher failed: {error}"),
            };
            return;
        }
    };

    while let Ok(sample) = subscriber.recv_async().await {
        let key = sample.key_expr().as_str().trim_matches('/');
        let segments = key.split('/').collect::<Vec<_>>();
        if segments.is_empty() {
            continue;
        }

        let mut manager = state.zenoh.lock().await;
        let now = now_ms();
        for robot in manager.robots.values_mut() {
            if key_matches_robot(&segments, &robot.name) {
                robot.last_seen_ms = now;
            }
        }
    }
}

fn key_matches_robot(segments: &[&str], robot_name: &str) -> bool {
    let full_key = segments.join("/");
    segments.iter().any(|segment| segment.contains(robot_name))
        || full_key == robot_name
        || full_key.starts_with(&format!("{robot_name}/"))
        || full_key.contains(&format!("/{robot_name}/"))
        || full_key.ends_with(&format!("/{robot_name}"))
}

fn robot_name_from_key(key: &str, topic_kind: TopicKind) -> Option<String> {
    let trimmed = key.trim_matches('/');
    let prefix = match topic_kind {
        TopicKind::Odom => trimmed.strip_suffix("/odom")?,
        TopicKind::Gnss => trimmed.split_once("/gnss")?.0,
    };
    let name = prefix.rsplit('/').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn decode_ros_message<T>(payload: &[u8]) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    T: for<'de> Deserialize<'de>,
{
    let mut cursor = Cursor::new(payload);
    Ok(cdr::deserialize_from(&mut cursor, cdr::size::Infinite)?)
}

fn quaternion_to_yaw(quaternion: &RosQuaternion) -> f64 {
    let siny_cosp = 2.0 * (quaternion.w * quaternion.z + quaternion.x * quaternion.y);
    let cosy_cosp = 1.0 - 2.0 * (quaternion.y * quaternion.y + quaternion.z * quaternion.z);
    siny_cosp.atan2(cosy_cosp)
}

#[derive(Clone, Debug, Deserialize)]
struct RosTime {
    sec: i32,
    nanosec: u32,
}

#[derive(Clone, Debug, Deserialize)]
struct RosHeader {
    stamp: RosTime,
    frame_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct RosNavSatStatus {
    status: i8,
    service: u16,
}

#[derive(Clone, Debug, Deserialize)]
struct RosNavSatFix {
    header: RosHeader,
    status: RosNavSatStatus,
    latitude: f64,
    longitude: f64,
    altitude: f64,
    position_covariance: [f64; 9],
    position_covariance_type: u8,
}

#[derive(Clone, Debug, Deserialize)]
struct RosPoint {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct RosQuaternion {
    x: f64,
    y: f64,
    z: f64,
    w: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct RosPose {
    position: RosPoint,
    orientation: RosQuaternion,
}

#[derive(Clone, Debug, Deserialize)]
struct RosPoseWithCovariance {
    pose: RosPose,
    #[serde(with = "BigArray")]
    covariance: [f64; 36],
}

#[derive(Clone, Debug, Deserialize)]
struct RosVector3 {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct RosTwist {
    linear: RosVector3,
    angular: RosVector3,
}

#[derive(Clone, Debug, Deserialize)]
struct RosTwistWithCovariance {
    twist: RosTwist,
    #[serde(with = "BigArray")]
    covariance: [f64; 36],
}

#[derive(Clone, Debug, Deserialize)]
struct RosOdometry {
    header: RosHeader,
    child_frame_id: String,
    pose: RosPoseWithCovariance,
    twist: RosTwistWithCovariance,
}
