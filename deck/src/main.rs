use leptos::prelude::*;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{
    Event, FileReader, HtmlAnchorElement, HtmlElement, HtmlInputElement, MouseEvent, ProgressEvent,
    Request, RequestCache, RequestInit, Response,
};

thread_local! {
    static MAP_HANDLE: RefCell<Option<JsValue>> = const { RefCell::new(None) };
    static ACTIVE_MARKERS: RefCell<Vec<JsValue>> = const { RefCell::new(Vec::new()) };
    static LAST_ZONE_INSERT: RefCell<(String, f64)> = const { RefCell::new((String::new(), 0.0)) };
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct JsonPoint {
    lat: f64,
    lon: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct ZoneJson {
    id: String,
    name: String,
    #[serde(rename = "type")]
    zone_type: String,
    parent_id: String,
    child_ids: Vec<String>,
    #[serde(default)]
    node_ids: Vec<String>,
    polygon_latlon: Vec<JsonPoint>,
    grid_enabled: bool,
    grid_resolution: f64,
    #[serde(default)]
    properties: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum CoordMode {
    #[default]
    Global,
    Local,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct NodeJson {
    id: String,
    name: String,
    latlon: JsonPoint,
    #[serde(default)]
    zone_ids: Vec<String>,
    #[serde(default)]
    properties: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct EdgeJson {
    id: String,
    source_id: String,
    target_id: String,
    directed: bool,
    weight: f64,
    #[serde(default)]
    zone_ids: Vec<String>,
    #[serde(default)]
    properties: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct WorkspaceJson {
    name: String,
    root_zone_id: String,
    #[serde(default)]
    coord_mode: CoordMode,
    #[serde(rename = "ref")]
    ref_point: Option<JsonPoint>,
    #[serde(default, skip_serializing)]
    local_ref: bool,
    datum: Option<JsonPoint>,
    #[serde(default)]
    zones: BTreeMap<String, ZoneJson>,
    #[serde(default)]
    nodes: BTreeMap<String, NodeJson>,
    #[serde(default)]
    edges: BTreeMap<String, EdgeJson>,
}

impl Default for WorkspaceJson {
    fn default() -> Self {
        Self {
            name: "Workspace".into(),
            root_zone_id: String::new(),
            coord_mode: CoordMode::Global,
            ref_point: None,
            local_ref: false,
            datum: Some(JsonPoint {
                lat: 52.0,
                lon: 5.0,
            }),
            zones: BTreeMap::new(),
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    Workspace,
    Zone(String),
    Node(String),
    Edge(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Inspect,
    AddNode,
    ConnectEdge,
    PlaceRef,
    PlaceZone,
}

#[derive(Clone, Debug, PartialEq)]
enum SceneDrag {
    Node(String),
    ZoneVertex(String, usize),
    ZoneBody(String, JsonPoint),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeftPanel {
    Reference,
    Zones,
    Graph,
    Zenoh,
    Robots,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RightPanel {
    Details,
    Json,
    Files,
    Scheduler,
    Tasks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppMode {
    Editor,
    Management,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ZenohConnectionType {
    Ws,
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

    fn from_value(value: &str) -> Self {
        match value {
            "tcp" => Self::Tcp,
            "udp" => Self::Udp,
            "quic" => Self::Quic,
            _ => Self::Ws,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Ws => "WebSocket",
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
            Self::Quic => "QUIC",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ZenohConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Clone, Debug, Serialize)]
struct ZenohConnectRequest {
    endpoint: String,
    connection_type: ZenohConnectionType,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ZenohStatusResponse {
    state: ZenohConnectionState,
    endpoint: String,
    connection_type: ZenohConnectionType,
    status: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SchedulerTasksResponse {
    tasks: Vec<String>,
    status: String,
}

#[derive(Clone, Debug, Serialize)]
struct SchedulerRunRequest {
    robot: String,
    task: String,
    node_id: String,
    node_name: String,
    lat: f64,
    lon: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SchedulerRunResponse {
    status: String,
}

const ORDER_OPTIONS: [&str; 2] = ["goto", "patrol"];

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct RobotSummary {
    name: String,
    #[serde(default)]
    available_tasks: Vec<String>,
    odom_key: String,
    gnss_keys: Vec<String>,
    gnss_lat: Option<f64>,
    gnss_lon: Option<f64>,
    odom_x: Option<f64>,
    odom_y: Option<f64>,
    yaw_rad: Option<f64>,
    last_seen_ms: u64,
    stale: bool,
    #[serde(skip, default = "random_robot_color")]
    color: String,
}

impl Default for ZenohConnectionType {
    fn default() -> Self {
        Self::Tcp
    }
}

impl Default for ZenohConnectionState {
    fn default() -> Self {
        Self::Disconnected
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(|| view! { <App/> });
}

#[component]
fn App() -> impl IntoView {
    let workspace = RwSignal::new(WorkspaceJson::default());
    let app_mode = RwSignal::new(AppMode::Editor);
    let selection = RwSignal::new(Selection::Workspace);
    let mode = RwSignal::new(Mode::Inspect);
    let edge_source_id = RwSignal::new(String::new());
    let hovered_node_id = RwSignal::new(String::new());
    let hovered_edge_id = RwSignal::new(String::new());
    let preview_mouse_point = RwSignal::new(None::<JsonPoint>);
    let scene_drag = RwSignal::new(None::<SceneDrag>);
    let zone_draft_points = RwSignal::new(Vec::<JsonPoint>::new());
    let pending_zone_parent = RwSignal::new(None::<String>);
    let active_left_panel = RwSignal::new(Some(LeftPanel::Zones));
    let active_right_panel = RwSignal::new(Some(RightPanel::Files));
    let management_left_panel = RwSignal::new(Some(LeftPanel::Robots));
    let management_right_panel = RwSignal::new(None::<RightPanel>);
    let status = RwSignal::new(String::from("Ready. Add a root zone or paste workspace JSON."));
    let raw_json = RwSignal::new(
        serde_json::to_string_pretty(&workspace.get()).unwrap_or_else(|_| "{}".into()),
    );
    let zenoh_endpoint = RwSignal::new(String::from("localhost:7447"));
    let zenoh_connection_type = RwSignal::new(ZenohConnectionType::Tcp);
    let zenoh_state = RwSignal::new(ZenohConnectionState::Disconnected);
    let zenoh_status = RwSignal::new(String::from("Not connected."));
    let robots = RwSignal::new(Vec::<RobotSummary>::new());
    let robot_clock_ms = RwSignal::new(now_ms());
    let selected_robot_name = RwSignal::new(String::new());
    let schedule_selected_task = RwSignal::new(String::new());
    let scheduler_task_options = RwSignal::new(Vec::<String>::new());
    let scheduler_selected_task = RwSignal::new(String::new());
    let scheduler_target_node_id = RwSignal::new(String::new());
    let scheduler_picking_node = RwSignal::new(false);
    let map_host = NodeRef::<leptos::html::Div>::new();
    let map_initialized = RwSignal::new(false);
    let map_view_tick = RwSignal::new(0_u64);
    let map_size = RwSignal::new((1000.0_f64, 1000.0_f64));
    let marker_dragging = RwSignal::new(false);

    let map_init_tick = map_view_tick;
    let map_init_size = map_size;
    Effect::new(move |_| {
        if map_initialized.get() {
            return;
        }
        let Some(host) = map_host.get() else {
            return;
        };
        let center = workspace.with(workspace_center);
        let host: HtmlElement = host.unchecked_into();
        if let Ok(map) = init_basemap(host.clone(), &center, map_init_tick, map_init_size) {
            let _ = bind_map_interactions(
                &map,
                workspace,
                app_mode,
                selection,
                mode,
                edge_source_id,
                hovered_node_id,
                hovered_edge_id,
                preview_mouse_point,
                scene_drag,
                zone_draft_points,
                pending_zone_parent,
                active_right_panel,
                management_right_panel,
                scheduler_picking_node,
                status,
                raw_json,
                marker_dragging,
            );
            MAP_HANDLE.with(|cell| {
                *cell.borrow_mut() = Some(map);
            });
            map_initialized.set(true);
        }
    });

    Effect::new(move |_| {
        let ws = workspace.get();
        let selected = selection.get();
        let edge_source = edge_source_id.get();
        let hovered_node = hovered_node_id.get();
        let hovered_edge = hovered_edge_id.get();
        let preview_point = preview_mouse_point.get();
        let zone_draft = zone_draft_points.get();
        let dragging = marker_dragging.get();
        let current_app_mode = app_mode.get();
        let now_ms = robot_clock_ms.get();
        let robot_state = visible_robots(&robots.get(), now_ms);
        let _ = map_view_tick.get();
        MAP_HANDLE.with(|cell| {
            if let Some(map) = cell.borrow().as_ref() {
                if map_style_loaded(map) {
                    let _ = ensure_map_layers(map);
                    let _ = update_map_sources(
                        map,
                        &ws,
                        &selected,
                        &edge_source,
                        &hovered_node,
                        &hovered_edge,
                        preview_point.as_ref(),
                        &zone_draft,
                    );
                    if current_app_mode == AppMode::Editor && !dragging {
                        let _ = render_markers(
                            map,
                            workspace,
                            selection,
                            edge_source_id,
                            hovered_edge_id,
                            raw_json,
                            active_right_panel,
                            status,
                            marker_dragging,
                        );
                    } else if current_app_mode == AppMode::Management {
                        let _ = render_robot_markers(map, &ws, &robot_state);
                    } else if current_app_mode != AppMode::Editor {
                        clear_markers();
                    }
                }
            }
        });
    });

    Effect::new(move |_| {
        if !scheduler_picking_node.get() {
            return;
        }
        if let Selection::Node(node_id) = selection.get() {
            scheduler_target_node_id.set(node_id.clone());
            scheduler_picking_node.set(false);
            status.set(format!("Selected node {node_id} for the scheduler."));
        }
    });

    Effect::new(move |_| {
        let selected_robot = selected_robot_name.get();
        let right_panel = management_right_panel.get();
        let has_tasks = robot_has_available_tasks(&robots.get(), &selected_robot);
        if right_panel == Some(RightPanel::Tasks) && !has_tasks {
            management_right_panel.set(None);
        }
    });

    Effect::new(move |_| {
        if mode.get() != Mode::PlaceZone && !zone_draft_points.with(|points| points.is_empty()) {
            zone_draft_points.set(Vec::new());
        }
    });

    Effect::new(move |_| {
        if app_mode.get() == AppMode::Editor {
            return;
        }
        mode.set(Mode::Inspect);
        edge_source_id.set(String::new());
        preview_mouse_point.set(None);
        zone_draft_points.set(Vec::new());
        pending_zone_parent.set(None);
        scene_drag.set(None);
        marker_dragging.set(false);
        clear_markers();
    });

    let sync_json = move || {
        raw_json.set(serde_json::to_string_pretty(&workspace.get()).unwrap_or_else(|_| "{}".into()));
    };

    let update_workspace = move |message: &'static str, open_details: bool| {
        sync_associations(&workspace);
        sync_json();
        if open_details {
            active_right_panel.set(Some(RightPanel::Details));
        }
        status.set(message.into());
    };

    let arm_zone_placement = move |parent_id: Option<String>| {
        if parent_id.is_none() && workspace.with(|ws| !ws.root_zone_id.is_empty()) {
            status.set("Root zone already exists. Use the + button on a zone to add a child.".into());
            return;
        }
        zone_draft_points.set(Vec::new());
        pending_zone_parent.set(parent_id);
        mode.set(Mode::PlaceZone);
        status.set("Click the map to add zone points. Right-click to finish the polygon.".into());
    };

    let remove_node = move |node_id: String| {
        workspace.update(|ws| {
            ws.nodes.remove(&node_id);
            ws.edges
                .retain(|_, edge| edge.source_id != node_id && edge.target_id != node_id);
        });
        selection.set(Selection::Workspace);
        update_workspace("Removed node.", false);
    };

    let remove_edge = move |edge_id: String| {
        workspace.update(|ws| {
            ws.edges.remove(&edge_id);
        });
        selection.set(Selection::Workspace);
        update_workspace("Removed edge.", false);
    };

    let switch_to_editor = move |_| {
        app_mode.set(AppMode::Editor);
        status.set("Editor mode.".into());
    };

    let switch_to_management = move |_| {
        app_mode.set(AppMode::Management);
        status.set("Management mode.".into());
    };

    let toggle_left_panel = move |panel: LeftPanel| {
        active_left_panel.update(|active| {
            *active = if *active == Some(panel) {
                None
            } else {
                Some(panel)
            };
        });
    };

    let toggle_right_panel = move |panel: RightPanel| {
        active_right_panel.update(|active| {
            *active = if *active == Some(panel) {
                None
            } else {
                Some(panel)
            };
        });
    };

    let toggle_management_left_panel = move |panel: LeftPanel| {
        management_left_panel.update(|active| {
            *active = if *active == Some(panel) {
                None
            } else {
                Some(panel)
            };
        });
    };

    let toggle_management_right_panel = move |panel: RightPanel| {
        management_right_panel.update(|active| {
            *active = if *active == Some(panel) {
                None
            } else {
                Some(panel)
            };
        });
    };

    let open_scheduler_panel = move |_| {
        let next = if management_right_panel.get_untracked() == Some(RightPanel::Scheduler) {
            None
        } else {
            Some(RightPanel::Scheduler)
        };
        management_right_panel.set(next);
    };

    let open_tasks_panel = move |_| {
        let robot = selected_robot_name.get_untracked();
        if !robot_has_available_tasks(&robots.get_untracked(), &robot) {
            management_right_panel.set(None);
            status.set("No task topic detected for the selected robot.".into());
            return;
        }
        let next = if management_right_panel.get_untracked() == Some(RightPanel::Tasks) {
            None
        } else {
            Some(RightPanel::Tasks)
        };
        management_right_panel.set(next);
        if next.is_some() {
            if !robot.is_empty() {
                spawn_local(refresh_scheduler_tasks(
                    robot,
                    scheduler_task_options,
                    scheduler_selected_task,
                    status,
                ));
            }
        }
    };

    let arm_scheduler_node_pick = move |_| {
        let next = !scheduler_picking_node.get_untracked();
        scheduler_picking_node.set(next);
        if next {
            scheduler_target_node_id.set(String::new());
            status.set("Scheduler node pick armed. Click one node.".into());
        } else {
            status.set("Scheduler node pick cancelled.".into());
        }
    };

    let run_scheduler_task = move |_| {
        let robot = selected_robot_name.get_untracked();
        let task = scheduler_selected_task.get_untracked();
        let node_id = scheduler_target_node_id.get_untracked();
        let Some(node) = workspace.get_untracked().nodes.get(&node_id).cloned() else {
            status.set("Select a scheduler node first.".into());
            return;
        };
        if robot.is_empty() {
            status.set("Select a robot first.".into());
            return;
        }
        if task.is_empty() {
            status.set("Select a task first.".into());
            return;
        }
        status.set(format!("Sending {task} to {robot}..."));
        spawn_local(run_scheduler_task_api(
            SchedulerRunRequest {
                robot,
                task,
                node_id: node.id,
                node_name: node.name,
                lat: node.latlon.lat,
                lon: node.latlon.lon,
            },
            status,
        ));
    };

    let run_schedule_task = move |_| {
        let robot = selected_robot_name.get_untracked();
        let task = schedule_selected_task.get_untracked();
        let node_id = scheduler_target_node_id.get_untracked();
        let Some(node) = workspace.get_untracked().nodes.get(&node_id).cloned() else {
            status.set("Select a scheduler node first.".into());
            return;
        };
        if robot.is_empty() {
            status.set("Select a robot first.".into());
            return;
        }
        if task.is_empty() {
            status.set("Select a schedule first.".into());
            return;
        }
        status.set(format!("Sending {task} schedule to {robot}..."));
        spawn_local(run_scheduler_task_api(
            SchedulerRunRequest {
                robot,
                task,
                node_id: node.id,
                node_name: node.name,
                lat: node.latlon.lat,
                lon: node.latlon.lon,
            },
            status,
        ));
    };

    let apply_json = move |_: MouseEvent| match serde_json::from_str::<WorkspaceJson>(&raw_json.get()) {
        Ok(mut next) => {
            normalize_workspace_json(&mut next);
            workspace.set(next);
            update_workspace("Applied raw JSON.", false);
            selection.set(Selection::Workspace);
            active_right_panel.set(Some(RightPanel::Files));
        }
        Err(error) => status.set(format!("Invalid JSON: {error}")),
    };

    let format_json = move |_: MouseEvent| match serde_json::from_str::<WorkspaceJson>(&raw_json.get()) {
        Ok(mut next) => {
            normalize_workspace_json(&mut next);
            workspace.set(next);
            selection.set(Selection::Workspace);
            active_right_panel.set(Some(RightPanel::Json));
            update_workspace("Formatted current JSON.", false);
        }
        Err(error) => status.set(format!("Invalid JSON: {error}")),
    };

    let new_workspace = move |_: MouseEvent| {
        workspace.set(WorkspaceJson::default());
        selection.set(Selection::Workspace);
        active_right_panel.set(Some(RightPanel::Files));
        mode.set(Mode::Inspect);
        edge_source_id.set(String::new());
        update_workspace("Reset workspace.", false);
    };

    let open_workspace = move |_: MouseEvent| {
        open_workspace_json_file(workspace, selection, active_right_panel, raw_json, status);
    };

    let save_workspace = move |_: MouseEvent| {
        save_workspace_json(&raw_json.get(), status);
    };

    let locate_me_action = move |_: MouseEvent| {
        MAP_HANDLE.with(|cell| {
            if let Some(map) = cell.borrow().as_ref() {
                locate_me(map, workspace, status);
            } else {
                status.set("Map is not ready for geolocation yet.".into());
            }
        });
    };

    let graph_nodes = move || workspace.with(|ws| ws.nodes.values().cloned().collect::<Vec<_>>());
    let graph_edges = move || workspace.with(|ws| ws.edges.values().cloned().collect::<Vec<_>>());
    let validation = move || validation_messages(&workspace.get());

    {
        let poll_state = zenoh_state;
        let poll_status = zenoh_status;
        let poll_endpoint = zenoh_endpoint;
        let poll_type = zenoh_connection_type;
        let poll_robots = robots;
        let robot_clock = robot_clock_ms;
        let app_status = status;
        Effect::new(move |_| {
            spawn_local(refresh_zenoh_status(
                poll_state,
                poll_status,
                poll_endpoint,
                poll_type,
                app_status,
                false,
            ));
            spawn_local(refresh_robots(poll_robots));
            robot_clock.set(now_ms());

            let callback = Closure::<dyn FnMut()>::wrap(Box::new(move || {
                spawn_local(refresh_zenoh_status(
                    poll_state,
                    poll_status,
                    poll_endpoint,
                    poll_type,
                    app_status,
                    false,
                ));
                spawn_local(refresh_robots(poll_robots));
                robot_clock.set(now_ms());
            }));

            if let Some(window) = web_sys::window() {
                let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
                    callback.as_ref().unchecked_ref(),
                    2000,
                );
            }

            callback.forget();
        });
    }

    let connect_zenoh = move |_: MouseEvent| {
        let endpoint = zenoh_endpoint.get().trim().to_string();
        let connection_type = zenoh_connection_type.get();

        if endpoint.is_empty() {
            zenoh_state.set(ZenohConnectionState::Error);
            zenoh_status.set("Enter a Zenoh endpoint first.".into());
            status.set("Zenoh endpoint is missing.".into());
            return;
        }

        zenoh_state.set(ZenohConnectionState::Connecting);
        zenoh_status.set(format!(
            "Connecting to {} via {}...",
            endpoint,
            connection_type.display_name()
        ));
        status.set(format!(
            "Requesting Rust backend Zenoh connection to {} via {}...",
            endpoint,
            connection_type.display_name()
        ));

        spawn_local(connect_zenoh_api(
            ZenohConnectRequest {
                endpoint,
                connection_type,
            },
            zenoh_state,
            zenoh_status,
            zenoh_endpoint,
            zenoh_connection_type,
            status,
        ));
    };

    let disconnect_zenoh = move |_: MouseEvent| {
        zenoh_state.set(ZenohConnectionState::Connecting);
        zenoh_status.set("Disconnecting...".into());
        status.set("Requesting Rust backend Zenoh disconnect...".into());
        spawn_local(disconnect_zenoh_api(
            zenoh_state,
            zenoh_status,
            zenoh_endpoint,
            zenoh_connection_type,
            status,
        ));
    };

    view! {
        <div class="shell">
            <main class="stage">
                <section class="workspace">
                    <div class="map-frame">
                        <div class="map">
                            <div node_ref=map_host class="map-base"></div>
                            <div class="map-watermark">
                                <span>"Leptos shell active"</span>
                            </div>
                        </div>

                        <div class="floating left-stack">
                            <Show
                                when=move || app_mode.get() == AppMode::Editor
                                fallback=move || view! {
                                    <button
                                        class="panel-toggle"
                                        class:is-active=move || management_left_panel.get() == Some(LeftPanel::Zenoh)
                                        on:click=move |_| toggle_management_left_panel(LeftPanel::Zenoh)
                                        type="button"
                                        title="Toggle Zenoh panel"
                                    >
                                        "Z"
                                    </button>
                                    <button
                                        class="panel-toggle"
                                        class:is-active=move || management_left_panel.get() == Some(LeftPanel::Robots)
                                        on:click=move |_| toggle_management_left_panel(LeftPanel::Robots)
                                        type="button"
                                        title="Toggle robots panel"
                                    >
                                        "R"
                                    </button>
                                }
                            >
                                <button
                                    class="panel-toggle"
                                    class:is-active=move || active_left_panel.get() == Some(LeftPanel::Reference)
                                    on:click=move |_| toggle_left_panel(LeftPanel::Reference)
                                    type="button"
                                    title="Toggle reference panel"
                                >
                                    "R"
                                </button>
                                <button
                                    class="panel-toggle"
                                    class:is-active=move || active_left_panel.get() == Some(LeftPanel::Zones)
                                    on:click=move |_| toggle_left_panel(LeftPanel::Zones)
                                    type="button"
                                    title="Toggle zones panel"
                                >
                                    "Z"
                                </button>
                                <button
                                    class="panel-toggle"
                                    class:is-active=move || active_left_panel.get() == Some(LeftPanel::Graph)
                                    on:click=move |_| toggle_left_panel(LeftPanel::Graph)
                                    type="button"
                                    title="Toggle graph panel"
                                >
                                    "G"
                                </button>
                            </Show>
                        </div>

                        <div class="floating left-panels">
                            <Show when=move || app_mode.get() == AppMode::Editor>
                            <section
                                class="card floating-panel left-panel-card"
                                class:is-hidden=move || active_left_panel.get() != Some(LeftPanel::Reference)
                            >
                                <div class="panel-head">
                                    <div>
                                        <p class="section">"Reference"</p>
                                        <h3>"Reference Point"</h3>
                                    </div>
                                </div>
                                <div class="mini-stats">
                                    <div class="mini-stat">
                                        <span>"Current"</span>
                                        <strong>
                                            {move || {
                                                workspace.get().ref_point.as_ref().map(point_text).unwrap_or_else(|| "Not set".into())
                                            }}
                                        </strong>
                                    </div>
                                </div>
                                <div class="panel-card">
                                    <div class="panel-head compact-head">
                                        <div>
                                            <p class="section">"Reference"</p>
                                            <h3>"Local Ref"</h3>
                                        </div>
                                    </div>
                                    <label class="field">
                                        <span>"Mode"</span>
                                        <select
                                            prop:value=move || (workspace.get().coord_mode == CoordMode::Local || workspace.get().local_ref).to_string()
                                            on:change=move |ev| {
                                                let value = event_target_value(&ev) == "true";
                                                workspace.update(|ws| {
                                                    ws.local_ref = value;
                                                    ws.coord_mode = if value { CoordMode::Local } else { CoordMode::Global };
                                                });
                                                update_workspace("Updated reference mode.", false);
                                            }
                                        >
                                            <option value="false">"Lat / Lon"</option>
                                            <option value="true">"Local X / Y"</option>
                                        </select>
                                    </label>
                                </div>
                                <div class="dock-actions single-action">
                                    <button
                                        class="primary"
                                        class:is-armed=move || mode.get() == Mode::PlaceRef
                                        on:click=move |_| {
                                            let next = if mode.get() == Mode::PlaceRef { Mode::Inspect } else { Mode::PlaceRef };
                                            mode.set(next);
                                            status.set(if next == Mode::PlaceRef {
                                                "Click the map to place the reference point.".into()
                                            } else {
                                                "Inspect mode.".into()
                                            });
                                        }
                                        type="button"
                                    >
                                        {move || if mode.get() == Mode::PlaceRef { "Stop Placing Reference" } else { "Add Reference Point" }}
                                    </button>
                                </div>
                                <p class="hint">{move || status.get()}</p>
                            </section>

                            <section
                                class="card floating-panel left-panel-card"
                                class:is-hidden=move || active_left_panel.get() != Some(LeftPanel::Zones)
                            >
                                <div class="panel-head">
                                    <div>
                                        <p class="section">"Hierarchy"</p>
                                        <h3>"Zones"</h3>
                                    </div>
                                    <span class="panel-stat">
                                        {move || workspace.get().zones.len().to_string()}
                                    </span>
                                </div>
                                <div class="mini-stats">
                                    <div class="mini-stat">
                                        <span>"Root"</span>
                                        <strong>
                                            {move || {
                                                let ws = workspace.get();
                                                ws.zones
                                                    .get(&ws.root_zone_id)
                                                    .map(|zone| zone.name.clone())
                                                    .unwrap_or_else(|| "-".into())
                                            }}
                                        </strong>
                                    </div>
                                </div>
                                <div class="dock-actions single-action">
                                    <button class="primary" on:click=move |_| arm_zone_placement(None) type="button">
                                        "Add Root Zone"
                                    </button>
                                </div>
                                <ZoneTree
                                    workspace=workspace
                                    selection=selection
                                    status=status
                                    raw_json=raw_json
                                    mode=mode
                                    pending_zone_parent=pending_zone_parent
                                />
                            </section>

                            <section
                                class="card floating-panel left-panel-card"
                                class:is-hidden=move || active_left_panel.get() != Some(LeftPanel::Graph)
                            >
                                <div class="panel-head">
                                    <div>
                                        <p class="section">"Graph"</p>
                                        <h3>"Nodes & Edges"</h3>
                                    </div>
                                </div>
                                <div class="mini-stats multi">
                                    <div class="mini-stat mini-stat-action">
                                        <div class="mini-stat-top">
                                            <span>"Nodes"</span>
                                            <button
                                                class="icon-button"
                                                on:click=move |_| {
                                                    let next = if mode.get() == Mode::AddNode {
                                                        Mode::Inspect
                                                    } else {
                                                        Mode::AddNode
                                                    };
                                                    mode.set(next);
                                                    status.set(if next == Mode::AddNode {
                                                        "Click the map to place nodes.".into()
                                                    } else {
                                                        "Inspect mode.".into()
                                                    });
                                                }
                                                type="button"
                                                title="Add node"
                                            >
                                                "+"
                                            </button>
                                        </div>
                                        <strong>{move || workspace.get().nodes.len().to_string()}</strong>
                                    </div>
                                    <div class="mini-stat mini-stat-action">
                                        <div class="mini-stat-top">
                                            <span>"Edges"</span>
                                            <button
                                                class="icon-button"
                                                on:click=move |_| {
                                                    edge_source_id.set(String::new());
                                                    let next = if mode.get() == Mode::ConnectEdge {
                                                        Mode::Inspect
                                                    } else {
                                                        Mode::ConnectEdge
                                                    };
                                                    mode.set(next);
                                                    status.set(if next == Mode::ConnectEdge {
                                                        "Click one node, then another.".into()
                                                    } else {
                                                        "Inspect mode.".into()
                                                    });
                                                }
                                                type="button"
                                                title="Add edge"
                                            >
                                                "+"
                                            </button>
                                        </div>
                                        <strong>{move || workspace.get().edges.len().to_string()}</strong>
                                    </div>
                                </div>
                                <div class="list" class:empty=move || workspace.get().nodes.is_empty() && workspace.get().edges.is_empty()>
                                    <Show
                                        when=move || !workspace.get().nodes.is_empty() || !workspace.get().edges.is_empty()
                                        fallback=move || view! { <p class="empty-copy">"No graph items yet."</p> }
                                    >
                                        <For each=graph_nodes key=|node| node.id.clone() let:node>
                                            <div class="list-row-wrap" style="--depth:0" data-depth="0">
                                                <button
                                                    class="list-row"
                                                    class:is-selected={
                                                        let selected_id = node.id.clone();
                                                        move || matches!(selection.get(), Selection::Node(ref id) if id == &selected_id)
                                                    }
                                                    on:click={
                                                        let node_id = node.id.clone();
                                                        move |_| {
                                                            if mode.get() == Mode::ConnectEdge {
                                                                let source_id = edge_source_id.get();
                                                                if source_id.is_empty() {
                                                                    edge_source_id.set(node_id.clone());
                                                                    status.set("Selected edge source. Pick a target node.".into());
                                                                } else if source_id != node_id {
                                                                    let edge_id = new_id();
                                                                    workspace.update(|ws| {
                                                                        ws.edges.insert(edge_id.clone(), EdgeJson {
                                                                            id: edge_id.clone(),
                                                                            source_id: source_id.clone(),
                                                                            target_id: node_id.clone(),
                                                                            directed: true,
                                                                            weight: 1.0,
                                                                            zone_ids: Vec::new(),
                                                                            properties: BTreeMap::new(),
                                                                        });
                                                                    });
                                                                    selection.set(Selection::Edge(edge_id));
                                                                    edge_source_id.set(String::new());
                                                                    update_workspace("Created edge.", true);
                                                                }
                                                            } else {
                                                                let already_selected =
                                                                    matches!(selection.get(), Selection::Node(ref id) if id == &node_id);
                                                                selection.set(Selection::Node(node_id.clone()));
                                                                active_right_panel.set(Some(RightPanel::Details));
                                                                if already_selected {
                                                                    let ws = workspace.get();
                                                                    MAP_HANDLE.with(|cell| {
                                                                        if let Some(map) = cell.borrow().as_ref() {
                                                                            if let Some(node) = ws.nodes.get(&node_id) {
                                                                                let _ = focus_map_node(map, node);
                                                                            }
                                                                        }
                                                                    });
                                                                } else {
                                                                    status.set("Node selected.".into());
                                                                }
                                                            }
                                                        }
                                                    }
                                                    on:dblclick={
                                                        let node_id = node.id.clone();
                                                        move |_| {
                                                            selection.set(Selection::Node(node_id.clone()));
                                                            active_right_panel.set(Some(RightPanel::Details));
                                                            let ws = workspace.get();
                                                            MAP_HANDLE.with(|cell| {
                                                                if let Some(map) = cell.borrow().as_ref() {
                                                                    if let Some(node) = ws.nodes.get(&node_id) {
                                                                        let _ = focus_map_node(map, node);
                                                                    }
                                                                }
                                                            });
                                                        }
                                                    }
                                                >
                                                    <div class="row-main">
                                                        <p class="row-title">{node.name.clone()}</p>
                                                        <p class="row-meta">{format!("{} · {} zones", point_text(&node.latlon), node.zone_ids.len())}</p>
                                                    </div>
                                                    <span
                                                        class="row-tail-action row-tail-danger"
                                                        role="button"
                                                        tabindex="0"
                                                        on:click={let id = node.id.clone(); move |ev| { ev.stop_propagation(); ev.prevent_default(); remove_node(id.clone()) }}
                                                    >
                                                        <span class="row-tail-glyph" aria-hidden="true">"x"</span>
                                                    </span>
                                                </button>
                                            </div>
                                        </For>
                                        <For each=graph_edges key=|edge| edge.id.clone() let:edge>
                                            <div class="list-row-wrap" style="--depth:0" data-depth="0">
                                                <button
                                                    class="list-row"
                                                    class:is-selected={
                                                        let selected_id = edge.id.clone();
                                                        move || matches!(selection.get(), Selection::Edge(ref id) if id == &selected_id)
                                                    }
                                                    on:click={let edge_id = edge.id.clone(); move |_| {
                                                        let already_selected =
                                                            matches!(selection.get(), Selection::Edge(ref id) if id == &edge_id);
                                                        selection.set(Selection::Edge(edge_id.clone()));
                                                        active_right_panel.set(Some(RightPanel::Details));
                                                        if already_selected {
                                                            let ws = workspace.get();
                                                            MAP_HANDLE.with(|cell| {
                                                                if let Some(map) = cell.borrow().as_ref() {
                                                                    if let Some(edge) = ws.edges.get(&edge_id) {
                                                                        let _ = focus_map_edge(map, &ws, edge);
                                                                    }
                                                                }
                                                            });
                                                        } else {
                                                            status.set("Edge selected.".into());
                                                        }
                                                    }}
                                                    on:dblclick={let edge_id = edge.id.clone(); move |_| {
                                                        selection.set(Selection::Edge(edge_id.clone()));
                                                        active_right_panel.set(Some(RightPanel::Details));
                                                        let ws = workspace.get();
                                                        MAP_HANDLE.with(|cell| {
                                                            if let Some(map) = cell.borrow().as_ref() {
                                                                if let Some(edge) = ws.edges.get(&edge_id) {
                                                                    let _ = focus_map_edge(map, &ws, edge);
                                                                }
                                                            }
                                                        });
                                                    }}
                                                >
                                                    <div class="row-main">
                                                        <p class="row-title">{move || if edge.directed { "Directed edge" } else { "Edge" }}</p>
                                                        <p class="row-meta">{format!("{} → {}", node_summary(&workspace.get(), &edge.source_id), node_summary(&workspace.get(), &edge.target_id))}</p>
                                                    </div>
                                                    <span
                                                        class="row-tail-action row-tail-danger"
                                                        role="button"
                                                        tabindex="0"
                                                        on:click={let id = edge.id.clone(); move |ev| { ev.stop_propagation(); ev.prevent_default(); remove_edge(id.clone()) }}
                                                    >
                                                        <span class="row-tail-glyph" aria-hidden="true">"x"</span>
                                                    </span>
                                                </button>
                                            </div>
                                        </For>
                                    </Show>
                                </div>
                            </section>
                            </Show>

                            <Show when=move || app_mode.get() == AppMode::Management>
                                <section
                                    class="card floating-panel left-panel-card"
                                    class:is-hidden=move || management_left_panel.get() != Some(LeftPanel::Zenoh)
                                >
                                    <div class="panel-head panel-head-stack">
                                        <div>
                                            <p class="section">"Zenoh"</p>
                                            <h3>"Connection"</h3>
                                        </div>
                                        <div
                                            class="connection-pill"
                                            class:is-connected=move || zenoh_state.get() == ZenohConnectionState::Connected
                                            class:is-pending=move || zenoh_state.get() == ZenohConnectionState::Connecting
                                        >
                                            {move || {
                                                match zenoh_state.get() {
                                                    ZenohConnectionState::Disconnected => "Disconnected",
                                                    ZenohConnectionState::Connecting => "Connecting",
                                                    ZenohConnectionState::Connected => "Connected",
                                                    ZenohConnectionState::Error => "Error",
                                                }
                                            }}
                                        </div>
                                    </div>
                                    <div class="field-group">
                                        <label class="field">
                                            <span>"Connection"</span>
                                            <input
                                                placeholder="localhost:7447"
                                                prop:value=move || zenoh_endpoint.get()
                                                on:input=move |ev| zenoh_endpoint.set(event_target_value(&ev))
                                            />
                                        </label>
                                        <label class="field">
                                            <span>"Connection Type"</span>
                                            <select
                                                prop:value=move || zenoh_connection_type.get().as_str()
                                                on:change=move |ev| {
                                                    zenoh_connection_type.set(ZenohConnectionType::from_value(&event_target_value(&ev)));
                                                }
                                            >
                                                <option value="ws">"WebSocket"</option>
                                                <option value="tcp">"TCP"</option>
                                                <option value="udp">"UDP"</option>
                                                <option value="quic">"QUIC"</option>
                                            </select>
                                        </label>
                                    </div>
                                    <div class="panel-card">
                                        <div class="panel-head compact-head">
                                            <div>
                                                <p class="section">"Status"</p>
                                                <h3>"Session"</h3>
                                            </div>
                                        </div>
                                        <p class="hint">
                                            {move || zenoh_status.get()}
                                        </p>
                                        <p class="hint">
                                            {move || {
                                                format!(
                                                    "The Rust backend owns the Zenoh session and reports connection state back to this tab over the local API."
                                                )
                                            }}
                                        </p>
                                    </div>
                                    <div class="dock-actions">
                                        <button
                                            class="primary"
                                            on:click=connect_zenoh
                                            type="button"
                                        >
                                            "Connect"
                                        </button>
                                        <button
                                            class="ghost"
                                            on:click=disconnect_zenoh
                                            type="button"
                                        >
                                            "Disconnect"
                                        </button>
                                    </div>
                                </section>

                                <section
                                    class="card floating-panel left-panel-card"
                                    class:is-hidden=move || management_left_panel.get() != Some(LeftPanel::Robots)
                                >
                                    <div class="panel-head">
                                        <div>
                                            <p class="section">"Fleet"</p>
                                            <h3>"Robots"</h3>
                                        </div>
                                    </div>
                                    <div class="list" class:empty=move || robots.get().is_empty()>
                                        <Show
                                            when=move || !robots.get().is_empty()
                                            fallback=move || view! {
                                                <p class="empty-copy">"No robot odom or gnss topics detected yet."</p>
                                            }
                                        >
                                            <For
                                                each=move || visible_robots(&robots.get(), robot_clock_ms.get())
                                                key=|robot| robot.name.clone()
                                                let:robot
                                            >
                                                <RobotFleetRow
                                                    robot=robot
                                                    workspace=workspace
                                                    selected_robot_name=selected_robot_name
                                                    schedule_selected_task=schedule_selected_task
                                                    scheduler_task_options=scheduler_task_options
                                                    scheduler_selected_task=scheduler_selected_task
                                                    scheduler_target_node_id=scheduler_target_node_id
                                                    management_right_panel=management_right_panel
                                                    status=status
                                                    robot_clock_ms=robot_clock_ms
                                                />
                                            </For>
                                        </Show>
                                    </div>
                                </section>
                            </Show>
                        </div>

                        <div class="floating top-mode-switch">
                            <button
                                class="panel-toggle"
                                class:is-active=move || app_mode.get() == AppMode::Editor
                                on:click=switch_to_editor
                                type="button"
                                title="Editor"
                            >
                                "E"
                            </button>
                            <button
                                class="panel-toggle"
                                class:is-active=move || app_mode.get() == AppMode::Management
                                on:click=switch_to_management
                                type="button"
                                title="Management"
                            >
                                "M"
                            </button>
                        </div>

                        <div class="floating right-panel-shell">
                            <div class="right-stack">
                                <Show
                                    when=move || app_mode.get() == AppMode::Editor
                                fallback=move || view! {
                                        <button
                                            class="panel-toggle"
                                            class:is-active=move || management_right_panel.get() == Some(RightPanel::Details)
                                            on:click=move |_| toggle_management_right_panel(RightPanel::Details)
                                            type="button"
                                            title="Toggle inspector panel"
                                        >
                                            "I"
                                        </button>
                                        <button
                                            class="panel-toggle"
                                            class:is-active=move || management_right_panel.get() == Some(RightPanel::Scheduler)
                                            on:click=open_scheduler_panel
                                            type="button"
                                            title="Toggle order panel"
                                        >
                                            "O"
                                        </button>
                                        <Show when=move || robot_has_available_tasks(&robots.get(), &selected_robot_name.get())>
                                            <button
                                                class="panel-toggle"
                                                class:is-active=move || management_right_panel.get() == Some(RightPanel::Tasks)
                                                on:click=open_tasks_panel
                                                type="button"
                                                title="Toggle tasks panel"
                                            >
                                                "T"
                                            </button>
                                        </Show>
                                    }
                                >
                                    <button
                                        class="panel-toggle"
                                        class:is-active=move || active_right_panel.get() == Some(RightPanel::Details)
                                        on:click=move |_| toggle_right_panel(RightPanel::Details)
                                        type="button"
                                        title="Toggle inspector panel"
                                    >
                                        "I"
                                    </button>
                                    <button
                                        class="panel-toggle"
                                        class:is-active=move || active_right_panel.get() == Some(RightPanel::Json)
                                        on:click=move |_| toggle_right_panel(RightPanel::Json)
                                        type="button"
                                        title="Toggle raw json panel"
                                    >
                                        "J"
                                    </button>
                                    <button
                                        class="panel-toggle"
                                        class:is-active=move || active_right_panel.get() == Some(RightPanel::Files)
                                        on:click=move |_| toggle_right_panel(RightPanel::Files)
                                        type="button"
                                        title="Toggle files panel"
                                    >
                                        "F"
                                    </button>
                                </Show>
                            </div>
                            <div class="right-panels">
                                <Show when=move || app_mode.get() == AppMode::Editor>
                                <section
                                    class="card floating-panel right-panel-card"
                                    class:is-hidden=move || active_right_panel.get() != Some(RightPanel::Details)
                                >
                                    <div class="panel-head">
                                        <div>
                                            <p class="section">"Inspector"</p>
                                            <h3>"Selection"</h3>
                                        </div>
                                    </div>
                                    <Inspector workspace=workspace selection=selection raw_json=raw_json status=status />
                                </section>

                                <section
                                    class="card floating-panel right-panel-card"
                                    class:is-hidden=move || active_right_panel.get() != Some(RightPanel::Files)
                                >
                                    <div class="panel-head panel-head-stack">
                                        <div>
                                            <p class="section">"Workspace"</p>
                                            <h3>{move || workspace.get().name}</h3>
                                        </div>
                                        <p class="hint">{move || status.get()}</p>
                                    </div>
                                    <div class="field-group">
                                        <label class="field">
                                            <span>"Name"</span>
                                            <input
                                                prop:value=move || workspace.get().name
                                                on:input=move |ev| {
                                                    let value = event_target_value(&ev);
                                                    workspace.update(|ws| ws.name = value);
                                                    sync_json();
                                                }
                                            />
                                        </label>
                                        <label class="field">
                                            <span>"Root Zone ID"</span>
                                            <input
                                                prop:value=move || workspace.get().root_zone_id
                                                on:input=move |ev| {
                                                    let value = event_target_value(&ev);
                                                    workspace.update(|ws| ws.root_zone_id = value);
                                                    update_workspace("Updated root zone id.", false);
                                                }
                                            />
                                        </label>
                                        <div class="field-grid">
                                            <label class="field">
                                                <span>"Datum Lat"</span>
                                                <input
                                                    prop:value=move || workspace.get().datum.as_ref().map(|p| p.lat.to_string()).unwrap_or_default()
                                                    on:input=move |ev| {
                                                        let value = event_target_value(&ev);
                                                        workspace.update(|ws| {
                                                            let lat = value.parse::<f64>().ok();
                                                            let current = ws.datum.clone().unwrap_or(JsonPoint { lat: 52.0, lon: 5.0 });
                                                            ws.datum = lat.map(|lat| JsonPoint { lat, lon: current.lon });
                                                        });
                                                        update_workspace("Updated datum latitude.", false);
                                                    }
                                                />
                                            </label>
                                            <label class="field">
                                                <span>"Datum Lon"</span>
                                                <input
                                                    prop:value=move || workspace.get().datum.as_ref().map(|p| p.lon.to_string()).unwrap_or_default()
                                                    on:input=move |ev| {
                                                        let value = event_target_value(&ev);
                                                        workspace.update(|ws| {
                                                            let lon = value.parse::<f64>().ok();
                                                            let current = ws.datum.clone().unwrap_or(JsonPoint { lat: 52.0, lon: 5.0 });
                                                            ws.datum = lon.map(|lon| JsonPoint { lat: current.lat, lon });
                                                        });
                                                        update_workspace("Updated datum longitude.", false);
                                                    }
                                                />
                                            </label>
                                        </div>
                                    </div>
                                    <div class="panel-card">
                                        <div class="panel-head">
                                            <div>
                                                <p class="section">"Health"</p>
                                                <h3>"Validation"</h3>
                                            </div>
                                            <p class="hint">{move || if validation().is_empty() { "Healthy".into() } else { format!("{} issues", validation().len()) }}</p>
                                        </div>
                                        <div class="validation-list" class:is-clean=move || validation().is_empty()>
                                            <Show
                                                when=move || !validation().is_empty()
                                                fallback=move || view! { <div class="validation-item validation-ok">"Workspace structure is currently valid enough for export."</div> }
                                            >
                                                <For each=validation key=|msg| msg.clone() let:msg>
                                                    <div class="validation-item">{msg}</div>
                                                </For>
                                            </Show>
                                        </div>
                                    </div>
                                    <div class="dock-actions">
                                        <button class="ghost" on:click=new_workspace type="button">"New"</button>
                                        <button class="ghost" on:click=open_workspace type="button">"Open"</button>
                                        <button class="primary" on:click=save_workspace type="button">"Save"</button>
                                        <button class="ghost" on:click=locate_me_action type="button">"Locate Me"</button>
                                    </div>
                                </section>

                                <section
                                    class="card floating-panel right-panel-card"
                                    class:is-hidden=move || active_right_panel.get() != Some(RightPanel::Json)
                                >
                                    <div class="panel-head">
                                        <div>
                                            <p class="section">"Reference"</p>
                                            <h3>"Raw JSON"</h3>
                                        </div>
                                        <div class="dock-actions">
                                            <button class="ghost" on:click=format_json>"Format"</button>
                                            <button class="ghost" on:click=apply_json>"Apply"</button>
                                        </div>
                                    </div>
                                    <textarea
                                        id="raw-json"
                                        spellcheck="false"
                                        prop:value=move || raw_json.get()
                                        on:input=move |ev| raw_json.set(event_target_value(&ev))
                                    />
                                </section>
                                </Show>

                                <Show when=move || app_mode.get() == AppMode::Management>
                                    <section
                                        class="card floating-panel right-panel-card"
                                        class:is-hidden=move || management_right_panel.get() != Some(RightPanel::Scheduler)
                                    >
                                        <div class="panel-head panel-head-stack">
                                            <div>
                                                <p class="section">"Order"</p>
                                                <h3>{move || {
                                                    let robot = selected_robot_name.get();
                                                    if robot.is_empty() {
                                                        "No robot selected".to_string()
                                                    } else {
                                                        robot
                                                    }
                                                }}</h3>
                                            </div>
                                            <p class="hint">
                                                {move || {
                                                    if scheduler_picking_node.get() {
                                                        "Click a node on the map or in the list.".to_string()
                                                    } else if selected_robot_name.get().is_empty() {
                                                        "No tasks available.".to_string()
                                                    } else {
                                                        "Select an order and node.".to_string()
                                                    }
                                                }}
                                            </p>
                                        </div>
                                        <div class="field-group">
                                            <div class="panel-card">
                                                <div class="panel-head compact-head">
                                                    <div>
                                                        <p class="section">"Order"</p>
                                                        <h3>{move || {
                                                            let selected = schedule_selected_task.get();
                                                            if selected.is_empty() {
                                                                "No tasks available".to_string()
                                                            } else {
                                                                selected
                                                            }
                                                        }}</h3>
                                                    </div>
                                                </div>
                                                <div class="dock-actions">
                                                    <button
                                                        class="ghost"
                                                        class:is-armed=move || schedule_selected_task.get() == ORDER_OPTIONS[0]
                                                        disabled=move || selected_robot_name.get().is_empty()
                                                        on:click=move |_| schedule_selected_task.set(ORDER_OPTIONS[0].into())
                                                        type="button"
                                                    >
                                                        "goto"
                                                    </button>
                                                    <button
                                                        class="ghost"
                                                        class:is-armed=move || schedule_selected_task.get() == ORDER_OPTIONS[1]
                                                        disabled=move || selected_robot_name.get().is_empty()
                                                        on:click=move |_| schedule_selected_task.set(ORDER_OPTIONS[1].into())
                                                        type="button"
                                                    >
                                                        "patrol"
                                                    </button>
                                                </div>
                                            </div>
                                            <div class="panel-card">
                                                <div class="panel-head compact-head">
                                                    <div>
                                                        <p class="section">"Target"</p>
                                                        <h3>{move || {
                                                            let node_id = scheduler_target_node_id.get();
                                                            if node_id.is_empty() {
                                                                "No node selected".to_string()
                                                            } else {
                                                                workspace
                                                                    .get()
                                                                    .nodes
                                                                    .get(&node_id)
                                                                    .map(|node| node.name.clone())
                                                                    .unwrap_or(node_id)
                                                            }
                                                        }}</h3>
                                                    </div>
                                                </div>
                                                <p class="hint">
                                                    {move || {
                                                        let node_id = scheduler_target_node_id.get();
                                                        if node_id.is_empty() {
                                                            "Arm node picking, then click one node.".to_string()
                                                        } else {
                                                            workspace
                                                                .get()
                                                                .nodes
                                                                .get(&node_id)
                                                                .map(|node| point_text(&node.latlon))
                                                                .unwrap_or_else(|| "Node no longer exists.".to_string())
                                                        }
                                                    }}
                                                </p>
                                            </div>
                                        </div>
                                        <div class="dock-actions">
                                            <button
                                                class="ghost"
                                                class:is-armed=move || scheduler_picking_node.get()
                                                disabled=move || selected_robot_name.get().is_empty()
                                                on:click=arm_scheduler_node_pick
                                                type="button"
                                            >
                                                {move || if scheduler_picking_node.get() { "Cancel Node Pick" } else { "Select Node" }}
                                            </button>
                                            <button
                                                class="primary"
                                                disabled=move || {
                                                    selected_robot_name.get().is_empty()
                                                        || schedule_selected_task.get().is_empty()
                                                        || scheduler_target_node_id.get().is_empty()
                                                }
                                                on:click=run_schedule_task
                                                type="button"
                                            >
                                                "Run"
                                            </button>
                                        </div>
                                    </section>

                                    <section
                                        class="card floating-panel right-panel-card"
                                        class:is-hidden=move || management_right_panel.get() != Some(RightPanel::Tasks)
                                    >
                                        <div class="panel-head panel-head-stack">
                                            <div>
                                                <p class="section">"Tasks"</p>
                                                <h3>{move || {
                                                    let robot = selected_robot_name.get();
                                                    if robot.is_empty() {
                                                        "No robot selected".to_string()
                                                    } else {
                                                        robot
                                                    }
                                                }}</h3>
                                            </div>
                                            <p class="hint">
                                                {move || {
                                                    if scheduler_picking_node.get() {
                                                        "Click a node on the map or in the list.".to_string()
                                                    } else if selected_robot_name.get().is_empty() {
                                                        "No tasks available.".to_string()
                                                    } else {
                                                        "Select a robot, task, and node.".to_string()
                                                    }
                                                }}
                                            </p>
                                        </div>
                                        <div class="field-group">
                                            <label class="field">
                                                <span>"Task"</span>
                                                <select
                                                    prop:value=move || scheduler_selected_task.get()
                                                    disabled=move || selected_robot_name.get().is_empty()
                                                    on:change=move |ev| scheduler_selected_task.set(event_target_value(&ev))
                                                >
                                                    <Show
                                                        when=move || !scheduler_task_options.get().is_empty()
                                                        fallback=move || view! {
                                                            <option value="">"No tasks available"</option>
                                                        }
                                                    >
                                                        <option value="">"Select task"</option>
                                                        <For
                                                            each=move || scheduler_task_options.get()
                                                            key=|task| task.clone()
                                                            let:task
                                                        >
                                                            <option value=task.clone()>{task.clone()}</option>
                                                        </For>
                                                    </Show>
                                                </select>
                                            </label>
                                            <div class="panel-card">
                                                <div class="panel-head compact-head">
                                                    <div>
                                                        <p class="section">"Target"</p>
                                                        <h3>{move || {
                                                            let node_id = scheduler_target_node_id.get();
                                                            if node_id.is_empty() {
                                                                "No node selected".to_string()
                                                            } else {
                                                                workspace
                                                                    .get()
                                                                    .nodes
                                                                    .get(&node_id)
                                                                    .map(|node| node.name.clone())
                                                                    .unwrap_or(node_id)
                                                            }
                                                        }}</h3>
                                                    </div>
                                                </div>
                                                <p class="hint">
                                                    {move || {
                                                        let node_id = scheduler_target_node_id.get();
                                                        if node_id.is_empty() {
                                                            "Arm node picking, then click one node.".to_string()
                                                        } else {
                                                            workspace
                                                                .get()
                                                                .nodes
                                                                .get(&node_id)
                                                                .map(|node| point_text(&node.latlon))
                                                                .unwrap_or_else(|| "Node no longer exists.".to_string())
                                                        }
                                                    }}
                                                </p>
                                            </div>
                                        </div>
                                        <div class="dock-actions">
                                            <button
                                                class="ghost"
                                                class:is-armed=move || scheduler_picking_node.get()
                                                disabled=move || selected_robot_name.get().is_empty()
                                                on:click=arm_scheduler_node_pick
                                                type="button"
                                            >
                                                {move || if scheduler_picking_node.get() { "Cancel Node Pick" } else { "Select Node" }}
                                            </button>
                                            <button
                                                class="primary"
                                                disabled=move || {
                                                    selected_robot_name.get().is_empty()
                                                        || scheduler_selected_task.get().is_empty()
                                                        || scheduler_target_node_id.get().is_empty()
                                                }
                                                on:click=run_scheduler_task
                                                type="button"
                                            >
                                                "Run"
                                            </button>
                                        </div>
                                    </section>

                                    <section
                                        class="card floating-panel right-panel-card"
                                        class:is-hidden=move || management_right_panel.get() != Some(RightPanel::Details)
                                    >
                                        <div class="panel-head">
                                            <div>
                                                <p class="section">"Inspector"</p>
                                                <h3>"Fleet"</h3>
                                            </div>
                                        </div>
                                        <div class="inspector-stack">
                                            <p class="hint">"Fleet management is not implemented yet."</p>
                                        </div>
                                    </section>
                                </Show>
                            </div>
                        </div>
                    </div>
                </section>
            </main>
        </div>
    }
}

#[component]
fn ZoneTree(
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    mode: RwSignal<Mode>,
    pending_zone_parent: RwSignal<Option<String>>,
) -> impl IntoView {
    move || {
        let ws = workspace.get();
        if ws.zones.is_empty() {
            return view! {
                <div class="list empty">
                    <button
                        class="empty-action"
                        on:click=move |_| {
                            pending_zone_parent.set(None);
                            mode.set(Mode::PlaceZone);
                            status.set("Click the map to add zone points. Right-click to finish the polygon.".into());
                        }
                        type="button"
                    >
                        "Add Root Zone"
                    </button>
                </div>
            }
            .into_any();
        }

        let roots = root_zone_ids(&ws);
        view! {
            <div class="list">
                <ul class="tree-list root-tree">
                    <For each=move || roots.clone() key=|id| id.clone() let:zone_id>
                        <ZoneTreeNode
                            workspace=workspace
                            selection=selection
                            status=status
                            raw_json=raw_json
                            mode=mode
                            pending_zone_parent=pending_zone_parent
                            zone_id=zone_id
                        />
                    </For>
                </ul>
            </div>
        }
        .into_any()
    }
}

#[component]
fn ZoneTreeNode(
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    mode: RwSignal<Mode>,
    pending_zone_parent: RwSignal<Option<String>>,
    zone_id: String,
) -> impl IntoView {
    move || {
        let ws = workspace.get();
        let Some(zone) = ws.zones.get(&zone_id).cloned() else {
            return view! { <></> }.into_any();
        };
        let child_ids = sorted_child_zone_ids(&ws, &zone_id);
        let child_ids_for_show = child_ids.clone();
        let depth = zone_depth(&ws, &zone_id);
        let selected = matches!(selection.get(), Selection::Zone(ref id) if id == &zone.id);
        let role = if zone.id == ws.root_zone_id {
            "root".to_string()
        } else {
            zone.zone_type.clone()
        };

        let sync_json = move || {
            raw_json.set(serde_json::to_string_pretty(&workspace.get()).unwrap_or_else(|_| "{}".into()));
        };

        view! {
            <li class="tree-node" data-depth=depth.to_string()>
                <div class="list-row-wrap">
                    <button
                        class="list-row"
                        class:is-selected=selected
                        on:click={
                            let zone_id = zone.id.clone();
                            move |_| {
                                let already_selected =
                                    matches!(selection.get(), Selection::Zone(ref id) if id == &zone_id);
                                selection.set(Selection::Zone(zone_id.clone()));
                                if already_selected {
                                    let ws = workspace.get();
                                    MAP_HANDLE.with(|cell| {
                                        if let Some(map) = cell.borrow().as_ref() {
                                            if let Some(zone) = ws.zones.get(&zone_id) {
                                                let _ = focus_map_zone(map, zone);
                                            }
                                        }
                                    });
                                }
                            }
                        }
                        on:dblclick={
                            let zone_id = zone.id.clone();
                            move |_| {
                                selection.set(Selection::Zone(zone_id.clone()));
                                let ws = workspace.get();
                                MAP_HANDLE.with(|cell| {
                                    if let Some(map) = cell.borrow().as_ref() {
                                        if let Some(zone) = ws.zones.get(&zone_id) {
                                            let _ = focus_map_zone(map, zone);
                                        }
                                    }
                                });
                            }
                        }
                        type="button"
                    >
                        <div class="row-main">
                            <p class="row-title">{zone.name.clone()}</p>
                            <p class="row-meta">{format!("{} · {} pts", role, zone.polygon_latlon.len())}</p>
                        </div>
                        <span class="row-tail-stack">
                            <span
                                class="row-tail-action"
                                role="button"
                                tabindex="0"
                                class:is-armed={
                                    let armed_parent = zone.id.clone();
                                    move || mode.get() == Mode::PlaceZone && pending_zone_parent.get() == Some(armed_parent.clone())
                                }
                                on:click={
                                    let parent_id = zone.id.clone();
                                    move |ev| {
                                        ev.stop_propagation();
                                        ev.prevent_default();
                                        pending_zone_parent.set(Some(parent_id.clone()));
                                        mode.set(Mode::PlaceZone);
                                        status.set("Click the map to add child-zone points. Right-click to finish the polygon.".into());
                                    }
                                }
                            >
                                <span class="row-tail-glyph" aria-hidden="true">"+"</span>
                            </span>
                            <span
                                class="row-tail-action row-tail-danger"
                                role="button"
                                tabindex="0"
                                on:click={
                                    let remove_id = zone.id.clone();
                                    move |ev| {
                                        ev.stop_propagation();
                                        ev.prevent_default();
                                        workspace.update(|ws| {
                                            let Some(zone) = ws.zones.get(&remove_id).cloned() else {
                                                return;
                                            };

                                            for child_id in &zone.child_ids {
                                                if let Some(child) = ws.zones.get_mut(child_id) {
                                                    child.parent_id.clear();
                                                }
                                            }

                                            if !zone.parent_id.is_empty() {
                                                if let Some(parent) = ws.zones.get_mut(&zone.parent_id) {
                                                    parent.child_ids.retain(|id| id != &remove_id);
                                                }
                                            }

                                            ws.zones.remove(&remove_id);

                                            if ws.root_zone_id == remove_id {
                                                let next_root = root_zone_ids(ws).into_iter().next().unwrap_or_default();
                                                ws.root_zone_id = next_root.clone();
                                                if let Some(root) = ws.zones.get_mut(&next_root) {
                                                    root.parent_id.clear();
                                                }
                                            }
                                        });
                                        selection.set(Selection::Workspace);
                                        sync_associations(&workspace);
                                        sync_json();
                                        status.set("Removed zone.".into());
                                    }
                                }
                            >
                                <span class="row-tail-glyph" aria-hidden="true">"x"</span>
                            </span>
                        </span>
                    </button>
                </div>
                <Show
                    when=move || !child_ids_for_show.is_empty()
                    fallback=move || view! { <></> }
                >
                    <ul class="tree-list">
                        <For each={
                            let child_ids = child_ids.clone();
                            move || child_ids.clone()
                        } key=|id| id.clone() let:child_id>
                            <ZoneTreeNode
                                workspace=workspace
                                selection=selection
                                status=status
                                raw_json=raw_json
                                mode=mode
                                pending_zone_parent=pending_zone_parent
                                zone_id=child_id
                            />
                        </For>
                    </ul>
                </Show>
            </li>
        }
        .into_any()
    }
}

#[component]
fn RobotFleetRow(
    robot: RobotSummary,
    workspace: RwSignal<WorkspaceJson>,
    selected_robot_name: RwSignal<String>,
    schedule_selected_task: RwSignal<String>,
    scheduler_task_options: RwSignal<Vec<String>>,
    scheduler_selected_task: RwSignal<String>,
    scheduler_target_node_id: RwSignal<String>,
    management_right_panel: RwSignal<Option<RightPanel>>,
    status: RwSignal<String>,
    robot_clock_ms: RwSignal<u64>,
) -> impl IntoView {
    let robot_name = robot.name.clone();
    let selected_robot_id = robot.name.clone();
    let row_meta = {
        let mut parts = Vec::new();
        if !robot.odom_key.is_empty() {
            parts.push(format!("odom: {}", robot.odom_key));
        }
        if !robot.gnss_keys.is_empty() {
            parts.push(format!("gnss: {}", robot.gnss_keys.join(", ")));
        }
        if parts.is_empty() {
            "No odom or gnss keys yet.".to_string()
        } else {
            parts.join(" | ")
        }
    };
    let robot_color = robot.color.clone();

    let focus_robot = {
        let robot = robot.clone();
        move |_| {
            selected_robot_name.set(robot.name.clone());
            schedule_selected_task.set(String::new());
            scheduler_selected_task.set(String::new());
            scheduler_target_node_id.set(String::new());
            let ws = workspace.get();
            if let Some(point) = robot_point(&ws, &robot) {
                MAP_HANDLE.with(|cell| {
                    if let Some(map) = cell.borrow().as_ref() {
                        let _ = focus_map_point(map, &point, 18.0);
                    }
                });
                status.set(format!("Selected robot {}.", robot.name));
            } else {
                status.set(format!(
                    "Selected robot {}. It has no mappable odom or gnss position yet.",
                    robot.name
                ));
            }
            spawn_local(refresh_scheduler_tasks(
                robot.name.clone(),
                scheduler_task_options,
                scheduler_selected_task,
                status,
            ));
            if management_right_panel.get_untracked() == Some(RightPanel::Scheduler) {
                scheduler_target_node_id.set(String::new());
            }
        }
    };

    view! {
        <button
            class="list-row"
            class:is-stale=move || is_robot_stale(&robot, robot_clock_ms.get())
            class:is-selected=move || selected_robot_name.get() == selected_robot_id
            type="button"
            on:click=focus_robot
        >
            <div class="row-main">
                <p class="row-title">{robot_name}</p>
                <p class="row-meta">{row_meta}</p>
            </div>
            <span
                class="row-tail-action row-tail-color"
                style:background-color=robot_color
                title="Robot color"
            ></span>
        </button>
    }
}

#[component]
fn Inspector(
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    raw_json: RwSignal<String>,
    status: RwSignal<String>,
) -> impl IntoView {
    let sync_json = move || {
        raw_json.set(serde_json::to_string_pretty(&workspace.get()).unwrap_or_else(|_| "{}".into()));
    };

    move || match selection.get() {
        Selection::Workspace => view! {
            <p class="hint">"Nothing selected. Click a zone or node, or edit the workspace name."</p>
        }
        .into_any(),
        Selection::Zone(zone_id) => {
            let zone = workspace.get().zones.get(&zone_id).cloned();
            match zone {
                Some(zone) => {
                    let zone_id_name = zone_id.clone();
                    let zone_id_type = zone_id.clone();
                    let zone_id_grid = zone_id.clone();
                    let zone_id_res = zone_id.clone();
                    let zone_id_policy = zone_id.clone();
                    let zone_id_capacity = zone_id.clone();
                    let zone_id_priority = zone_id.clone();
                    let zone_id_speed_limit = zone_id.clone();
                    let zone_id_claim_required = zone_id.clone();
                    let zone_id_blocked = zone_id.clone();
                    let zone_id_poly_value = zone_id.clone();
                    let zone_id_poly_change = zone_id.clone();
                    let zone_id_props = zone_id.clone();
                    let zone_id_nodes = zone_id.clone();
                    view! {
                    <div class="inspector-stack">
                        <div class="inspector-header">
                            <div class="inspector-header-copy">
                                <p class="section">"Selection"</p>
                                <h3>{zone.name.clone()}</h3>
                                <p class="inspector-id">{zone.id.clone()}</p>
                            </div>
                            <button class="ghost danger-button" on:click={
                                let remove_id = zone.id.clone();
                                move |_| {
                                    workspace.update(|ws| {
                                        let Some(zone) = ws.zones.get(&remove_id).cloned() else {
                                            return;
                                        };
                                        for child_id in &zone.child_ids {
                                            if let Some(child) = ws.zones.get_mut(child_id) {
                                                child.parent_id.clear();
                                            }
                                        }
                                        if !zone.parent_id.is_empty() {
                                            if let Some(parent) = ws.zones.get_mut(&zone.parent_id) {
                                                parent.child_ids.retain(|id| id != &remove_id);
                                            }
                                        }
                                        ws.zones.remove(&remove_id);
                                        if ws.root_zone_id == remove_id {
                                            let next_root = root_zone_ids(ws).into_iter().next().unwrap_or_default();
                                            ws.root_zone_id = next_root.clone();
                                            if let Some(root) = ws.zones.get_mut(&next_root) {
                                                root.parent_id.clear();
                                            }
                                        }
                                    });
                                    selection.set(Selection::Workspace);
                                    sync_associations(&workspace);
                                    sync_json();
                                    status.set("Removed zone.".into());
                                }
                            } type="button">"Delete Zone"</button>
                        </div>
                        <details class="fold-card" open>
                            <summary>"Basics"</summary>
                            <div class="fold-body">
                        <label class="field">
                            <span>"Zone Name"</span>
                            <input
                                prop:value=zone.name.clone()
                                on:input=move |ev| {
                                    let value = event_target_value(&ev);
                                    workspace.update(|ws| {
                                        if let Some(zone) = ws.zones.get_mut(&zone_id_name) {
                                            zone.name = value;
                                        }
                                    });
                                    sync_json();
                                }
                            />
                        </label>
                        <label class="field">
                            <span>"Type"</span>
                            <input
                                prop:value=zone.zone_type.clone()
                                on:input=move |ev| {
                                    let value = event_target_value(&ev);
                                    workspace.update(|ws| {
                                        if let Some(zone) = ws.zones.get_mut(&zone_id_type) {
                                            zone.zone_type = value;
                                        }
                                    });
                                    sync_json();
                                }
                            />
                        </label>
                        <label class="field">
                            <span>"Grid"</span>
                            <select
                                prop:value=zone.grid_enabled.to_string()
                                on:change=move |ev| {
                                    let value = event_target_value(&ev) == "true";
                                    workspace.update(|ws| {
                                        if let Some(zone) = ws.zones.get_mut(&zone_id_grid) {
                                            zone.grid_enabled = value;
                                        }
                                    });
                                    sync_json();
                                }
                            >
                                <option value="false">"Disabled"</option>
                                <option value="true">"Enabled"</option>
                            </select>
                        </label>
                        <label class="field">
                            <span>"Resolution"</span>
                            <input
                                prop:value=zone.grid_resolution.to_string()
                                on:input=move |ev| {
                                    if let Ok(value) = event_target_value(&ev).parse::<f64>() {
                                        workspace.update(|ws| {
                                            if let Some(zone) = ws.zones.get_mut(&zone_id_res) {
                                                zone.grid_resolution = value;
                                            }
                                        });
                                        sync_json();
                                    }
                                }
                            />
                        </label>
                            </div>
                        </details>
                        <details class="fold-card" open>
                            <summary>"Traffic Rules"</summary>
                            <div class="fold-body">
                        <div class="field-grid">
                            <label class="field">
                                <span>"Traffic Policy"</span>
                                <input
                                    prop:value=get_prop(&zone.properties, "traffic.policy")
                                    on:input=move |ev| {
                                        let value = event_target_value(&ev);
                                        workspace.update(|ws| {
                                            if let Some(zone) = ws.zones.get_mut(&zone_id_policy) {
                                                set_prop(&mut zone.properties, "traffic.policy", value);
                                            }
                                        });
                                        sync_json();
                                    }
                                />
                            </label>
                            <label class="field">
                                <span>"Traffic Capacity"</span>
                                <input
                                    prop:value=get_prop(&zone.properties, "traffic.capacity")
                                    on:input=move |ev| {
                                        let value = event_target_value(&ev);
                                        workspace.update(|ws| {
                                            if let Some(zone) = ws.zones.get_mut(&zone_id_capacity) {
                                                set_prop(&mut zone.properties, "traffic.capacity", value);
                                            }
                                        });
                                        sync_json();
                                    }
                                />
                            </label>
                        </div>
                        <div class="field-grid">
                            <label class="field">
                                <span>"Traffic Priority"</span>
                                <input
                                    prop:value=get_prop(&zone.properties, "traffic.priority")
                                    on:input=move |ev| {
                                        let value = event_target_value(&ev);
                                        workspace.update(|ws| {
                                            if let Some(zone) = ws.zones.get_mut(&zone_id_priority) {
                                                set_prop(&mut zone.properties, "traffic.priority", value);
                                            }
                                        });
                                        sync_json();
                                    }
                                />
                            </label>
                            <label class="field">
                                <span>"Speed Limit"</span>
                                <input
                                    prop:value=get_prop(&zone.properties, "traffic.speed_limit")
                                    on:input=move |ev| {
                                        let value = event_target_value(&ev);
                                        workspace.update(|ws| {
                                            if let Some(zone) = ws.zones.get_mut(&zone_id_speed_limit) {
                                                set_prop(&mut zone.properties, "traffic.speed_limit", value);
                                            }
                                        });
                                        sync_json();
                                    }
                                />
                            </label>
                        </div>
                        <div class="field-grid">
                            <label class="field">
                                <span>"Claim Required"</span>
                                <select
                                    prop:value=get_bool_prop(&zone.properties, "traffic.claim_required").to_string()
                                    on:change=move |ev| {
                                        let value = event_target_value(&ev) == "true";
                                        workspace.update(|ws| {
                                            if let Some(zone) = ws.zones.get_mut(&zone_id_claim_required) {
                                                set_bool_prop(&mut zone.properties, "traffic.claim_required", value);
                                            }
                                        });
                                        sync_json();
                                    }
                                >
                                    <option value="false">"No"</option>
                                    <option value="true">"Yes"</option>
                                </select>
                            </label>
                            <label class="field">
                                <span>"Blocked"</span>
                                <select
                                    prop:value=get_bool_prop(&zone.properties, "traffic.blocked").to_string()
                                    on:change=move |ev| {
                                        let value = event_target_value(&ev) == "true";
                                        workspace.update(|ws| {
                                            if let Some(zone) = ws.zones.get_mut(&zone_id_blocked) {
                                                set_bool_prop(&mut zone.properties, "traffic.blocked", value);
                                            }
                                        });
                                        sync_json();
                                    }
                                >
                                    <option value="false">"No"</option>
                                    <option value="true">"Yes"</option>
                                </select>
                            </label>
                        </div>
                            </div>
                        </details>
                        <details class="fold-card" open>
                            <summary>"Geometry & Membership"</summary>
                            <div class="fold-body">
                        <label class="field">
                            <span>{move || if uses_local_coords(&workspace.get()) { "Polygon Y,X" } else { "Polygon Lat,Lon" }}</span>
                            <textarea
                                prop:value=move || {
                                    let ws = workspace.get();
                                    let zone = ws.zones.get(&zone_id_poly_value).cloned();
                                    zone.map(|zone| {
                                        zone.polygon_latlon.iter().map(|point| polygon_point_text(&ws, point)).collect::<Vec<_>>().join("\n")
                                    }).unwrap_or_default()
                                }
                                on:change=move |ev| {
                                    let value = event_target_value(&ev);
                                    workspace.update(|ws| {
                                        let next_polygon = parse_polygon_text(ws, &value);
                                        if let Some(zone) = ws.zones.get_mut(&zone_id_poly_change) {
                                            zone.polygon_latlon = next_polygon;
                                        }
                                    });
                                    sync_associations(&workspace);
                                    sync_json();
                                }
                            />
                        </label>
                        <label class="field">
                            <span>"Advanced Properties"</span>
                            <textarea
                                prop:value=properties_text(&zone.properties)
                                on:change=move |ev| {
                                    let value = event_target_value(&ev);
                                    workspace.update(|ws| {
                                        if let Some(zone) = ws.zones.get_mut(&zone_id_props) {
                                            zone.properties = parse_properties_text(&value);
                                        }
                                    });
                                    sync_json();
                                }
                            />
                        </label>
                        <label class="field">
                            <span>"Nodes"</span>
                            <textarea readonly prop:value=move || zone_node_lines(&workspace.get(), &zone_id_nodes).join("\n") />
                        </label>
                        <p class="hint">{format!("{} vertices", zone.polygon_latlon.len())}</p>
                            </div>
                        </details>
                    </div>
                }
                .into_any()
                }
                None => view! { <p class="hint">"Missing zone."</p> }.into_any(),
            }
        }
        Selection::Node(node_id) => {
            let node = workspace.get().nodes.get(&node_id).cloned();
            match node {
                Some(node) => {
                    let node_id_name = node_id.clone();
                    let node_id_lat_v = node_id.clone();
                    let node_id_lat_u = node_id.clone();
                    let node_id_lon_v = node_id.clone();
                    let node_id_lon_u = node_id.clone();
                    let node_id_zones = node_id.clone();
                    let node_id_props = node_id.clone();
                    view! {
                    <div class="inspector-stack">
                        <div class="inspector-header">
                            <div class="inspector-header-copy">
                                <p class="section">"Selection"</p>
                                <h3>{node.name.clone()}</h3>
                                <p class="inspector-id">{node.id.clone()}</p>
                            </div>
                            <button class="ghost danger-button" on:click={
                                let remove_id = node.id.clone();
                                move |_| {
                                    workspace.update(|ws| {
                                        ws.nodes.remove(&remove_id);
                                        ws.edges.retain(|_, edge| edge.source_id != remove_id && edge.target_id != remove_id);
                                    });
                                    selection.set(Selection::Workspace);
                                    sync_associations(&workspace);
                                    sync_json();
                                    status.set("Removed node.".into());
                                }
                            } type="button">"Delete Node"</button>
                        </div>
                        <details class="fold-card" open>
                            <summary>"Basics"</summary>
                            <div class="fold-body">
                                <label class="field">
                                    <span>"Node Name"</span>
                                    <input
                                        prop:value=node.name.clone()
                                        on:input=move |ev| {
                                            let value = event_target_value(&ev);
                                            workspace.update(|ws| {
                                                if let Some(node) = ws.nodes.get_mut(&node_id_name) {
                                                    node.name = value.clone();
                                                    node.properties.insert("name".into(), value);
                                                }
                                            });
                                            sync_json();
                                        }
                                    />
                                </label>
                                <div class="field-grid">
                                    <label class="field">
                                        <span>{move || if uses_local_coords(&workspace.get()) { "Y" } else { "Latitude" }}</span>
                                        <input
                                            prop:value=move || node_axis_text(&workspace.get(), &node_id_lat_v, true)
                                            on:input=move |ev| {
                                                let value = event_target_value(&ev);
                                                update_node_axis(&workspace, &node_id_lat_u, true, &value);
                                                sync_associations(&workspace);
                                                sync_json();
                                            }
                                        />
                                    </label>
                                    <label class="field">
                                        <span>{move || if uses_local_coords(&workspace.get()) { "X" } else { "Longitude" }}</span>
                                        <input
                                            prop:value=move || node_axis_text(&workspace.get(), &node_id_lon_v, false)
                                            on:input=move |ev| {
                                                let value = event_target_value(&ev);
                                                update_node_axis(&workspace, &node_id_lon_u, false, &value);
                                                sync_associations(&workspace);
                                                sync_json();
                                            }
                                        />
                                    </label>
                                </div>
                                <p class="hint">{format!("Lat {:.6} · Lon {:.6}", node.latlon.lat, node.latlon.lon)}</p>
                            </div>
                        </details>
                        <details class="fold-card" open>
                            <summary>"Membership & Properties"</summary>
                            <div class="fold-body">
                                <label class="field">
                                    <span>"Zones"</span>
                                    <textarea
                                        prop:value={
                                            let node_id_value = node_id_zones.clone();
                                            move || {
                                                let ws = workspace.get();
                                                let zone_ids = ws
                                                    .nodes
                                                    .get(&node_id_value)
                                                    .map(|node| node.zone_ids.clone())
                                                    .unwrap_or_default();
                                                zone_membership_lines(&ws, &zone_ids).join("\n")
                                            }
                                        }
                                        on:change=move |ev| {
                                            let value = event_target_value(&ev);
                                            workspace.update(|ws| {
                                                let zone_ids = parse_association_lines(&value)
                                                    .into_iter()
                                                    .filter(|zone_id| ws.zones.contains_key(zone_id))
                                                    .collect::<Vec<_>>();
                                                if let Some(node) = ws.nodes.get_mut(&node_id_zones) {
                                                    node.zone_ids = zone_ids;
                                                }
                                            });
                                            sync_associations(&workspace);
                                            sync_json();
                                        }
                                    />
                                </label>
                                <label class="field">
                                    <span>"Advanced Properties"</span>
                                    <textarea
                                        prop:value=properties_text(&node.properties)
                                        on:change=move |ev| {
                                            let value = event_target_value(&ev);
                                            workspace.update(|ws| {
                                                if let Some(node) = ws.nodes.get_mut(&node_id_props) {
                                                    let mut properties = parse_properties_text(&value);
                                                    properties.insert("name".into(), node.name.clone());
                                                    node.properties = properties;
                                                }
                                            });
                                            sync_json();
                                        }
                                    />
                                </label>
                            </div>
                        </details>
                    </div>
                }
                .into_any()
                }
                None => view! { <p class="hint">"Missing node."</p> }.into_any(),
            }
        }
        Selection::Edge(edge_id) => {
            let edge = workspace.get().edges.get(&edge_id).cloned();
            match edge {
                Some(edge) => {
                    let edge_id_weight = edge_id.clone();
                    let edge_id_directed = edge_id.clone();
                    let edge_id_source = edge_id.clone();
                    let edge_id_target = edge_id.clone();
                    let edge_id_reverse = edge_id.clone();
                    let edge_id_speed_limit = edge_id.clone();
                    let edge_id_lane_type = edge_id.clone();
                    let edge_id_reversible = edge_id.clone();
                    let edge_id_passing_allowed = edge_id.clone();
                    let edge_id_priority = edge_id.clone();
                    let edge_id_capacity = edge_id.clone();
                    let edge_id_no_stop = edge_id.clone();
                    let edge_id_cost_bias = edge_id.clone();
                    let edge_id_zones = edge_id.clone();
                    let edge_id_props = edge_id.clone();
                    view! {
                    <div class="inspector-stack">
                        <div class="inspector-header">
                            <div class="inspector-header-copy">
                                <p class="section">"Selection"</p>
                                <h3>"Edge"</h3>
                                <p class="inspector-id">{edge.id.clone()}</p>
                            </div>
                            <button class="ghost danger-button" on:click={
                                let remove_id = edge.id.clone();
                                move |_| {
                                    workspace.update(|ws| {
                                        ws.edges.remove(&remove_id);
                                    });
                                    selection.set(Selection::Workspace);
                                    sync_associations(&workspace);
                                    sync_json();
                                    status.set("Removed edge.".into());
                                }
                            } type="button">"Delete Edge"</button>
                        </div>
                        <details class="fold-card" open>
                            <summary>"Basics"</summary>
                            <div class="fold-body">
                                <p class="hint">{format!("{} -> {}", edge.source_id, edge.target_id)}</p>
                                <label class="field">
                                    <span>"Weight"</span>
                                    <input
                                        prop:value=edge.weight.to_string()
                                        on:input=move |ev| {
                                            let value = event_target_value(&ev);
                                            let Ok(weight) = value.parse::<f64>() else {
                                                return;
                                            };
                                            workspace.update(|ws| {
                                                if let Some(edge) = ws.edges.get_mut(&edge_id_weight) {
                                                    edge.weight = weight;
                                                }
                                            });
                                            sync_json();
                                        }
                                    />
                                </label>
                                <label class="field">
                                    <span>"Directed"</span>
                                    <select
                                        prop:value=edge.directed.to_string()
                                        on:change=move |ev| {
                                            let value = event_target_value(&ev) == "true";
                                            workspace.update(|ws| {
                                                if let Some(edge) = ws.edges.get_mut(&edge_id_directed) {
                                                    edge.directed = value;
                                                }
                                            });
                                            sync_json();
                                        }
                                    >
                                        <option value="false">"No"</option>
                                        <option value="true">"Yes"</option>
                                    </select>
                                </label>
                                <label class="field">
                                    <span>"Source"</span>
                                    <input
                                        prop:value=edge.source_id.clone()
                                        on:input=move |ev| {
                                            let value = event_target_value(&ev);
                                            workspace.update(|ws| {
                                                if let Some(edge) = ws.edges.get_mut(&edge_id_source) {
                                                    edge.source_id = value.trim().to_string();
                                                }
                                            });
                                            sync_associations(&workspace);
                                            sync_json();
                                        }
                                    />
                                </label>
                                <label class="field">
                                    <span>"Target"</span>
                                    <input
                                        prop:value=edge.target_id.clone()
                                        on:input=move |ev| {
                                            let value = event_target_value(&ev);
                                            workspace.update(|ws| {
                                                if let Some(edge) = ws.edges.get_mut(&edge_id_target) {
                                                    edge.target_id = value.trim().to_string();
                                                }
                                            });
                                            sync_associations(&workspace);
                                            sync_json();
                                        }
                                    />
                                </label>
                                <div class="dock-actions single-action">
                                    <button class="ghost" on:click=move |_| {
                                        workspace.update(|ws| {
                                            if let Some(edge) = ws.edges.get_mut(&edge_id_reverse) {
                                                std::mem::swap(&mut edge.source_id, &mut edge.target_id);
                                            }
                                        });
                                        sync_associations(&workspace);
                                        sync_json();
                                    }>"Reverse Direction"</button>
                                </div>
                            </div>
                        </details>
                        <details class="fold-card" open>
                            <summary>"Traffic & Membership"</summary>
                            <div class="fold-body">
                                <label class="field">
                                    <span>"Zones"</span>
                                    <textarea
                                        prop:value={
                                            let edge_id_value = edge_id_zones.clone();
                                            move || {
                                                let ws = workspace.get();
                                                let zone_ids = ws
                                                    .edges
                                                    .get(&edge_id_value)
                                                    .map(|edge| edge.zone_ids.clone())
                                                    .unwrap_or_default();
                                                zone_membership_lines(&ws, &zone_ids).join("\n")
                                            }
                                        }
                                        on:change=move |ev| {
                                            let value = event_target_value(&ev);
                                            workspace.update(|ws| {
                                                let zone_ids = parse_association_lines(&value)
                                                    .into_iter()
                                                    .filter(|zone_id| ws.zones.contains_key(zone_id))
                                                    .collect::<Vec<_>>();
                                                if let Some(edge) = ws.edges.get_mut(&edge_id_zones) {
                                                    edge.zone_ids = zone_ids;
                                                }
                                            });
                                            sync_associations(&workspace);
                                            sync_json();
                                        }
                                    />
                                </label>
                                <div class="field-grid">
                                    <label class="field">
                                        <span>"Speed Limit"</span>
                                        <input
                                            prop:value=get_prop(&edge.properties, "traffic.speed_limit")
                                            on:input=move |ev| {
                                                let value = event_target_value(&ev);
                                                workspace.update(|ws| {
                                                    if let Some(edge) = ws.edges.get_mut(&edge_id_speed_limit) {
                                                        set_prop(&mut edge.properties, "traffic.speed_limit", value);
                                                    }
                                                });
                                                sync_json();
                                            }
                                        />
                                    </label>
                                    <label class="field">
                                        <span>"Lane Type"</span>
                                        <input
                                            prop:value=get_prop(&edge.properties, "traffic.lane_type")
                                            on:input=move |ev| {
                                                let value = event_target_value(&ev);
                                                workspace.update(|ws| {
                                                    if let Some(edge) = ws.edges.get_mut(&edge_id_lane_type) {
                                                        set_prop(&mut edge.properties, "traffic.lane_type", value);
                                                    }
                                                });
                                                sync_json();
                                            }
                                        />
                                    </label>
                                </div>
                                <div class="field-grid">
                                    <label class="field">
                                        <span>"Reversible"</span>
                                        <select
                                            prop:value=get_bool_prop(&edge.properties, "traffic.reversible").to_string()
                                            on:change=move |ev| {
                                                let value = event_target_value(&ev) == "true";
                                                workspace.update(|ws| {
                                                    if let Some(edge) = ws.edges.get_mut(&edge_id_reversible) {
                                                        set_bool_prop(&mut edge.properties, "traffic.reversible", value);
                                                    }
                                                });
                                                sync_json();
                                            }
                                        >
                                            <option value="false">"No"</option>
                                            <option value="true">"Yes"</option>
                                        </select>
                                    </label>
                                    <label class="field">
                                        <span>"Passing Allowed"</span>
                                        <select
                                            prop:value=get_bool_prop(&edge.properties, "traffic.passing_allowed").to_string()
                                            on:change=move |ev| {
                                                let value = event_target_value(&ev) == "true";
                                                workspace.update(|ws| {
                                                    if let Some(edge) = ws.edges.get_mut(&edge_id_passing_allowed) {
                                                        set_bool_prop(&mut edge.properties, "traffic.passing_allowed", value);
                                                    }
                                                });
                                                sync_json();
                                            }
                                        >
                                            <option value="false">"No"</option>
                                            <option value="true">"Yes"</option>
                                        </select>
                                    </label>
                                </div>
                                <div class="field-grid">
                                    <label class="field">
                                        <span>"Traffic Priority"</span>
                                        <input
                                            prop:value=get_prop(&edge.properties, "traffic.priority")
                                            on:input=move |ev| {
                                                let value = event_target_value(&ev);
                                                workspace.update(|ws| {
                                                    if let Some(edge) = ws.edges.get_mut(&edge_id_priority) {
                                                        set_prop(&mut edge.properties, "traffic.priority", value);
                                                    }
                                                });
                                                sync_json();
                                            }
                                        />
                                    </label>
                                    <label class="field">
                                        <span>"Traffic Capacity"</span>
                                        <input
                                            prop:value=get_prop(&edge.properties, "traffic.capacity")
                                            on:input=move |ev| {
                                                let value = event_target_value(&ev);
                                                workspace.update(|ws| {
                                                    if let Some(edge) = ws.edges.get_mut(&edge_id_capacity) {
                                                        set_prop(&mut edge.properties, "traffic.capacity", value);
                                                    }
                                                });
                                                sync_json();
                                            }
                                        />
                                    </label>
                                </div>
                                <div class="field-grid">
                                    <label class="field">
                                        <span>"No Stop"</span>
                                        <select
                                            prop:value=get_bool_prop(&edge.properties, "traffic.no_stop").to_string()
                                            on:change=move |ev| {
                                                let value = event_target_value(&ev) == "true";
                                                workspace.update(|ws| {
                                                    if let Some(edge) = ws.edges.get_mut(&edge_id_no_stop) {
                                                        set_bool_prop(&mut edge.properties, "traffic.no_stop", value);
                                                    }
                                                });
                                                sync_json();
                                            }
                                        >
                                            <option value="false">"No"</option>
                                            <option value="true">"Yes"</option>
                                        </select>
                                    </label>
                                    <label class="field">
                                        <span>"Cost Bias"</span>
                                        <input
                                            prop:value=get_prop(&edge.properties, "traffic.cost_bias")
                                            on:input=move |ev| {
                                                let value = event_target_value(&ev);
                                                workspace.update(|ws| {
                                                    if let Some(edge) = ws.edges.get_mut(&edge_id_cost_bias) {
                                                        set_prop(&mut edge.properties, "traffic.cost_bias", value);
                                                    }
                                                });
                                                sync_json();
                                            }
                                        />
                                    </label>
                                </div>
                                <label class="field">
                                    <span>"Advanced Properties"</span>
                                    <textarea
                                        prop:value=properties_text(&edge.properties)
                                        on:change=move |ev| {
                                            let value = event_target_value(&ev);
                                            workspace.update(|ws| {
                                                if let Some(edge) = ws.edges.get_mut(&edge_id_props) {
                                                    edge.properties = parse_properties_text(&value);
                                                }
                                            });
                                            sync_json();
                                        }
                                    />
                                </label>
                            </div>
                        </details>
                    </div>
                }
                .into_any()
                }
                None => view! { <p class="hint">"Missing edge."</p> }.into_any(),
            }
        }
    }
}

fn sync_associations(workspace: &RwSignal<WorkspaceJson>) {
    let ordered = ordered_zones(&workspace.get());
    workspace.update(|ws| {
        for zone in ws.zones.values_mut() {
            zone.node_ids.clear();
        }

        let node_zone_map = ws
            .nodes
            .iter()
            .map(|(node_id, node)| {
                let zone_ids = ordered
                .iter()
                .filter(|zone| point_in_polygon(&node.latlon, &zone.polygon_latlon))
                .map(|zone| zone.id.clone())
                .collect::<Vec<_>>();
                (node_id.clone(), zone_ids)
            })
            .collect::<BTreeMap<_, _>>();

        for (node_id, zone_ids) in &node_zone_map {
            if let Some(node) = ws.nodes.get_mut(node_id) {
                node.zone_ids = zone_ids.clone();
                node.properties.insert("name".into(), node.name.clone());
            }
            for zone_id in zone_ids {
                if let Some(zone) = ws.zones.get_mut(zone_id) {
                    zone.node_ids.push(node_id.clone());
                }
            }
        }

        for edge in ws.edges.values_mut() {
            let mut ids = BTreeSet::new();
            if let Some(source_zone_ids) = node_zone_map.get(&edge.source_id) {
                ids.extend(source_zone_ids.iter().cloned());
            }
            if let Some(target_zone_ids) = node_zone_map.get(&edge.target_id) {
                ids.extend(target_zone_ids.iter().cloned());
            }
            edge.zone_ids = ids.into_iter().collect();
        }
    });
}

fn validation_messages(workspace: &WorkspaceJson) -> Vec<String> {
    let mut messages = Vec::new();
    if workspace.name.trim().is_empty() {
        messages.push("Workspace name is empty.".into());
    }
    if uses_local_coords(workspace) && workspace.ref_point.is_none() && workspace.datum.is_none() {
        messages.push("Local coordinate mode requires a ref or datum.".into());
    }
    if !workspace.root_zone_id.is_empty() && !workspace.zones.contains_key(&workspace.root_zone_id) {
        messages.push("Root zone id does not point to an existing zone.".into());
    }
    for zone in workspace.zones.values() {
        if zone.polygon_latlon.len() < 3 {
            messages.push(format!("Zone '{}' has fewer than 3 polygon points.", zone.name));
        }
        let unique_points = zone
            .polygon_latlon
            .iter()
            .map(|point| format!("{:.9}:{:.9}", point.lat, point.lon))
            .collect::<BTreeSet<_>>()
            .len();
        if unique_points < 3 {
            messages.push(format!("Zone '{}' has fewer than 3 unique polygon points.", zone.name));
        }
        if !zone.parent_id.is_empty() && !workspace.zones.contains_key(&zone.parent_id) {
            messages.push(format!("Zone '{}' has a missing parent '{}'.", zone.name, zone.parent_id));
        }
        for child_id in &zone.child_ids {
            if !workspace.zones.contains_key(child_id) {
                messages.push(format!("Zone '{}' references missing child '{}'.", zone.name, child_id));
                continue;
            }
            if workspace
                .zones
                .get(child_id)
                .map(|child| child.parent_id.as_str())
                != Some(zone.id.as_str())
            {
                messages.push(format!(
                    "Zone '{}' lists child '{}' but that child does not point back with matching parent_id.",
                    zone.name, child_id
                ));
            }
        }
        if !zone.parent_id.is_empty()
            && workspace
                .zones
                .get(&zone.parent_id)
                .map(|parent| parent.child_ids.contains(&zone.id))
                == Some(false)
        {
            messages.push(format!(
                "Zone '{}' points to parent '{}' but is missing from that parent's child_ids.",
                zone.name, zone.parent_id
            ));
        }
        for node_id in &zone.node_ids {
            if !workspace.nodes.contains_key(node_id) {
                messages.push(format!("Zone '{}' references missing node '{}'.", zone.name, node_id));
            }
        }
    }
    for node in workspace.nodes.values() {
        for zone_id in &node.zone_ids {
            if !workspace.zones.contains_key(zone_id) {
                messages.push(format!("Node '{}' references missing zone '{}'.", node.name, zone_id));
            }
        }
    }
    for edge in workspace.edges.values() {
        if !workspace.nodes.contains_key(&edge.source_id) {
            messages.push(format!("Edge '{}' has a missing source node.", edge.id));
        }
        if !workspace.nodes.contains_key(&edge.target_id) {
            messages.push(format!("Edge '{}' has a missing target node.", edge.id));
        }
        if !edge.weight.is_finite() {
            messages.push(format!("Edge '{}' has a non-finite weight.", edge.id));
        }
        for zone_id in &edge.zone_ids {
            if !workspace.zones.contains_key(zone_id) {
                messages.push(format!("Edge '{}' references missing zone '{}'.", edge.id, zone_id));
            }
        }
    }
    messages
}

fn ordered_zones(workspace: &WorkspaceJson) -> Vec<ZoneJson> {
    let mut ordered = Vec::new();
    let mut visited = BTreeSet::new();

    fn walk(
        workspace: &WorkspaceJson,
        zone_id: &str,
        visited: &mut BTreeSet<String>,
        ordered: &mut Vec<ZoneJson>,
    ) {
        let Some(zone) = workspace.zones.get(zone_id) else {
            return;
        };
        if !visited.insert(zone_id.to_string()) {
            return;
        }
        ordered.push(zone.clone());
        for child in &zone.child_ids {
            walk(workspace, child, visited, ordered);
        }
    }

    if !workspace.root_zone_id.is_empty() {
        walk(
            workspace,
            &workspace.root_zone_id,
            &mut visited,
            &mut ordered,
        );
    }

    ordered
}

fn root_zone_ids(workspace: &WorkspaceJson) -> Vec<String> {
    let mut ids = Vec::new();
    if !workspace.root_zone_id.is_empty() && workspace.zones.contains_key(&workspace.root_zone_id) {
        ids.push(workspace.root_zone_id.clone());
    }
    let mut orphan_ids = workspace
        .zones
        .values()
        .filter(|zone| zone.parent_id.is_empty() && zone.id != workspace.root_zone_id)
        .map(|zone| zone.id.clone())
        .collect::<Vec<_>>();
    orphan_ids.sort_by_key(|id| workspace.zones.get(id).map(|zone| zone.name.clone()).unwrap_or_default());
    ids.extend(orphan_ids);
    ids
}

fn sorted_child_zone_ids(workspace: &WorkspaceJson, zone_id: &str) -> Vec<String> {
    let mut ids = workspace
        .zones
        .values()
        .filter(|zone| zone.parent_id == zone_id)
        .map(|zone| zone.id.clone())
        .collect::<Vec<_>>();
    ids.sort_by_key(|id| workspace.zones.get(id).map(|zone| zone.name.clone()).unwrap_or_default());
    ids
}

fn zone_depth(workspace: &WorkspaceJson, zone_id: &str) -> usize {
    let mut depth = 0;
    let mut current = zone_id;
    while let Some(zone) = workspace.zones.get(current) {
        if zone.parent_id.is_empty() {
            break;
        }
        depth += 1;
        current = &zone.parent_id;
    }
    depth
}

fn workspace_center(workspace: &WorkspaceJson) -> JsonPoint {
    workspace
        .datum
        .clone()
        .or_else(|| workspace.ref_point.clone())
        .unwrap_or(JsonPoint {
            lat: 52.0,
            lon: 5.0,
        })
}

fn point_text(point: &JsonPoint) -> String {
    format!("{:.5}, {:.5}", point.lat, point.lon)
}

fn node_summary(workspace: &WorkspaceJson, node_id: &str) -> String {
    workspace
        .nodes
        .get(node_id)
        .map(|node| node.name.clone())
        .unwrap_or_else(|| node_id.to_string())
}

fn uses_local_coords(workspace: &WorkspaceJson) -> bool {
    (workspace.coord_mode == CoordMode::Local || workspace.local_ref) && workspace.ref_point.is_some()
}

fn properties_text(properties: &BTreeMap<String, String>) -> String {
    properties
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_properties_text(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (key, value) = line.split_once('=').unwrap_or((line, ""));
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn get_prop(properties: &BTreeMap<String, String>, key: &str) -> String {
    properties.get(key).cloned().unwrap_or_default()
}

fn get_bool_prop(properties: &BTreeMap<String, String>, key: &str) -> bool {
    properties.get(key).map(|value| value == "true").unwrap_or(false)
}

fn set_prop(properties: &mut BTreeMap<String, String>, key: &str, value: String) {
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        properties.remove(key);
    } else {
        properties.insert(key.to_string(), trimmed);
    }
}

fn set_bool_prop(properties: &mut BTreeMap<String, String>, key: &str, value: bool) {
    if value {
        properties.insert(key.to_string(), "true".into());
    } else {
        properties.remove(key);
    }
}

fn zone_node_lines(workspace: &WorkspaceJson, zone_id: &str) -> Vec<String> {
    let Some(zone) = workspace.zones.get(zone_id) else {
        return Vec::new();
    };
    zone.node_ids
        .iter()
        .map(|node_id| {
            workspace
                .nodes
                .get(node_id)
                .map(|node| format!("{} ({})", node.name, node.id))
                .unwrap_or_else(|| node_id.clone())
        })
        .collect()
}

fn zone_membership_lines(workspace: &WorkspaceJson, zone_ids: &[String]) -> Vec<String> {
    zone_ids
        .iter()
        .map(|zone_id| {
            workspace
                .zones
                .get(zone_id)
                .map(|zone| format!("{} ({})", zone.name, zone.id))
                .unwrap_or_else(|| zone_id.clone())
        })
        .collect()
}

fn parse_association_lines(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let id = if let Some(open_idx) = line.rfind('(') {
            if line.ends_with(')') && open_idx + 1 < line.len() - 1 {
                line[open_idx + 1..line.len() - 1].trim()
            } else {
                line
            }
        } else {
            line
        };
        let id = id.trim();
        if !id.is_empty() && !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }
    ids
}

fn polygon_point_text(workspace: &WorkspaceJson, point: &JsonPoint) -> String {
    if uses_local_coords(workspace) {
        let local = to_local_xy(workspace, point);
        format!("{:.3}, {:.3}", local.1, local.0)
    } else {
        format!("{:.6}, {:.6}", point.lat, point.lon)
    }
}

fn parse_polygon_text(workspace: &WorkspaceJson, text: &str) -> Vec<JsonPoint> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let parts = line.split(',').map(|part| part.trim().parse::<f64>().ok()).collect::<Vec<_>>();
            if parts.len() != 2 {
                return None;
            }
            match (parts[0], parts[1]) {
                (Some(a), Some(b)) => {
                    if uses_local_coords(workspace) {
                        Some(from_local_xy(workspace, b, a))
                    } else {
                        Some(JsonPoint { lat: a, lon: b })
                    }
                }
                _ => None,
            }
        })
        .collect()
}

fn to_local_xy(workspace: &WorkspaceJson, point: &JsonPoint) -> (f64, f64) {
    let origin = workspace.ref_point.as_ref().or(workspace.datum.as_ref()).unwrap_or(point);
    let lat_scale = 111_320.0;
    let lon_scale = 111_320.0 * origin.lat.to_radians().cos().abs().max(0.1);
    let x = (point.lon - origin.lon) * lon_scale;
    let y = (point.lat - origin.lat) * lat_scale;
    (x, y)
}

fn from_local_xy(workspace: &WorkspaceJson, x: f64, y: f64) -> JsonPoint {
    let origin = workspace
        .ref_point
        .clone()
        .or_else(|| workspace.datum.clone())
        .unwrap_or(JsonPoint { lat: 52.0, lon: 5.0 });
    let lat_scale = 111_320.0;
    let lon_scale = 111_320.0 * origin.lat.to_radians().cos().abs().max(0.1);
    JsonPoint {
        lat: origin.lat + y / lat_scale,
        lon: origin.lon + x / lon_scale,
    }
}

fn node_axis_text(workspace: &WorkspaceJson, node_id: &str, primary: bool) -> String {
    let Some(node) = workspace.nodes.get(node_id) else {
        return String::new();
    };
    if uses_local_coords(workspace) {
        let (x, y) = to_local_xy(workspace, &node.latlon);
        if primary { y.to_string() } else { x.to_string() }
    } else if primary {
        node.latlon.lat.to_string()
    } else {
        node.latlon.lon.to_string()
    }
}

fn update_node_axis(workspace: &RwSignal<WorkspaceJson>, node_id: &str, primary: bool, value: &str) {
    let Ok(value) = value.parse::<f64>() else {
        return;
    };
    workspace.update(|ws| {
        let local_mode = uses_local_coords(ws);
        if local_mode {
            let current = ws.nodes.get(node_id).map(|node| to_local_xy(ws, &node.latlon));
            if let Some((x, y)) = current {
                let (next_x, next_y) = if primary { (x, value) } else { (value, y) };
                let next = from_local_xy(ws, next_x, next_y);
                if let Some(node) = ws.nodes.get_mut(node_id) {
                    node.latlon = next;
                }
            }
        } else if let Some(node) = ws.nodes.get_mut(node_id) {
            if primary {
                node.latlon.lat = value;
            } else {
                node.latlon.lon = value;
            }
        }
    });
}

fn point_in_polygon(point: &JsonPoint, polygon: &[JsonPoint]) -> bool {
    if polygon.len() < 3 {
        return false;
    }

    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let xi = polygon[i].lon;
        let yi = polygon[i].lat;
        let xj = polygon[j].lon;
        let yj = polygon[j].lat;
        let denom = if (yj - yi).abs() <= f64::EPSILON {
            f64::EPSILON
        } else {
            yj - yi
        };
        let intersects = ((yi > point.lat) != (yj > point.lat))
            && (point.lon < (xj - xi) * (point.lat - yi) / denom + xi);
        if intersects {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn polygon_bounds(points: &[JsonPoint]) -> Option<(f64, f64, f64, f64)> {
    if points.is_empty() {
        return None;
    }
    let min_lat = points.iter().map(|point| point.lat).fold(f64::INFINITY, f64::min);
    let max_lat = points.iter().map(|point| point.lat).fold(f64::NEG_INFINITY, f64::max);
    let min_lon = points.iter().map(|point| point.lon).fold(f64::INFINITY, f64::min);
    let max_lon = points.iter().map(|point| point.lon).fold(f64::NEG_INFINITY, f64::max);
    Some((min_lat, max_lat, min_lon, max_lon))
}

fn sync_raw_json(workspace: RwSignal<WorkspaceJson>, raw_json: RwSignal<String>) {
    raw_json.set(serde_json::to_string_pretty(&workspace.get()).unwrap_or_else(|_| "{}".into()));
}

fn sync_workspace_state(workspace: RwSignal<WorkspaceJson>, raw_json: RwSignal<String>) {
    sync_associations(&workspace);
    sync_raw_json(workspace, raw_json);
}

fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

fn visible_robots(robots: &[RobotSummary], _now_ms: u64) -> Vec<RobotSummary> {
    robots.to_vec()
}

fn is_robot_stale(robot: &RobotSummary, now_ms: u64) -> bool {
    let _ = now_ms;
    robot.stale
}

fn robot_has_available_tasks(robots: &[RobotSummary], robot_name: &str) -> bool {
    !robot_name.is_empty()
        && robots
            .iter()
            .find(|robot| robot.name == robot_name)
            .map(|robot| !robot.available_tasks.is_empty())
            .unwrap_or(false)
}

fn random_robot_color() -> String {
    let mut rng = rand::thread_rng();
    let hue = rng.gen_range(0..360);
    let saturation = rng.gen_range(62..82);
    let lightness = rng.gen_range(54..68);
    format!("hsl({hue} {saturation}% {lightness}%)")
}

fn preserve_robot_colors(current: &[RobotSummary], next: &mut [RobotSummary]) {
    let colors_by_name = current
        .iter()
        .map(|robot| (robot.name.clone(), robot.color.clone()))
        .collect::<BTreeMap<_, _>>();

    for robot in next {
        if let Some(color) = colors_by_name.get(&robot.name) {
            robot.color = color.clone();
        }
    }
}

async fn refresh_zenoh_status(
    zenoh_state: RwSignal<ZenohConnectionState>,
    zenoh_status: RwSignal<String>,
    zenoh_endpoint: RwSignal<String>,
    zenoh_connection_type: RwSignal<ZenohConnectionType>,
    app_status: RwSignal<String>,
    update_app_status: bool,
) {
    if let Ok(response) = zenoh_status_api().await {
        apply_zenoh_status_response(
            response,
            zenoh_state,
            zenoh_status,
            zenoh_endpoint,
            zenoh_connection_type,
            app_status,
            update_app_status,
        );
    }
}

async fn refresh_robots(robots: RwSignal<Vec<RobotSummary>>) {
    if let Ok(mut next) = robots_api().await {
        preserve_robot_colors(&robots.get_untracked(), &mut next);
        robots.set(next);
    }
}

async fn connect_zenoh_api(
    request: ZenohConnectRequest,
    zenoh_state: RwSignal<ZenohConnectionState>,
    zenoh_status: RwSignal<String>,
    zenoh_endpoint: RwSignal<String>,
    zenoh_connection_type: RwSignal<ZenohConnectionType>,
    app_status: RwSignal<String>,
) {
    match zenoh_connect_request(request).await {
        Ok(response) => apply_zenoh_status_response(
            response,
            zenoh_state,
            zenoh_status,
            zenoh_endpoint,
            zenoh_connection_type,
            app_status,
            true,
        ),
        Err(error) => {
            zenoh_state.set(ZenohConnectionState::Error);
            zenoh_status.set(error.clone());
            app_status.set(error);
        }
    }
}

async fn disconnect_zenoh_api(
    zenoh_state: RwSignal<ZenohConnectionState>,
    zenoh_status: RwSignal<String>,
    zenoh_endpoint: RwSignal<String>,
    zenoh_connection_type: RwSignal<ZenohConnectionType>,
    app_status: RwSignal<String>,
) {
    match zenoh_disconnect_request().await {
        Ok(response) => apply_zenoh_status_response(
            response,
            zenoh_state,
            zenoh_status,
            zenoh_endpoint,
            zenoh_connection_type,
            app_status,
            true,
        ),
        Err(error) => {
            zenoh_state.set(ZenohConnectionState::Error);
            zenoh_status.set(error.clone());
            app_status.set(error);
        }
    }
}

fn apply_zenoh_status_response(
    response: ZenohStatusResponse,
    zenoh_state: RwSignal<ZenohConnectionState>,
    zenoh_status: RwSignal<String>,
    zenoh_endpoint: RwSignal<String>,
    zenoh_connection_type: RwSignal<ZenohConnectionType>,
    app_status: RwSignal<String>,
    update_app_status: bool,
) {
    zenoh_state.set(response.state);
    zenoh_status.set(response.status.clone());
    if !response.endpoint.is_empty() {
        zenoh_endpoint.set(response.endpoint.clone());
    }
    zenoh_connection_type.set(response.connection_type);
    if update_app_status {
        app_status.set(response.status);
    }
}

async fn zenoh_status_api() -> Result<ZenohStatusResponse, String> {
    zenoh_api_request("GET", "status", None).await
}

async fn zenoh_connect_request(request: ZenohConnectRequest) -> Result<ZenohStatusResponse, String> {
    let body = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    zenoh_api_request("POST", "connect", Some(body)).await
}

async fn zenoh_disconnect_request() -> Result<ZenohStatusResponse, String> {
    zenoh_api_request("POST", "disconnect", None).await
}

async fn refresh_scheduler_tasks(
    robot: String,
    scheduler_task_options: RwSignal<Vec<String>>,
    scheduler_selected_task: RwSignal<String>,
    app_status: RwSignal<String>,
) {
    match scheduler_tasks_api(&robot).await {
        Ok(response) => {
            scheduler_task_options.set(response.tasks);
            scheduler_selected_task.set(String::new());
            app_status.set(response.status);
        }
        Err(error) => {
            scheduler_task_options.set(Vec::new());
            scheduler_selected_task.set(String::new());
            app_status.set(error);
        }
    }
}

async fn run_scheduler_task_api(
    request: SchedulerRunRequest,
    app_status: RwSignal<String>,
) {
    match scheduler_run_request(request).await {
        Ok(response) => app_status.set(response.status),
        Err(error) => app_status.set(error),
    }
}

async fn scheduler_tasks_api(robot: &str) -> Result<SchedulerTasksResponse, String> {
    let Some(window) = web_sys::window() else {
        return Err("Browser window is unavailable.".into());
    };

    let init = RequestInit::new();
    init.set_cache(RequestCache::NoStore);
    let request = Request::new_with_str_and_init(&format!("/api/zenoh/tasks/{robot}"), &init)
        .map_err(js_error_text)?;
    request
        .headers()
        .set("Accept", "application/json")
        .map_err(js_error_text)?;

    let response_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|error| format!("{} Could not reach /api/zenoh/tasks/{robot}.", js_error_text(error)))?;
    let response: Response = response_value
        .dyn_into()
        .map_err(|_| String::from("Invalid backend response."))?;
    let text = JsFuture::from(response.text().map_err(js_error_text)?)
        .await
        .map_err(js_error_text)?
        .as_string()
        .unwrap_or_default();
    let payload: SchedulerTasksResponse =
        serde_json::from_str(&text).map_err(|error| format!("Invalid backend JSON: {error}"))?;
    if response.ok() {
        Ok(payload)
    } else {
        Err(payload.status)
    }
}

async fn scheduler_run_request(request: SchedulerRunRequest) -> Result<SchedulerRunResponse, String> {
    let Some(window) = web_sys::window() else {
        return Err("Browser window is unavailable.".into());
    };

    let body = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_cache(RequestCache::NoStore);
    init.set_body(&JsValue::from_str(&body));

    let request = Request::new_with_str_and_init("/api/zenoh/task", &init)
        .map_err(js_error_text)?;
    request
        .headers()
        .set("Accept", "application/json")
        .map_err(js_error_text)?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(js_error_text)?;

    let response_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|error| format!("{} Could not reach /api/zenoh/task.", js_error_text(error)))?;
    let response: Response = response_value
        .dyn_into()
        .map_err(|_| String::from("Invalid backend response."))?;
    let text = JsFuture::from(response.text().map_err(js_error_text)?)
        .await
        .map_err(js_error_text)?
        .as_string()
        .unwrap_or_default();
    let payload: SchedulerRunResponse =
        serde_json::from_str(&text).map_err(|error| format!("Invalid backend JSON: {error}"))?;
    if response.ok() {
        Ok(payload)
    } else {
        Err(payload.status)
    }
}

async fn robots_api() -> Result<Vec<RobotSummary>, String> {
    let Some(window) = web_sys::window() else {
        return Err("Browser window is unavailable.".into());
    };

    let init = RequestInit::new();
    init.set_cache(RequestCache::NoStore);
    let request = Request::new_with_str_and_init("/api/robots", &init).map_err(js_error_text)?;
    request
        .headers()
        .set("Accept", "application/json")
        .map_err(js_error_text)?;

    let response_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|error| format!("{} Could not reach /api/robots.", js_error_text(error)))?;
    let response: Response = response_value
        .dyn_into()
        .map_err(|_| String::from("Invalid backend response."))?;
    let text = JsFuture::from(response.text().map_err(js_error_text)?)
        .await
        .map_err(js_error_text)?
        .as_string()
        .unwrap_or_default();

    if response.ok() {
        serde_json::from_str(&text).map_err(|error| format!("Invalid backend JSON: {error}"))
    } else {
        Err(text)
    }
}

async fn zenoh_api_request(
    method: &str,
    path: &str,
    body: Option<String>,
) -> Result<ZenohStatusResponse, String> {
    let Some(window) = web_sys::window() else {
        return Err("Browser window is unavailable.".into());
    };

    let init = RequestInit::new();
    init.set_method(method);
    init.set_cache(RequestCache::NoStore);
    if let Some(body) = body.as_ref() {
        init.set_body(&JsValue::from_str(body));
    }

    let request = Request::new_with_str_and_init(&format!("/api/zenoh/{path}"), &init)
        .map_err(js_error_text)?;
    request
        .headers()
        .set("Accept", "application/json")
        .map_err(js_error_text)?;
    if body.is_some() {
        request
            .headers()
            .set("Content-Type", "application/json")
            .map_err(js_error_text)?;
    }

    let response_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|error| {
            format!(
                "{} Could not reach {}.",
                js_error_text(error),
                format!("/api/zenoh/{path}")
            )
        })?;
    let response: Response = response_value
        .dyn_into()
        .map_err(|_| String::from("Invalid backend response."))?;
    let text = JsFuture::from(response.text().map_err(js_error_text)?)
        .await
        .map_err(js_error_text)?
        .as_string()
        .unwrap_or_default();

    let payload: ZenohStatusResponse =
        serde_json::from_str(&text).map_err(|error| format!("Invalid backend JSON: {error}"))?;

    if response.ok() {
        Ok(payload)
    } else {
        Err(payload.status)
    }
}

fn js_error_text(value: JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "Browser request failed.".into())
}

fn select_item(
    selection: RwSignal<Selection>,
    active_right_panel: RwSignal<Option<RightPanel>>,
    status: RwSignal<String>,
    next: Selection,
    message: &str,
) {
    selection.set(next);
    active_right_panel.set(Some(RightPanel::Details));
    status.set(message.into());
}

fn normalize_workspace_json(workspace: &mut WorkspaceJson) {
    if workspace.local_ref && workspace.coord_mode == CoordMode::Global {
        workspace.coord_mode = CoordMode::Local;
    }
    workspace.local_ref = workspace.coord_mode == CoordMode::Local;

    for node in workspace.nodes.values_mut() {
        if node.name.trim().is_empty() {
            node.name = node
                .properties
                .get("name")
                .cloned()
                .unwrap_or_else(|| "Node".into());
        } else {
            node.properties.insert("name".into(), node.name.clone());
        }
    }
}

fn save_workspace_json(raw_json: &str, status: RwSignal<String>) {
    let array = js_sys::Array::new();
    array.push(&JsValue::from_str(raw_json));
    let blob = match web_sys::Blob::new_with_str_sequence(&array) {
        Ok(blob) => blob,
        Err(_) => {
            status.set("Failed to create workspace blob.".into());
            return;
        }
    };

    let url = match web_sys::Url::create_object_url_with_blob(&blob) {
        Ok(url) => url,
        Err(_) => {
            status.set("Failed to create download URL.".into());
            return;
        }
    };

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        let _ = web_sys::Url::revoke_object_url(&url);
        status.set("Document unavailable for save.".into());
        return;
    };

    let anchor = match document
        .create_element("a")
        .ok()
        .and_then(|element| element.dyn_into::<HtmlAnchorElement>().ok())
    {
        Some(anchor) => anchor,
        None => {
            let _ = web_sys::Url::revoke_object_url(&url);
            status.set("Failed to create download link.".into());
            return;
        }
    };

    anchor.set_href(&url);
    anchor.set_download("workspace.json");
    anchor.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    status.set("Saved workspace.json.".into());
}

fn open_workspace_json_file(
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    active_right_panel: RwSignal<Option<RightPanel>>,
    raw_json: RwSignal<String>,
    status: RwSignal<String>,
) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        status.set("Document unavailable for open.".into());
        return;
    };

    let Some(input) = document
        .create_element("input")
        .ok()
        .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
    else {
        status.set("Failed to create file input.".into());
        return;
    };

    input.set_type("file");
    input.set_accept(".json,application/json");

    let input_for_change = input.clone();
    let change = Closure::<dyn FnMut(Event)>::wrap(Box::new(move |_: Event| {
        let Some(files) = input_for_change.files() else {
            return;
        };
        let Some(file) = files.get(0) else {
            return;
        };
        let Ok(reader) = FileReader::new() else {
            status.set("Failed to read file.".into());
            return;
        };
        let reader_for_load = reader.clone();
        let onload = Closure::<dyn FnMut(ProgressEvent)>::wrap(Box::new(move |_: ProgressEvent| {
            let Some(result) = reader_for_load.result().ok() else {
                status.set("Failed to load workspace JSON.".into());
                return;
            };
            let Some(text) = result.as_string() else {
                status.set("Workspace file was not text.".into());
                return;
            };
            match serde_json::from_str::<WorkspaceJson>(&text) {
                Ok(mut next) => {
                    normalize_workspace_json(&mut next);
                    workspace.set(next);
                    sync_associations(&workspace);
                    raw_json.set(
                        serde_json::to_string_pretty(&workspace.get()).unwrap_or_else(|_| "{}".into()),
                    );
                    selection.set(Selection::Workspace);
                    active_right_panel.set(Some(RightPanel::Files));
                    status.set("Loaded workspace.json.".into());
                }
                Err(error) => status.set(format!("Invalid JSON: {error}")),
            }
        }));
        reader.set_onload(Some(onload.as_ref().unchecked_ref()));
        onload.forget();
        let _ = reader.read_as_text(&file);
    }));

    input.set_onchange(Some(change.as_ref().unchecked_ref()));
    change.forget();
    input.click();
}

fn finish_zone_draft(
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    mode: RwSignal<Mode>,
    zone_draft_points: RwSignal<Vec<JsonPoint>>,
    pending_zone_parent: RwSignal<Option<String>>,
    active_right_panel: RwSignal<Option<RightPanel>>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    map: &JsValue,
) {
    let polygon = zone_draft_points.get();
    if polygon.len() < 3 {
        status.set("Need at least 3 points before the zone can be created.".into());
        return;
    }

    let parent_id = pending_zone_parent.get();
    let zone_id = new_id();
    workspace.update(|ws| {
        let zone_type = if parent_id.is_none() { "root" } else { "zone" }.to_string();
        let zone_name = if parent_id.is_none() {
            "Root Zone".to_string()
        } else {
            format!("Zone {}", ws.zones.len() + 1)
        };
        ws.zones.insert(
            zone_id.clone(),
            ZoneJson {
                id: zone_id.clone(),
                name: zone_name,
                zone_type,
                parent_id: parent_id.clone().unwrap_or_default(),
                child_ids: Vec::new(),
                node_ids: Vec::new(),
                polygon_latlon: polygon.clone(),
                grid_enabled: false,
                grid_resolution: 1.0,
                properties: BTreeMap::new(),
            },
        );
        if let Some(parent_id) = &parent_id {
            if let Some(parent) = ws.zones.get_mut(parent_id) {
                parent.child_ids.push(zone_id.clone());
            }
        } else {
            ws.root_zone_id = zone_id.clone();
        }
    });
    zone_draft_points.set(Vec::new());
    pending_zone_parent.set(None);
    mode.set(Mode::Inspect);
    selection.set(Selection::Zone(zone_id.clone()));
    active_right_panel.set(Some(RightPanel::Details));
    if let Some(zone) = workspace.get().zones.get(&zone_id).cloned() {
        let _ = focus_map_zone(map, &zone);
    }
    sync_workspace_state(workspace, raw_json);
    status.set("Created zone polygon.".into());
}

fn place_node_at(
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    active_right_panel: RwSignal<Option<RightPanel>>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    point: JsonPoint,
) {
    let id = new_id();
    workspace.update(|ws| {
        ws.nodes.insert(
            id.clone(),
            NodeJson {
                id: id.clone(),
                name: format!("Node {}", ws.nodes.len() + 1),
                latlon: point,
                zone_ids: Vec::new(),
                properties: BTreeMap::from([("name".into(), format!("Node {}", ws.nodes.len() + 1))]),
            },
        );
    });
    selection.set(Selection::Node(id));
    active_right_panel.set(Some(RightPanel::Details));
    sync_workspace_state(workspace, raw_json);
    status.set("Placed node.".into());
}

fn set_reference_point(
    workspace: RwSignal<WorkspaceJson>,
    mode: RwSignal<Mode>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    point: JsonPoint,
) {
    workspace.update(|ws| {
        ws.ref_point = Some(point.clone());
        if ws.datum.is_none() {
            ws.datum = Some(point);
        }
    });
    mode.set(Mode::Inspect);
    sync_workspace_state(workspace, raw_json);
    status.set("Placed reference point.".into());
}

fn locate_me(map: &JsValue, workspace: RwSignal<WorkspaceJson>, status: RwSignal<String>) {
    let Some(window) = web_sys::window() else {
        status.set("Window unavailable for geolocation.".into());
        return;
    };
    let Ok(navigator) = js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("navigator")) else {
        status.set("Navigator is unavailable in this browser.".into());
        return;
    };
    let Ok(geolocation) = js_sys::Reflect::get(&navigator, &JsValue::from_str("geolocation")) else {
        status.set("Geolocation is unavailable in this browser.".into());
        return;
    };

    status.set("Requesting browser location...".into());

    let map = map.clone();
    let success = Closure::<dyn FnMut(JsValue)>::wrap(Box::new(move |position: JsValue| {
        let Ok(coords) = js_sys::Reflect::get(&position, &JsValue::from_str("coords")) else {
            status.set("Failed to read browser location.".into());
            return;
        };
        let lat = js_sys::Reflect::get(&coords, &JsValue::from_str("latitude"))
            .ok()
            .and_then(|value| value.as_f64());
        let lon = js_sys::Reflect::get(&coords, &JsValue::from_str("longitude"))
            .ok()
            .and_then(|value| value.as_f64());
        let (Some(lat), Some(lon)) = (lat, lon) else {
            status.set("Browser location did not include coordinates.".into());
            return;
        };
        let point = JsonPoint { lat, lon };
        workspace.update(|ws| {
            if ws.datum.is_none() {
                ws.datum = Some(point.clone());
            }
        });
        let _ = focus_map_point(&map, &point, 16.0);
        status.set("Centered map near your current browser location.".into());
    }));

    let error = Closure::<dyn FnMut(JsValue)>::wrap(Box::new(move |_| {
        status.set("Browser location request was denied or failed.".into());
    }));

    let Ok(get_current_position) = js_sys::Reflect::get(&geolocation, &JsValue::from_str("getCurrentPosition"))
        .and_then(|value| value.dyn_into::<js_sys::Function>().map_err(Into::into))
    else {
        status.set("Geolocation API is unavailable.".into());
        return;
    };

    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &JsValue::from_str("enableHighAccuracy"), &JsValue::TRUE);
    let _ = js_sys::Reflect::set(&options, &JsValue::from_str("timeout"), &JsValue::from_f64(10_000.0));
    let _ = js_sys::Reflect::set(&options, &JsValue::from_str("maximumAge"), &JsValue::from_f64(300_000.0));
    let _ = get_current_position.call3(
        &geolocation,
        success.as_ref().unchecked_ref(),
        error.as_ref().unchecked_ref(),
        &options,
    );
    success.forget();
    error.forget();
}

fn pick_node_for_edge(
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    edge_source_id: RwSignal<String>,
    active_right_panel: RwSignal<Option<RightPanel>>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    node_id: String,
) {
    let source_id = edge_source_id.get();
    if source_id.is_empty() {
        edge_source_id.set(node_id.clone());
        select_item(
            selection,
            active_right_panel,
            status,
            Selection::Node(node_id),
            "Selected edge source. Pick a target node.",
        );
        return;
    }
    if source_id == node_id {
        return;
    }
    let edge_id = new_id();
    workspace.update(|ws| {
        ws.edges.insert(
            edge_id.clone(),
            EdgeJson {
                id: edge_id.clone(),
                source_id: source_id.clone(),
                target_id: node_id,
                directed: true,
                weight: 1.0,
                zone_ids: Vec::new(),
                properties: BTreeMap::new(),
            },
        );
    });
    edge_source_id.set(String::new());
    selection.set(Selection::Edge(edge_id));
    active_right_panel.set(Some(RightPanel::Details));
    sync_workspace_state(workspace, raw_json);
    status.set("Created edge.".into());
}

fn remove_node_by_id(
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    node_id: &str,
) {
    workspace.update(|ws| {
        ws.nodes.remove(node_id);
        ws.edges
            .retain(|_, edge| edge.source_id != node_id && edge.target_id != node_id);
    });
    selection.set(Selection::Workspace);
    sync_workspace_state(workspace, raw_json);
    status.set("Removed node.".into());
}

fn remove_edge_by_id(
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    edge_id: &str,
) {
    workspace.update(|ws| {
        ws.edges.remove(edge_id);
    });
    selection.set(Selection::Workspace);
    sync_workspace_state(workspace, raw_json);
    status.set("Removed edge.".into());
}

fn remove_zone_vertex(
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    zone_id: &str,
    index: usize,
) {
    workspace.update(|ws| {
        let Some(zone) = ws.zones.get_mut(zone_id) else {
            return;
        };
        if zone.polygon_latlon.len() <= 3 || index >= zone.polygon_latlon.len() {
            return;
        }
        zone.polygon_latlon.remove(index);
    });
    selection.set(Selection::Zone(zone_id.to_string()));
    sync_workspace_state(workspace, raw_json);
    status.set("Removed zone vertex.".into());
}

fn project_map_point(map: &JsValue, point: &JsonPoint) -> Option<(f64, f64)> {
    let lng_lat = js_sys::Array::new();
    lng_lat.push(&JsValue::from_f64(point.lon));
    lng_lat.push(&JsValue::from_f64(point.lat));
    let projected = call_method1(map, "project", &lng_lat.into()).ok()?;
    let x = js_sys::Reflect::get(&projected, &JsValue::from_str("x")).ok()?.as_f64()?;
    let y = js_sys::Reflect::get(&projected, &JsValue::from_str("y")).ok()?.as_f64()?;
    Some((x, y))
}

fn point_to_segment_distance_px(point: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let abx = b.0 - a.0;
    let aby = b.1 - a.1;
    let apx = point.0 - a.0;
    let apy = point.1 - a.1;
    let ab_len_sq = abx * abx + aby * aby;
    if ab_len_sq <= f64::EPSILON {
        return ((point.0 - a.0).powi(2) + (point.1 - a.1).powi(2)).sqrt();
    }
    let t = ((apx * abx + apy * aby) / ab_len_sq).clamp(0.0, 1.0);
    let closest_x = a.0 + t * abx;
    let closest_y = a.1 + t * aby;
    ((point.0 - closest_x).powi(2) + (point.1 - closest_y).powi(2)).sqrt()
}

fn zone_projected_bounds(map: &JsValue, zone: &ZoneJson) -> Option<(f64, f64)> {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for point in &zone.polygon_latlon {
        let (x, y) = project_map_point(map, point)?;
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
        any = true;
    }
    if !any {
        return None;
    }
    Some((max_x - min_x, max_y - min_y))
}

fn is_near_zone_border(map: &JsValue, zone: &ZoneJson, point: &JsonPoint, threshold_px: f64) -> bool {
    if zone.polygon_latlon.len() < 2 {
        return false;
    }
    let Some(click_px) = project_map_point(map, point) else {
        return false;
    };
    let mut best_distance_sq = f64::INFINITY;
    for index in 0..zone.polygon_latlon.len() {
        let start = &zone.polygon_latlon[index];
        let end = &zone.polygon_latlon[(index + 1) % zone.polygon_latlon.len()];
        let (Some(a), Some(b)) = (project_map_point(map, start), project_map_point(map, end)) else {
            continue;
        };
        let distance = point_to_segment_distance_px(click_px, a, b);
        best_distance_sq = best_distance_sq.min(distance * distance);
    }
    best_distance_sq <= threshold_px * threshold_px
}

fn is_small_zone_on_screen(map: &JsValue, zone: &ZoneJson) -> bool {
    let Some((width, height)) = zone_projected_bounds(map, zone) else {
        return false;
    };
    width <= 24.0 || height <= 24.0
}

fn selectable_zone_id_at_point(map: &JsValue, workspace: &WorkspaceJson, zone_ids: &[String], point: &JsonPoint) -> Option<String> {
    for zone_id in zone_ids {
        let Some(zone) = workspace.zones.get(zone_id) else {
            continue;
        };
        if is_near_zone_border(map, zone, point, 14.0) || is_small_zone_on_screen(map, zone) {
            return Some(zone_id.clone());
        }
    }
    None
}

fn project_point_to_segment(point: &JsonPoint, start: &JsonPoint, end: &JsonPoint) -> (JsonPoint, f64) {
    let ax = start.lon;
    let ay = start.lat;
    let bx = end.lon;
    let by = end.lat;
    let px = point.lon;
    let py = point.lat;

    let abx = bx - ax;
    let aby = by - ay;
    let apx = px - ax;
    let apy = py - ay;
    let ab_len_sq = abx * abx + aby * aby;
    let t = if ab_len_sq <= f64::EPSILON {
        0.0
    } else {
        ((apx * abx + apy * aby) / ab_len_sq).clamp(0.0, 1.0)
    };
    let projected = JsonPoint {
        lat: ay + aby * t,
        lon: ax + abx * t,
    };
    let dx = px - projected.lon;
    let dy = py - projected.lat;
    (projected, dx * dx + dy * dy)
}

fn is_near_selected_zone_border(map: &JsValue, workspace: &WorkspaceJson, selection: &Selection, point: &JsonPoint) -> bool {
    let Selection::Zone(zone_id) = selection else {
        return false;
    };
    let Some(zone) = workspace.zones.get(zone_id) else {
        return false;
    };
    is_near_zone_border(map, zone, point, 16.0)
}

fn insert_zone_vertex_at_click(
    map: &JsValue,
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    zone_id: &str,
    point: JsonPoint,
) {
    let Some(zone) = workspace.get().zones.get(zone_id).cloned() else {
        return;
    };
    if zone.polygon_latlon.len() < 2 {
        return;
    }
    let now = js_sys::Date::now();
    let should_skip = LAST_ZONE_INSERT.with(|cell| {
        let last = cell.borrow();
        last.0 == zone_id && now - last.1 < 250.0
    });
    if should_skip {
        return;
    }

    let mut best_index = None;
    let mut best_projection = None;
    let mut best_distance_sq = f64::INFINITY;
    for index in 0..zone.polygon_latlon.len() {
        let start = &zone.polygon_latlon[index];
        let end = &zone.polygon_latlon[(index + 1) % zone.polygon_latlon.len()];
        let (projection, distance_sq) = project_point_to_segment(&point, start, end);
        if distance_sq < best_distance_sq {
            best_distance_sq = distance_sq;
            best_index = Some(index + 1);
            best_projection = Some(projection);
        }
    }

    let Some(insert_at) = best_index else {
        return;
    };
    let Some(projected) = best_projection else {
        return;
    };

    let too_close = zone.polygon_latlon.iter().any(|vertex| {
        let (Some(a), Some(b)) = (project_map_point(map, vertex), project_map_point(map, &projected)) else {
            return false;
        };
        let dx = a.0 - b.0;
        let dy = a.1 - b.1;
        dx * dx + dy * dy <= 12.0 * 12.0
    });

    if too_close {
        status.set("Vertex already exists very close to that spot.".into());
        return;
    }

    workspace.update(|ws| {
        if let Some(zone) = ws.zones.get_mut(zone_id) {
            zone.polygon_latlon.insert(insert_at, projected.clone());
        }
    });
    LAST_ZONE_INSERT.with(|cell| {
        *cell.borrow_mut() = (zone_id.to_string(), now);
    });
    selection.set(Selection::Zone(zone_id.to_string()));
    sync_workspace_state(workspace, raw_json);
    status.set("Inserted zone vertex.".into());
}

fn centroid(points: &[JsonPoint]) -> JsonPoint {
    if points.is_empty() {
        return JsonPoint::default();
    }
    let (sum_lat, sum_lon) = points.iter().fold((0.0, 0.0), |(lat, lon), point| {
        (lat + point.lat, lon + point.lon)
    });
    let len = points.len() as f64;
    JsonPoint {
        lat: sum_lat / len,
        lon: sum_lon / len,
    }
}

fn midpoint(a: &JsonPoint, b: &JsonPoint) -> JsonPoint {
    JsonPoint {
        lat: (a.lat + b.lat) * 0.5,
        lon: (a.lon + b.lon) * 0.5,
    }
}

fn point_along_segment(a: &JsonPoint, b: &JsonPoint, t: f64) -> JsonPoint {
    JsonPoint {
        lat: a.lat + (b.lat - a.lat) * t,
        lon: a.lon + (b.lon - a.lon) * t,
    }
}

fn offset_from_direction(origin: &JsonPoint, from: &JsonPoint, to: &JsonPoint, forward_meters: f64, lateral_meters: f64) -> JsonPoint {
    let meters_per_deg_lat = 111_320.0;
    let meters_per_deg_lon = (111_320.0 * origin.lat.to_radians().cos()).abs().max(1.0);
    let dx_m = (to.lon - from.lon) * meters_per_deg_lon;
    let dy_m = (to.lat - from.lat) * meters_per_deg_lat;
    let length = (dx_m * dx_m + dy_m * dy_m).sqrt().max(f64::EPSILON);
    let tx = dx_m / length;
    let ty = dy_m / length;
    let nx = -dy_m / length;
    let ny = dx_m / length;

    JsonPoint {
        lat: origin.lat + (ty * forward_meters + ny * lateral_meters) / meters_per_deg_lat,
        lon: origin.lon + (tx * forward_meters + nx * lateral_meters) / meters_per_deg_lon,
    }
}

fn bind_map_interactions(
    map: &JsValue,
    workspace: RwSignal<WorkspaceJson>,
    app_mode: RwSignal<AppMode>,
    selection: RwSignal<Selection>,
    mode: RwSignal<Mode>,
    edge_source_id: RwSignal<String>,
    hovered_node_id: RwSignal<String>,
    hovered_edge_id: RwSignal<String>,
    preview_mouse_point: RwSignal<Option<JsonPoint>>,
    scene_drag: RwSignal<Option<SceneDrag>>,
    zone_draft_points: RwSignal<Vec<JsonPoint>>,
    pending_zone_parent: RwSignal<Option<String>>,
    active_right_panel: RwSignal<Option<RightPanel>>,
    management_right_panel: RwSignal<Option<RightPanel>>,
    scheduler_picking_node: RwSignal<bool>,
    status: RwSignal<String>,
    raw_json: RwSignal<String>,
    marker_dragging: RwSignal<bool>,
) -> Result<(), JsValue> {
    enable_middle_pan(map)?;

    for layer in [
        "zone-fill",
        "zone-line",
        "zone-hit",
        "selected-zone-fill",
        "selected-zone-line",
        "selected-zone-hit",
        "node-circle",
        "node-hit",
        "edge-line",
        "edge-hit",
    ] {
        attach_map_layer_event(map, "mouseenter", layer, {
            let map = map.clone();
            let layer = layer.to_string();
            move |_| {
                let cursor = if matches!(
                    layer.as_str(),
                    "edge-line" | "edge-hit" | "selected-zone-line" | "selected-zone-hit" | "zone-line" | "zone-hit"
                ) {
                    "pointer"
                } else {
                    "crosshair"
                };
                let _ = set_map_cursor(&map, cursor);
            }
        })?;
        attach_map_layer_event(map, "mouseleave", layer, {
            let map = map.clone();
            move |_| {
                let _ = clear_map_cursor(&map);
            }
        })?;
    }

    attach_map_layer_event(map, "mouseenter", "node-circle", {
        move |event| {
            let Some(id) = first_feature_id(&event) else {
                return;
            };
            hovered_node_id.set(id);
        }
    })?;
    attach_map_layer_event(map, "mouseenter", "node-hit", {
        move |event| {
            let Some(id) = first_feature_id(&event) else {
                return;
            };
            hovered_node_id.set(id);
        }
    })?;
    attach_map_layer_event(map, "mousedown", "node-circle", {
        move |event| {
            if app_mode.get() != AppMode::Editor || mode.get() != Mode::Inspect || event_button(&event) != Some(0) {
                return;
            }
            let Some(id) = first_feature_id(&event) else {
                return;
            };
            scene_drag.set(Some(SceneDrag::Node(id.clone())));
            marker_dragging.set(true);
            selection.set(Selection::Node(id));
            active_right_panel.set(Some(RightPanel::Details));
        }
    })?;
    attach_map_layer_event(map, "mousedown", "node-hit", {
        move |event| {
            if app_mode.get() != AppMode::Editor || mode.get() != Mode::Inspect || event_button(&event) != Some(0) {
                return;
            }
            let Some(id) = first_feature_id(&event) else {
                return;
            };
            scene_drag.set(Some(SceneDrag::Node(id.clone())));
            marker_dragging.set(true);
            selection.set(Selection::Node(id));
            active_right_panel.set(Some(RightPanel::Details));
        }
    })?;
    attach_map_layer_event(map, "mousedown", "selected-zone-fill", {
        move |event| {
            if app_mode.get() != AppMode::Editor || mode.get() != Mode::Inspect || event_button(&event) != Some(0) {
                return;
            }
            let Some(zone_id) = first_feature_id(&event) else {
                return;
            };
            let Some(point) = point_from_map_event(&event) else {
                return;
            };
            scene_drag.set(Some(SceneDrag::ZoneBody(zone_id.clone(), point)));
            marker_dragging.set(true);
            selection.set(Selection::Zone(zone_id));
            active_right_panel.set(Some(RightPanel::Details));
        }
    })?;
    attach_map_layer_event(map, "mousedown", "selected-zone-line", {
        let map = map.clone();
        move |event| {
            if app_mode.get() != AppMode::Editor || mode.get() != Mode::Inspect || event_button(&event) != Some(0) {
                return;
            }
            let Some(zone_id) = first_feature_id(&event) else {
                return;
            };
            let Some(point) = point_from_map_event(&event) else {
                return;
            };
            let ws = workspace.get();
            let Some(zone) = ws.zones.get(&zone_id) else {
                return;
            };
            if let Some(index) = nearest_zone_vertex_index(&map, zone, &point, 14.0) {
                scene_drag.set(Some(SceneDrag::ZoneVertex(zone_id.clone(), index)));
                marker_dragging.set(true);
                selection.set(Selection::Zone(zone_id));
                active_right_panel.set(Some(RightPanel::Details));
            }
        }
    })?;
    attach_map_layer_event(map, "mousedown", "selected-zone-hit", {
        let map = map.clone();
        move |event| {
            if app_mode.get() != AppMode::Editor || mode.get() != Mode::Inspect || event_button(&event) != Some(0) {
                return;
            }
            let Some(zone_id) = first_feature_id(&event) else {
                return;
            };
            let Some(point) = point_from_map_event(&event) else {
                return;
            };
            let ws = workspace.get();
            let Some(zone) = ws.zones.get(&zone_id) else {
                return;
            };
            if let Some(index) = nearest_zone_vertex_index(&map, zone, &point, 14.0) {
                scene_drag.set(Some(SceneDrag::ZoneVertex(zone_id.clone(), index)));
                marker_dragging.set(true);
                selection.set(Selection::Zone(zone_id));
                active_right_panel.set(Some(RightPanel::Details));
            }
        }
    })?;
    attach_map_layer_event(map, "mouseleave", "node-circle", {
        move |_| {
            hovered_node_id.set(String::new());
        }
    })?;
    attach_map_layer_event(map, "mouseleave", "node-hit", {
        move |_| {
            hovered_node_id.set(String::new());
        }
    })?;

    attach_map_layer_event(map, "mouseenter", "edge-hit", {
        move |event| {
            let Some(id) = first_feature_id(&event) else {
                return;
            };
            hovered_edge_id.set(id);
        }
    })?;
    attach_map_layer_event(map, "mouseleave", "edge-hit", {
        move |_| {
            hovered_edge_id.set(String::new());
        }
    })?;

    attach_map_event(map, "mousemove", move |event| {
        if app_mode.get() != AppMode::Editor {
            preview_mouse_point.set(None);
            return;
        }
        if let Some(point) = point_from_map_event(&event) {
            match scene_drag.get() {
                Some(SceneDrag::Node(node_id)) => {
                    workspace.update(|ws| {
                        if let Some(node) = ws.nodes.get_mut(&node_id) {
                            node.latlon = point.clone();
                        }
                    });
                    sync_associations(&workspace);
                    return;
                }
                Some(SceneDrag::ZoneVertex(zone_id, index)) => {
                    workspace.update(|ws| {
                        if let Some(zone) = ws.zones.get_mut(&zone_id) {
                            if index < zone.polygon_latlon.len() {
                                zone.polygon_latlon[index] = point.clone();
                            }
                        }
                    });
                    sync_associations(&workspace);
                    return;
                }
                Some(SceneDrag::ZoneBody(zone_id, previous)) => {
                    let delta_lat = point.lat - previous.lat;
                    let delta_lon = point.lon - previous.lon;
                    workspace.update(|ws| {
                        if let Some(zone) = ws.zones.get_mut(&zone_id) {
                            zone.polygon_latlon = zone
                                .polygon_latlon
                                .iter()
                                .map(|p| JsonPoint {
                                    lat: p.lat + delta_lat,
                                    lon: p.lon + delta_lon,
                                })
                                .collect();
                        }
                    });
                    scene_drag.set(Some(SceneDrag::ZoneBody(zone_id, point.clone())));
                    sync_associations(&workspace);
                    return;
                }
                None => {}
            }
        }

        if mode.get() != Mode::ConnectEdge || edge_source_id.get().is_empty() {
            preview_mouse_point.set(None);
            return;
        }
        preview_mouse_point.set(point_from_map_event(&event));
    })?;

    attach_map_event(map, "mouseup", move |_| {
        let drag = scene_drag.get();
        scene_drag.set(None);
        if drag.is_some() {
            marker_dragging.set(false);
            sync_workspace_state(workspace, raw_json);
            status.set("Moved scene item.".into());
        }
    })?;

    attach_map_event(map, "click", {
        let map = map.clone();
        move |event| {
            let current_app_mode = app_mode.get();
            let scheduler_pick_active =
                current_app_mode == AppMode::Management
                    && scheduler_picking_node.get()
                    && matches!(
                        management_right_panel.get(),
                        Some(RightPanel::Scheduler | RightPanel::Tasks)
                    );
            if current_app_mode != AppMode::Editor && !scheduler_pick_active {
                return;
            }
            if scene_drag.get().is_some() {
                return;
            }
            let Some(point) = point_from_map_event(&event) else {
                return;
            };
            if mode.get() == Mode::PlaceRef {
                set_reference_point(workspace, mode, status, raw_json, point);
                return;
            }
            if mode.get() == Mode::PlaceZone {
                zone_draft_points.update(|points| points.push(point));
                let point_count = zone_draft_points.with(|points| points.len());
                status.set(format!(
                    "Zone draft: {point_count} point{} added. Right-click to finish.",
                    if point_count == 1 { "" } else { "s" }
                ));
                return;
            }

            let node_features = query_rendered_feature_ids(&map, &event, &["node-hit", "node-circle"]);
            if let Some(node_id) = node_features.first().cloned() {
                if scheduler_pick_active {
                    selection.set(Selection::Node(node_id.clone()));
                    status.set(format!("Scheduler target node {node_id} selected."));
                } else if mode.get() == Mode::ConnectEdge {
                    pick_node_for_edge(
                        workspace,
                        selection,
                        edge_source_id,
                        active_right_panel,
                        status,
                        raw_json,
                        node_id,
                    );
                } else {
                    select_item(
                        selection,
                        active_right_panel,
                        status,
                        Selection::Node(node_id),
                        "Node selected.",
                    );
                }
                return;
            }

            let edge_features = query_rendered_feature_ids(&map, &event, &["edge-hit"]);
            if let Some(edge_id) = edge_features.first().cloned() {
                select_item(
                    selection,
                    active_right_panel,
                    status,
                    Selection::Edge(edge_id),
                    "Edge selected.",
                );
                return;
            }

            if mode.get() == Mode::AddNode {
                place_node_at(
                    workspace,
                    selection,
                    active_right_panel,
                    status,
                    raw_json,
                    point,
                );
                return;
            }

            let zone_features = query_rendered_feature_ids(
                &map,
                &event,
                &[
                    "zone-hit",
                    "zone-line",
                    "zone-fill",
                    "selected-zone-hit",
                    "selected-zone-line",
                    "selected-zone-fill",
                ],
            );
            let workspace_snapshot = workspace.get();
            if let Some(zone_id) = selectable_zone_id_at_point(&map, &workspace_snapshot, &zone_features, &point) {
                select_item(
                    selection,
                    active_right_panel,
                    status,
                    Selection::Zone(zone_id),
                    "Zone selected.",
                );
                return;
            }

            selection.set(Selection::Workspace);
            active_right_panel.set(None);
        }
    })?;

    attach_map_event(map, "contextmenu", {
        let map = map.clone();
        move |event| {
            if app_mode.get() != AppMode::Editor {
                return;
            }
            if mode.get() == Mode::PlaceZone {
                finish_zone_draft(
                    workspace,
                    selection,
                    mode,
                    zone_draft_points,
                    pending_zone_parent,
                    active_right_panel,
                    status,
                    raw_json,
                    &map,
                );
                return;
            }

            let node_features = query_rendered_feature_ids(&map, &event, &["node-hit", "node-circle"]);
            if let Some(node_id) = node_features.first().cloned() {
                remove_node_by_id(workspace, selection, status, raw_json, &node_id);
                return;
            }

            let edge_features = query_rendered_feature_ids(&map, &event, &["edge-hit"]);
            if let Some(edge_id) = edge_features.first().cloned() {
                remove_edge_by_id(workspace, selection, status, raw_json, &edge_id);
                return;
            }

            let Selection::Zone(zone_id) = selection.get() else {
                return;
            };
            let Some(point) = point_from_map_event(&event) else {
                return;
            };
            let current_selection = selection.get();
            let workspace_snapshot = workspace.get();
            if !is_near_selected_zone_border(&map, &workspace_snapshot, &current_selection, &point) {
                status.set("Right-click near a selected zone border to insert a vertex.".into());
                return;
            }
            insert_zone_vertex_at_click(&map, workspace, selection, status, raw_json, &zone_id, point);
        }
    })?;

    Ok(())
}


fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn event_target_value(ev: &Event) -> String {
    ev.target()
        .and_then(|target| target.dyn_into::<HtmlElement>().ok())
        .and_then(|element| js_sys::Reflect::get(&element, &JsValue::from_str("value")).ok())
        .and_then(|value| value.as_string())
        .unwrap_or_default()
}

fn init_basemap(
    container: HtmlElement,
    center: &JsonPoint,
    map_view_tick: RwSignal<u64>,
    map_size: RwSignal<(f64, f64)>,
) -> Result<JsValue, JsValue> {
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &JsValue::from_str("container"), &container)?;
    js_sys::Reflect::set(
        &options,
        &JsValue::from_str("style"),
        &JsValue::from_str("https://tiles.openfreemap.org/styles/liberty"),
    )?;
    let center_array = js_sys::Array::new();
    center_array.push(&JsValue::from_f64(center.lon));
    center_array.push(&JsValue::from_f64(center.lat));
    js_sys::Reflect::set(&options, &JsValue::from_str("center"), &center_array)?;
    js_sys::Reflect::set(&options, &JsValue::from_str("zoom"), &JsValue::from_f64(16.0))?;
    let global = js_sys::global();
    let maplibre = js_sys::Reflect::get(&global, &JsValue::from_str("maplibregl"))?;
    let ctor = js_sys::Reflect::get(&maplibre, &JsValue::from_str("Map"))?
        .dyn_into::<js_sys::Function>()?;
    let args = js_sys::Array::new();
    args.push(&options);
    let map = js_sys::Reflect::construct(&ctor, &args)?;
    update_map_size(&container, map_size);
    let resize = js_sys::Reflect::get(&map, &JsValue::from_str("resize"))?
        .dyn_into::<js_sys::Function>()?;
    resize.call0(&map)?;
    attach_map_event(&map, "load", {
        let container = container.clone();
        move |_| {
            update_map_size(&container, map_size);
            map_view_tick.update(|tick| *tick += 1);
        }
    })?;
    attach_map_event(&map, "move", {
        let container = container.clone();
        move |_| {
            update_map_size(&container, map_size);
            map_view_tick.update(|tick| *tick += 1);
        }
    })?;
    attach_map_event(&map, "zoom", {
        let container = container.clone();
        move |_| {
            update_map_size(&container, map_size);
            map_view_tick.update(|tick| *tick += 1);
        }
    })?;
    attach_map_event(&map, "resize", move |_| {
        update_map_size(&container, map_size);
        map_view_tick.update(|tick| *tick += 1);
    })?;
    attach_map_event(&map, "load", move |_| {})?;
    Ok(map)
}

fn attach_map_event<F>(map: &JsValue, name: &str, handler: F) -> Result<(), JsValue>
where
    F: FnMut(JsValue) + 'static,
{
    let on = js_sys::Reflect::get(map, &JsValue::from_str("on"))?.dyn_into::<js_sys::Function>()?;
    let closure = Closure::<dyn FnMut(JsValue)>::wrap(Box::new(handler));
    on.call2(map, &JsValue::from_str(name), closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

fn attach_map_layer_event<F>(map: &JsValue, name: &str, layer: &str, handler: F) -> Result<(), JsValue>
where
    F: FnMut(JsValue) + 'static,
{
    let on = js_sys::Reflect::get(map, &JsValue::from_str("on"))?.dyn_into::<js_sys::Function>()?;
    let closure = Closure::<dyn FnMut(JsValue)>::wrap(Box::new(handler));
    on.call3(
        map,
        &JsValue::from_str(name),
        &JsValue::from_str(layer),
        closure.as_ref().unchecked_ref(),
    )?;
    closure.forget();
    Ok(())
}

fn attach_object_event<F>(target: &JsValue, name: &str, handler: F) -> Result<(), JsValue>
where
    F: FnMut(JsValue) + 'static,
{
    let on = js_sys::Reflect::get(target, &JsValue::from_str("on"))?.dyn_into::<js_sys::Function>()?;
    let closure = Closure::<dyn FnMut(JsValue)>::wrap(Box::new(handler));
    on.call2(target, &JsValue::from_str(name), closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

fn update_map_size(container: &HtmlElement, map_size: RwSignal<(f64, f64)>) {
    map_size.set((
        f64::from(container.client_width().max(1)),
        f64::from(container.client_height().max(1)),
    ));
}

fn point_from_map_event(event: &JsValue) -> Option<JsonPoint> {
    let lng_lat = js_sys::Reflect::get(event, &JsValue::from_str("lngLat")).ok()?;
    let lon = js_sys::Reflect::get(&lng_lat, &JsValue::from_str("lng")).ok()?.as_f64()?;
    let lat = js_sys::Reflect::get(&lng_lat, &JsValue::from_str("lat")).ok()?.as_f64()?;
    Some(JsonPoint { lat, lon })
}

fn feature_id(feature: &JsValue) -> Option<String> {
    let props = js_sys::Reflect::get(feature, &JsValue::from_str("properties")).ok()?;
    js_sys::Reflect::get(&props, &JsValue::from_str("id"))
        .ok()
        .and_then(|value| value.as_string())
}

fn first_feature_id(event: &JsValue) -> Option<String> {
    let features = js_sys::Reflect::get(event, &JsValue::from_str("features")).ok()?;
    let array = js_sys::Array::from(&features);
    feature_id(&array.get(0))
}

fn set_map_cursor(map: &JsValue, cursor: &str) -> Result<(), JsValue> {
    let canvas = call_method0(map, "getCanvas")?;
    let style = js_sys::Reflect::get(&canvas, &JsValue::from_str("style"))?;
    js_sys::Reflect::set(&style, &JsValue::from_str("cursor"), &JsValue::from_str(cursor))?;
    Ok(())
}

fn clear_map_cursor(map: &JsValue) -> Result<(), JsValue> {
    let canvas = call_method0(map, "getCanvas")?;
    let style = js_sys::Reflect::get(&canvas, &JsValue::from_str("style"))?;
    js_sys::Reflect::set(&style, &JsValue::from_str("cursor"), &JsValue::from_str(""))?;
    Ok(())
}

fn query_rendered_feature_ids(map: &JsValue, event: &JsValue, layers: &[&str]) -> Vec<String> {
    let Ok(point) = js_sys::Reflect::get(event, &JsValue::from_str("point")) else {
        return Vec::new();
    };
    let options = js_sys::Object::new();
    let layer_array = js_sys::Array::new();
    for layer in layers {
        layer_array.push(&JsValue::from_str(layer));
    }
    if js_sys::Reflect::set(&options, &JsValue::from_str("layers"), &layer_array).is_err() {
        return Vec::new();
    }
    let Ok(features) = call_method2(map, "queryRenderedFeatures", &point, &options) else {
        return Vec::new();
    };
    let array = js_sys::Array::from(&features);
    array
        .iter()
        .filter_map(|feature| feature_id(&feature))
        .collect()
}

fn event_button(event: &JsValue) -> Option<i32> {
    let original = js_sys::Reflect::get(event, &JsValue::from_str("originalEvent")).ok()?;
    js_sys::Reflect::get(&original, &JsValue::from_str("button")).ok()?.as_f64().map(|value| value as i32)
}

fn nearest_zone_vertex_index(map: &JsValue, zone: &ZoneJson, point: &JsonPoint, threshold_px: f64) -> Option<usize> {
    let click_px = project_map_point(map, point)?;
    let mut best = None;
    let mut best_dist_sq = threshold_px * threshold_px;
    for (index, vertex) in zone.polygon_latlon.iter().enumerate() {
        let vertex_px = project_map_point(map, vertex)?;
        let dx = click_px.0 - vertex_px.0;
        let dy = click_px.1 - vertex_px.1;
        let dist_sq = dx * dx + dy * dy;
        if dist_sq <= best_dist_sq {
            best_dist_sq = dist_sq;
            best = Some(index);
        }
    }
    best
}

fn enable_middle_pan(map: &JsValue) -> Result<(), JsValue> {
    if let Ok(drag_pan) = js_sys::Reflect::get(map, &JsValue::from_str("dragPan")) {
        let _ = call_method0(&drag_pan, "disable");
    }
    let canvas_container: HtmlElement = call_method0(map, "getCanvasContainer")?.dyn_into()?;
    let canvas: HtmlElement = call_method0(map, "getCanvas")?.dyn_into()?;
    let state = std::rc::Rc::new(RefCell::new(None::<(i32, i32)>));

    {
        let state = state.clone();
        let canvas = canvas.clone();
        let down = Closure::<dyn FnMut(MouseEvent)>::wrap(Box::new(move |event: MouseEvent| {
            if event.button() != 1 {
                return;
            }
            event.prevent_default();
            *state.borrow_mut() = Some((event.client_x(), event.client_y()));
            let _ = canvas.style().set_property("cursor", "grabbing");
        }));
        canvas_container.add_event_listener_with_callback("mousedown", down.as_ref().unchecked_ref())?;
        down.forget();
    }

    {
        let state = state.clone();
        let map = map.clone();
        let move_handler = Closure::<dyn FnMut(MouseEvent)>::wrap(Box::new(move |event: MouseEvent| {
            let Some((last_x, last_y)) = *state.borrow() else {
                return;
            };
            event.prevent_default();
            let delta_x = event.client_x() - last_x;
            let delta_y = event.client_y() - last_y;
            *state.borrow_mut() = Some((event.client_x(), event.client_y()));
            let offset = js_sys::Array::new();
            offset.push(&JsValue::from_f64(-(delta_x as f64)));
            offset.push(&JsValue::from_f64(-(delta_y as f64)));
            let options = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&options, &JsValue::from_str("animate"), &JsValue::FALSE);
            let _ = call_method2(&map, "panBy", &offset.into(), &options.into());
        }));
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("window unavailable"))?
            .add_event_listener_with_callback("mousemove", move_handler.as_ref().unchecked_ref())?;
        move_handler.forget();
    }

    {
        let state = state.clone();
        let canvas = canvas.clone();
        let up = Closure::<dyn FnMut(MouseEvent)>::wrap(Box::new(move |event: MouseEvent| {
            if event.button() != 1 || state.borrow().is_none() {
                return;
            }
            *state.borrow_mut() = None;
            let _ = canvas.style().set_property("cursor", "");
        }));
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("window unavailable"))?
            .add_event_listener_with_callback("mouseup", up.as_ref().unchecked_ref())?;
        up.forget();
    }

    {
        let aux = Closure::<dyn FnMut(MouseEvent)>::wrap(Box::new(move |event: MouseEvent| {
            if event.button() == 1 {
                event.prevent_default();
            }
        }));
        canvas_container.add_event_listener_with_callback("auxclick", aux.as_ref().unchecked_ref())?;
        aux.forget();
    }

    Ok(())
}

fn ensure_map_layers(map: &JsValue) -> Result<(), JsValue> {
    if get_map_source(map, "zones").is_none() {
        add_geojson_source(map, "zones")?;
        add_geojson_source(map, "zone_draft")?;
        add_geojson_source(map, "selected_zone")?;
        add_geojson_source(map, "nodes")?;
        add_geojson_source(map, "edges")?;
        add_geojson_source(map, "edge_arrows")?;
        add_geojson_source(map, "edge_preview")?;
        add_geojson_source(map, "reference")?;

        add_layer(map, zone_fill_layer()?)?;
        add_layer(map, zone_line_layer()?)?;
        add_layer(map, zone_hit_layer()?)?;
        add_layer(map, zone_draft_fill_layer()?)?;
        add_layer(map, zone_draft_line_layer()?)?;
        add_layer(map, zone_draft_point_layer()?)?;
        add_layer(map, selected_zone_fill_layer()?)?;
        add_layer(map, selected_zone_line_layer()?)?;
        add_layer(map, selected_zone_hit_layer()?)?;
        add_layer(map, edge_line_layer()?)?;
        add_layer(map, edge_arrow_fill_layer()?)?;
        add_layer(map, edge_arrow_line_layer()?)?;
        add_layer(map, edge_hit_layer()?)?;
        add_layer(map, edge_preview_line_layer()?)?;
        add_layer(map, node_circle_layer()?)?;
        add_layer(map, node_hit_layer()?)?;
        add_layer(map, reference_circle_layer()?)?;
    }
    Ok(())
}

fn map_style_loaded(map: &JsValue) -> bool {
    call_method0(map, "isStyleLoaded")
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn update_map_sources(
    map: &JsValue,
    workspace: &WorkspaceJson,
    selection: &Selection,
    edge_source_id: &str,
    hovered_node_id: &str,
    hovered_edge_id: &str,
    preview_mouse_point: Option<&JsonPoint>,
    zone_draft_points: &[JsonPoint],
) -> Result<(), JsValue> {
    set_source_data(map, "zones", zones_geojson(workspace, selection)?)?;
    set_source_data(map, "zone_draft", zone_draft_geojson(zone_draft_points)?)?;
    set_source_data(map, "selected_zone", selected_zone_geojson(workspace, selection)?)?;
    set_source_data(
        map,
        "nodes",
        nodes_geojson(workspace, selection, edge_source_id, hovered_node_id)?,
    )?;
    set_source_data(map, "edges", edges_geojson(workspace, selection, hovered_edge_id)?)?;
    set_source_data(map, "edge_arrows", edge_arrows_geojson(workspace, selection, hovered_edge_id)?)?;
    set_source_data(
        map,
        "edge_preview",
        edge_preview_geojson(workspace, edge_source_id, preview_mouse_point)?,
    )?;
    set_source_data(map, "reference", reference_geojson(workspace)?)?;
    call_method0(map, "triggerRepaint")?;
    Ok(())
}

fn clear_markers() {
    ACTIVE_MARKERS.with(|cell| {
        for marker in cell.borrow_mut().drain(..) {
            let _ = call_method0(&marker, "remove");
        }
    });
}

fn push_marker(marker: JsValue) {
    ACTIVE_MARKERS.with(|cell| {
        cell.borrow_mut().push(marker);
    });
}

fn create_marker(element: &JsValue, draggable: bool) -> Result<JsValue, JsValue> {
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &JsValue::from_str("element"), element)?;
    js_sys::Reflect::set(&options, &JsValue::from_str("draggable"), &JsValue::from_bool(draggable))?;
    let global = js_sys::global();
    let maplibre = js_sys::Reflect::get(&global, &JsValue::from_str("maplibregl"))?;
    let ctor = js_sys::Reflect::get(&maplibre, &JsValue::from_str("Marker"))?
        .dyn_into::<js_sys::Function>()?;
    let args = js_sys::Array::new();
    args.push(&options);
    js_sys::Reflect::construct(&ctor, &args)
}

fn set_marker_lng_lat(marker: &JsValue, point: &JsonPoint) -> Result<(), JsValue> {
    let lng_lat = js_sys::Array::new();
    lng_lat.push(&JsValue::from_f64(point.lon));
    lng_lat.push(&JsValue::from_f64(point.lat));
    call_method1(marker, "setLngLat", &lng_lat.into())?;
    Ok(())
}

fn add_marker_to_map(marker: &JsValue, map: &JsValue) -> Result<(), JsValue> {
    call_method1(marker, "addTo", map)?;
    Ok(())
}

fn marker_lng_lat(marker: &JsValue) -> Option<JsonPoint> {
    let lng_lat = call_method0(marker, "getLngLat").ok()?;
    let lon = js_sys::Reflect::get(&lng_lat, &JsValue::from_str("lng")).ok()?.as_f64()?;
    let lat = js_sys::Reflect::get(&lng_lat, &JsValue::from_str("lat")).ok()?.as_f64()?;
    Some(JsonPoint { lat, lon })
}

fn create_handle(class_name: &str) -> Result<HtmlElement, JsValue> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("document unavailable"))?;
    let handle = document.create_element("button")?.dyn_into::<HtmlElement>()?;
    handle.set_attribute("type", "button")?;
    handle.set_class_name(class_name);
    Ok(handle)
}

fn create_robot_marker(name: &str, yaw_rad: Option<f64>) -> Result<HtmlElement, JsValue> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("document unavailable"))?;
    let root = document.create_element("div")?.dyn_into::<HtmlElement>()?;
    root.set_class_name("robot-marker");

    let arrow = document.create_element("div")?.dyn_into::<HtmlElement>()?;
    arrow.set_class_name("robot-arrow");
    let angle = yaw_rad.unwrap_or(0.0).to_degrees();
    arrow.style().set_property("transform", &format!("rotate({angle}deg)"))?;
    root.append_child(&arrow)?;

    let label = document.create_element("div")?.dyn_into::<HtmlElement>()?;
    label.set_class_name("robot-label");
    label.set_text_content(Some(name));
    root.append_child(&label)?;

    Ok(root)
}

fn set_inline_style(element: &HtmlElement, name: &str, value: &str) -> Result<(), JsValue> {
    element.style().set_property(name, value)
}

fn robot_point(workspace: &WorkspaceJson, robot: &RobotSummary) -> Option<JsonPoint> {
    if let (Some(lat), Some(lon)) = (robot.gnss_lat, robot.gnss_lon) {
        return Some(JsonPoint { lat, lon });
    }

    if workspace.ref_point.is_some() {
        if let (Some(x), Some(y)) = (robot.odom_x, robot.odom_y) {
            return Some(from_local_xy(workspace, x, y));
        }
    }

    None
}

fn render_robot_markers(map: &JsValue, workspace: &WorkspaceJson, robots: &[RobotSummary]) -> Result<(), JsValue> {
    clear_markers();

    for robot in robots {
        let Some(point) = robot_point(workspace, robot) else {
            continue;
        };

        let element = create_robot_marker(&robot.name, robot.yaw_rad)?;
        set_inline_style(&element, "--robot-color", &robot.color)?;
        if is_robot_stale(robot, now_ms()) {
            let _ = element.class_list().add_1("is-stale");
        }
        let marker = create_marker(&element.into(), false)?;
        set_marker_lng_lat(&marker, &point)?;
        add_marker_to_map(&marker, map)?;
        push_marker(marker);
    }

    Ok(())
}

fn render_markers(
    map: &JsValue,
    workspace: RwSignal<WorkspaceJson>,
    selection: RwSignal<Selection>,
    edge_source_id: RwSignal<String>,
    hovered_edge_id: RwSignal<String>,
    raw_json: RwSignal<String>,
    active_right_panel: RwSignal<Option<RightPanel>>,
    status: RwSignal<String>,
    marker_dragging: RwSignal<bool>,
) -> Result<(), JsValue> {
    clear_markers();
    let ws = workspace.get();
    let selected = selection.get();

    if let Selection::Node(node_id) = selected.clone() {
        if let Some(node) = ws.nodes.get(&node_id).cloned() {
            let handle = create_handle("map-handle node-handle")?;
            let marker = create_marker(handle.as_ref(), true)?;
            set_marker_lng_lat(&marker, &node.latlon)?;
            add_marker_to_map(&marker, map)?;

            {
                let workspace = workspace;
                let selection = selection;
                let status = status;
                let raw_json = raw_json;
                let node_id = node_id.clone();
                let context = Closure::<dyn FnMut(MouseEvent)>::wrap(Box::new(move |event: MouseEvent| {
                    event.prevent_default();
                    event.stop_propagation();
                    remove_node_by_id(workspace, selection, status, raw_json, &node_id);
                }));
                handle.add_event_listener_with_callback("contextmenu", context.as_ref().unchecked_ref())?;
                context.forget();
            }

            attach_object_event(&marker, "dragstart", {
                move |_| {
                    marker_dragging.set(true);
                }
            })?;
            attach_object_event(&marker, "drag", {
                let map = map.clone();
                let marker_for_drag = marker.clone();
                move |_| {
                    let Some(point) = marker_lng_lat(&marker_for_drag) else {
                        return;
                    };
                    workspace.update(|ws| {
                        if let Some(node) = ws.nodes.get_mut(&node_id) {
                            node.latlon = point;
                        }
                    });
                    sync_associations(&workspace);
                    let _ = update_map_sources(
                        &map,
                        &workspace.get(),
                        &selection.get(),
                        &edge_source_id.get(),
                        "",
                        &hovered_edge_id.get(),
                        None,
                        &[],
                    );
                }
            })?;
            attach_object_event(&marker, "dragend", {
                move |_| {
                    marker_dragging.set(false);
                    sync_workspace_state(workspace, raw_json);
                    status.set("Moved node.".into());
                    active_right_panel.set(Some(RightPanel::Details));
                }
            })?;
            push_marker(marker);
        }
    }

    if let Selection::Zone(zone_id) = selected {
        if let Some(zone) = ws.zones.get(&zone_id).cloned() {
            for (index, point) in zone.polygon_latlon.iter().cloned().enumerate() {
                let handle = create_handle("map-handle vertex-handle")?;
                let marker = create_marker(handle.as_ref(), true)?;
                set_marker_lng_lat(&marker, &point)?;
                add_marker_to_map(&marker, map)?;

                {
                    let workspace = workspace;
                    let selection = selection;
                    let status = status;
                    let raw_json = raw_json;
                    let zone_id = zone_id.clone();
                    let context = Closure::<dyn FnMut(MouseEvent)>::wrap(Box::new(move |event: MouseEvent| {
                        event.prevent_default();
                        event.stop_propagation();
                        remove_zone_vertex(workspace, selection, status, raw_json, &zone_id, index);
                    }));
                    handle.add_event_listener_with_callback("contextmenu", context.as_ref().unchecked_ref())?;
                    context.forget();
                }

                attach_object_event(&marker, "dragstart", {
                    move |_| {
                        marker_dragging.set(true);
                    }
                })?;
                attach_object_event(&marker, "drag", {
                    let map = map.clone();
                    let zone_id = zone_id.clone();
                    let marker_for_drag = marker.clone();
                    move |_| {
                        let Some(next) = marker_lng_lat(&marker_for_drag) else {
                            return;
                        };
                        workspace.update(|ws| {
                            if let Some(zone) = ws.zones.get_mut(&zone_id) {
                                if index < zone.polygon_latlon.len() {
                                    zone.polygon_latlon[index] = next;
                                }
                            }
                        });
                        sync_associations(&workspace);
                        let _ = update_map_sources(
                            &map,
                            &workspace.get(),
                            &selection.get(),
                            &edge_source_id.get(),
                            "",
                            &hovered_edge_id.get(),
                            None,
                            &[],
                        );
                    }
                })?;
                attach_object_event(&marker, "dragend", {
                    move |_| {
                        marker_dragging.set(false);
                        sync_workspace_state(workspace, raw_json);
                        status.set("Adjusted zone boundary.".into());
                        active_right_panel.set(Some(RightPanel::Details));
                    }
                })?;
                push_marker(marker);
            }

            let center = centroid(&zone.polygon_latlon);
            let center_marker = create_marker(create_handle("map-handle zone-handle")?.as_ref(), true)?;
            set_marker_lng_lat(&center_marker, &center)?;
            add_marker_to_map(&center_marker, map)?;
            let previous = std::rc::Rc::new(RefCell::new(center));

            attach_object_event(&center_marker, "dragstart", {
                let previous = previous.clone();
                let marker_for_dragstart = center_marker.clone();
                move |_| {
                    marker_dragging.set(true);
                    if let Some(point) = marker_lng_lat(&marker_for_dragstart) {
                        *previous.borrow_mut() = point;
                    }
                }
            })?;
            attach_object_event(&center_marker, "drag", {
                let map = map.clone();
                let previous = previous.clone();
                let zone_id = zone_id.clone();
                let marker_for_drag = center_marker.clone();
                move |_| {
                    let Some(next) = marker_lng_lat(&marker_for_drag) else {
                        return;
                    };
                    let prev = previous.borrow().clone();
                    let delta_lat = next.lat - prev.lat;
                    let delta_lon = next.lon - prev.lon;
                    workspace.update(|ws| {
                        if let Some(zone) = ws.zones.get_mut(&zone_id) {
                            zone.polygon_latlon = zone
                                .polygon_latlon
                                .iter()
                                .map(|point| JsonPoint {
                                    lat: point.lat + delta_lat,
                                    lon: point.lon + delta_lon,
                                })
                                .collect();
                        }
                    });
                    *previous.borrow_mut() = next;
                    sync_associations(&workspace);
                    let _ = update_map_sources(
                        &map,
                        &workspace.get(),
                        &selection.get(),
                        &edge_source_id.get(),
                        "",
                        &hovered_edge_id.get(),
                        None,
                        &[],
                    );
                }
            })?;
            attach_object_event(&center_marker, "dragend", {
                move |_| {
                    marker_dragging.set(false);
                    sync_workspace_state(workspace, raw_json);
                    status.set("Moved zone.".into());
                    active_right_panel.set(Some(RightPanel::Details));
                }
            })?;
            push_marker(center_marker);
        }
    }

    Ok(())
}

fn get_map_source(map: &JsValue, id: &str) -> Option<JsValue> {
    call_method1(map, "getSource", &JsValue::from_str(id)).ok().filter(|v| !v.is_undefined() && !v.is_null())
}

fn add_geojson_source(map: &JsValue, id: &str) -> Result<(), JsValue> {
    let source = js_sys::Object::new();
    js_sys::Reflect::set(&source, &JsValue::from_str("type"), &JsValue::from_str("geojson"))?;
    js_sys::Reflect::set(&source, &JsValue::from_str("data"), &empty_feature_collection()?)?;
    call_method2(map, "addSource", &JsValue::from_str(id), &source)?;
    Ok(())
}

fn set_source_data(map: &JsValue, id: &str, data: JsValue) -> Result<(), JsValue> {
    if let Some(source) = get_map_source(map, id) {
        call_method1(&source, "setData", &data)?;
    }
    Ok(())
}

fn add_layer(map: &JsValue, layer: JsValue) -> Result<(), JsValue> {
    call_method1(map, "addLayer", &layer)?;
    Ok(())
}

fn call_method0(target: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    let func = js_sys::Reflect::get(target, &JsValue::from_str(name))?.dyn_into::<js_sys::Function>()?;
    func.call0(target)
}

fn call_method1(target: &JsValue, name: &str, a: &JsValue) -> Result<JsValue, JsValue> {
    let func = js_sys::Reflect::get(target, &JsValue::from_str(name))?.dyn_into::<js_sys::Function>()?;
    func.call1(target, a)
}

fn call_method2(target: &JsValue, name: &str, a: &JsValue, b: &JsValue) -> Result<JsValue, JsValue> {
    let func = js_sys::Reflect::get(target, &JsValue::from_str(name))?.dyn_into::<js_sys::Function>()?;
    func.call2(target, a, b)
}

fn empty_feature_collection() -> Result<JsValue, JsValue> {
    js_sys::JSON::parse(r#"{"type":"FeatureCollection","features":[]}"#)
}

fn zones_geojson(workspace: &WorkspaceJson, selection: &Selection) -> Result<JsValue, JsValue> {
    let features = workspace
        .zones
        .values()
        .filter(|zone| !matches!(selection, Selection::Zone(id) if id == &zone.id))
        .map(|zone| {
            let mut ring = zone
                .polygon_latlon
                .iter()
                .map(|p| vec![p.lon, p.lat])
                .collect::<Vec<_>>();
            if let Some(first) = ring.first().cloned() {
                if ring.last() != Some(&first) {
                    ring.push(first);
                }
            }
            serde_json::json!({
                "type": "Feature",
                "properties": {
                    "id": zone.id,
                    "selected": false,
                    "root": zone.id == workspace.root_zone_id,
                },
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [ring]
                }
            })
        })
        .collect::<Vec<_>>();
    json_to_js(serde_json::json!({ "type": "FeatureCollection", "features": features }))
}

fn selected_zone_geojson(workspace: &WorkspaceJson, selection: &Selection) -> Result<JsValue, JsValue> {
    let features = match selection {
        Selection::Zone(zone_id) => workspace.zones.get(zone_id).map(|zone| {
            let mut ring = zone.polygon_latlon.iter().map(|p| vec![p.lon, p.lat]).collect::<Vec<_>>();
            if let Some(first) = ring.first().cloned() {
                if ring.last() != Some(&first) {
                    ring.push(first);
                }
            }
            vec![serde_json::json!({
                "type": "Feature",
                "properties": {
                    "id": zone.id,
                    "name": zone.name,
                    "selected": true,
                    "root": zone.id == workspace.root_zone_id,
                },
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [ring]
                }
            })]
        }).unwrap_or_default(),
        _ => Vec::new(),
    };
    json_to_js(serde_json::json!({ "type": "FeatureCollection", "features": features }))
}

fn zone_draft_geojson(points: &[JsonPoint]) -> Result<JsValue, JsValue> {
    let mut features = Vec::new();
    if points.len() >= 2 {
        features.push(serde_json::json!({
            "type": "Feature",
            "properties": { "kind": "line" },
            "geometry": {
                "type": "LineString",
                "coordinates": points.iter().map(|p| vec![p.lon, p.lat]).collect::<Vec<_>>()
            }
        }));
    }
    if points.len() >= 3 {
        let mut ring = points.iter().map(|p| vec![p.lon, p.lat]).collect::<Vec<_>>();
        if let Some(first) = ring.first().cloned() {
            ring.push(first);
        }
        features.push(serde_json::json!({
            "type": "Feature",
            "properties": { "kind": "fill" },
            "geometry": {
                "type": "Polygon",
                "coordinates": [ring]
            }
        }));
    }
    for (index, point) in points.iter().enumerate() {
        features.push(serde_json::json!({
            "type": "Feature",
            "properties": {
                "kind": "point",
                "index": index,
            },
            "geometry": {
                "type": "Point",
                "coordinates": [point.lon, point.lat]
            }
        }));
    }
    json_to_js(serde_json::json!({ "type": "FeatureCollection", "features": features }))
}

fn nodes_geojson(
    workspace: &WorkspaceJson,
    selection: &Selection,
    edge_source_id: &str,
    hovered_node_id: &str,
) -> Result<JsValue, JsValue> {
    let features = workspace.nodes.values().map(|node| {
        serde_json::json!({
            "type": "Feature",
            "properties": {
                "id": node.id,
                "selected": matches!(selection, Selection::Node(id) if id == &node.id),
                "edgeSource": edge_source_id == node.id,
                "hovered": hovered_node_id == node.id,
            },
            "geometry": {
                "type": "Point",
                "coordinates": [node.latlon.lon, node.latlon.lat]
            }
        })
    }).collect::<Vec<_>>();
    json_to_js(serde_json::json!({ "type": "FeatureCollection", "features": features }))
}

fn edge_preview_geojson(
    workspace: &WorkspaceJson,
    edge_source_id: &str,
    preview_mouse_point: Option<&JsonPoint>,
) -> Result<JsValue, JsValue> {
    let features = match (workspace.nodes.get(edge_source_id), preview_mouse_point) {
        (Some(source), Some(target)) => vec![serde_json::json!({
            "type": "Feature",
            "properties": { "id": "edge-preview" },
            "geometry": {
                "type": "LineString",
                "coordinates": [
                    [source.latlon.lon, source.latlon.lat],
                    [target.lon, target.lat]
                ]
            }
        })],
        _ => Vec::new(),
    };
    json_to_js(serde_json::json!({ "type": "FeatureCollection", "features": features }))
}

fn edges_geojson(workspace: &WorkspaceJson, selection: &Selection, hovered_edge_id: &str) -> Result<JsValue, JsValue> {
    let features = workspace.edges.values().filter_map(|edge| {
        let source = workspace.nodes.get(&edge.source_id)?;
        let target = workspace.nodes.get(&edge.target_id)?;
        let is_selected = matches!(selection, Selection::Edge(id) if id == &edge.id);
        let is_hovered = hovered_edge_id == edge.id;
        if edge.directed {
            return Some(vec![serde_json::json!({
                "type": "Feature",
                "properties": {
                    "id": edge.id,
                    "directed": true,
                    "selected": is_selected,
                    "hovered": is_hovered,
                    "laneOffset": 0,
                },
                "geometry": {
                    "type": "LineString",
                    "coordinates": [[source.latlon.lon, source.latlon.lat], [target.latlon.lon, target.latlon.lat]]
                }
            })]);
        }
        Some(vec![
            serde_json::json!({
                "type": "Feature",
                "properties": {
                    "id": edge.id,
                    "directed": false,
                    "selected": is_selected,
                    "hovered": is_hovered,
                    "laneOffset": 3,
                },
                "geometry": {
                    "type": "LineString",
                    "coordinates": [[source.latlon.lon, source.latlon.lat], [target.latlon.lon, target.latlon.lat]]
                }
            }),
            serde_json::json!({
                "type": "Feature",
                "properties": {
                    "id": edge.id,
                    "directed": false,
                    "selected": is_selected,
                    "hovered": is_hovered,
                    "laneOffset": 3,
                },
                "geometry": {
                    "type": "LineString",
                    "coordinates": [[target.latlon.lon, target.latlon.lat], [source.latlon.lon, source.latlon.lat]]
                }
            })
        ])
    }).flatten().collect::<Vec<_>>();
    json_to_js(serde_json::json!({ "type": "FeatureCollection", "features": features }))
}

fn edge_arrows_geojson(workspace: &WorkspaceJson, selection: &Selection, hovered_edge_id: &str) -> Result<JsValue, JsValue> {
    let features = workspace
        .edges
        .values()
        .filter(|edge| edge.directed)
        .filter_map(|edge| {
            let source = workspace.nodes.get(&edge.source_id)?;
            let target = workspace.nodes.get(&edge.target_id)?;
            let anchor = point_along_segment(&source.latlon, &target.latlon, 0.68);
            let tip = offset_from_direction(&anchor, &source.latlon, &target.latlon, 0.525, 0.0);
            let left = offset_from_direction(&anchor, &source.latlon, &target.latlon, -0.2625, 0.28125);
            let right = offset_from_direction(&anchor, &source.latlon, &target.latlon, -0.2625, -0.28125);
            Some(serde_json::json!({
                "type": "Feature",
                "properties": {
                    "id": edge.id,
                    "selected": matches!(selection, Selection::Edge(id) if id == &edge.id),
                    "hovered": hovered_edge_id == edge.id,
                },
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[
                        [tip.lon, tip.lat],
                        [left.lon, left.lat],
                        [right.lon, right.lat],
                        [tip.lon, tip.lat]
                    ]]
                }
            }))
        })
        .collect::<Vec<_>>();
    json_to_js(serde_json::json!({ "type": "FeatureCollection", "features": features }))
}

fn reference_geojson(workspace: &WorkspaceJson) -> Result<JsValue, JsValue> {
    let features = workspace.ref_point.as_ref().map(|point| {
        vec![serde_json::json!({
            "type": "Feature",
            "properties": { "id": "workspace-ref" },
            "geometry": {
                "type": "Point",
                "coordinates": [point.lon, point.lat]
            }
        })]
    }).unwrap_or_default();
    json_to_js(serde_json::json!({ "type": "FeatureCollection", "features": features }))
}

fn json_to_js(value: serde_json::Value) -> Result<JsValue, JsValue> {
    js_sys::JSON::parse(&value.to_string())
}

fn zone_fill_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "zone-fill",
        "type": "fill",
        "source": "zones",
        "paint": {
            "fill-color": ["case", ["==", ["get", "selected"], true], "#e67f4e", ["==", ["get", "root"], true], "#295d6b", "#6f9299"],
            "fill-opacity": ["case", ["==", ["get", "selected"], true], 0.34, 0.16]
        }
    }))
}

fn zone_line_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "zone-line",
        "type": "line",
        "source": "zones",
        "paint": {
            "line-color": ["case", ["==", ["get", "selected"], true], "#bb552d", "#17333b"],
            "line-width": ["case", ["==", ["get", "selected"], true], 4, 2]
        }
    }))
}

fn zone_hit_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "zone-hit",
        "type": "line",
        "source": "zones",
        "paint": {
            "line-color": "rgba(0,0,0,0)",
            "line-width": 14
        }
    }))
}

fn zone_draft_fill_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "zone-draft-fill",
        "type": "fill",
        "source": "zone_draft",
        "filter": ["==", ["geometry-type"], "Polygon"],
        "paint": {
            "fill-color": "#e07a4d",
            "fill-opacity": 0.16
        }
    }))
}

fn zone_draft_line_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "zone-draft-line",
        "type": "line",
        "source": "zone_draft",
        "filter": ["==", ["geometry-type"], "LineString"],
        "paint": {
            "line-color": "#f1a07a",
            "line-width": 3,
            "line-dasharray": [1.2, 1.2]
        }
    }))
}

fn zone_draft_point_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "zone-draft-point",
        "type": "circle",
        "source": "zone_draft",
        "filter": ["==", ["geometry-type"], "Point"],
        "paint": {
            "circle-radius": 5,
            "circle-color": "#f6f0e7",
            "circle-stroke-color": "#e07a4d",
            "circle-stroke-width": 2
        }
    }))
}

fn selected_zone_fill_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "selected-zone-fill",
        "type": "fill",
        "source": "selected_zone",
        "paint": {
            "fill-color": "#e67f4e",
            "fill-opacity": 0.34
        }
    }))
}

fn selected_zone_line_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "selected-zone-line",
        "type": "line",
        "source": "selected_zone",
        "paint": {
            "line-color": "#bb552d",
            "line-width": 4
        }
    }))
}

fn selected_zone_hit_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "selected-zone-hit",
        "type": "line",
        "source": "selected_zone",
        "paint": {
            "line-color": "rgba(0,0,0,0)",
            "line-width": 14
        }
    }))
}

fn edge_line_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "edge-line",
        "type": "line",
        "source": "edges",
        "paint": {
            "line-color": [
                "case",
                ["==", ["get", "selected"], true],
                "#dd5b2d",
                ["==", ["get", "hovered"], true],
                "#e18c63",
                "#13262e"
            ],
            "line-width": [
                "case",
                ["==", ["get", "selected"], true],
                4.5,
                ["==", ["get", "hovered"], true],
                3.5,
                2.5
            ],
            "line-offset": ["get", "laneOffset"]
        }
    }))
}

fn edge_arrow_fill_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "edge-arrow-fill",
        "type": "fill",
        "source": "edge_arrows",
        "paint": {
            "fill-color": [
                "case",
                ["==", ["get", "selected"], true],
                "#f1a07a",
                ["==", ["get", "hovered"], true],
                "#d77d55",
                "#2d4d57"
            ],
            "fill-opacity": 0.98
        }
    }))
}

fn edge_arrow_line_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "edge-arrow-line",
        "type": "line",
        "source": "edge_arrows",
        "paint": {
            "line-color": "#f6f0e7",
            "line-width": 1.8,
            "line-opacity": 0.96
        }
    }))
}

fn edge_hit_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "edge-hit",
        "type": "line",
        "source": "edges",
        "layout": {
            "line-cap": "butt",
            "line-join": "round"
        },
        "paint": {
            "line-color": "rgba(0,0,0,0)",
            "line-width": 16,
            "line-offset": ["get", "laneOffset"]
        }
    }))
}

fn edge_preview_line_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "edge-preview-line",
        "type": "line",
        "source": "edge_preview",
        "paint": {
            "line-color": "#ee7d46",
            "line-width": 3,
            "line-dasharray": [1.2, 1.0],
            "line-opacity": 0.9
        }
    }))
}

fn node_circle_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "node-circle",
        "type": "circle",
        "source": "nodes",
        "paint": {
            "circle-radius": [
                "case",
                ["==", ["get", "selected"], true],
                9,
                ["==", ["get", "hovered"], true],
                8.5,
                ["==", ["get", "edgeSource"], true],
                8,
                6
            ],
            "circle-color": [
                "case",
                ["==", ["get", "selected"], true],
                "#ee7d46",
                ["==", ["get", "hovered"], true],
                "#f09565",
                ["==", ["get", "edgeSource"], true],
                "#215f79",
                "#152128"
            ],
            "circle-stroke-color": "#f6f0e7",
            "circle-stroke-width": 2
        }
    }))
}

fn node_hit_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "node-hit",
        "type": "circle",
        "source": "nodes",
        "paint": {
            "circle-radius": 14,
            "circle-color": "rgba(0,0,0,0)"
        }
    }))
}

fn reference_circle_layer() -> Result<JsValue, JsValue> {
    json_to_js(serde_json::json!({
        "id": "reference-point",
        "type": "circle",
        "source": "reference",
        "paint": {
            "circle-radius": 8,
            "circle-color": "#d94141",
            "circle-stroke-color": "#fff1f1",
            "circle-stroke-width": 2
        }
    }))
}

fn focus_map_point(map: &JsValue, point: &JsonPoint, zoom: f64) -> Result<(), JsValue> {
    let options = js_sys::Object::new();
    let center = js_sys::Array::new();
    center.push(&JsValue::from_f64(point.lon));
    center.push(&JsValue::from_f64(point.lat));
    js_sys::Reflect::set(&options, &JsValue::from_str("center"), &center)?;
    js_sys::Reflect::set(&options, &JsValue::from_str("zoom"), &JsValue::from_f64(zoom))?;
    let _ = call_method1(map, "easeTo", &options.into())?;
    Ok(())
}

fn focus_map_zone(map: &JsValue, zone: &ZoneJson) -> Result<(), JsValue> {
    if zone.polygon_latlon.is_empty() {
        return Ok(());
    }
    let Some((min_lat, max_lat, min_lon, max_lon)) = polygon_bounds(&zone.polygon_latlon) else {
        return focus_map_point(map, &centroid(&zone.polygon_latlon), 17.0);
    };

    let lat_span = (max_lat - min_lat).abs();
    let lon_span = (max_lon - min_lon).abs();
    if lat_span <= 1e-7 && lon_span <= 1e-7 {
        return focus_map_point(map, &centroid(&zone.polygon_latlon), 17.0);
    }

    let sw = js_sys::Array::new();
    sw.push(&JsValue::from_f64(min_lon));
    sw.push(&JsValue::from_f64(min_lat));
    let ne = js_sys::Array::new();
    ne.push(&JsValue::from_f64(max_lon));
    ne.push(&JsValue::from_f64(max_lat));
    let bounds = js_sys::Array::new();
    bounds.push(&sw.into());
    bounds.push(&ne.into());

    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &JsValue::from_str("padding"), &JsValue::from_f64(96.0))?;
    js_sys::Reflect::set(&options, &JsValue::from_str("duration"), &JsValue::from_f64(700.0))?;
    js_sys::Reflect::set(&options, &JsValue::from_str("maxZoom"), &JsValue::from_f64(18.0))?;
    let _ = call_method2(map, "fitBounds", &bounds.into(), &options.into())?;
    Ok(())
}

fn focus_map_node(map: &JsValue, node: &NodeJson) -> Result<(), JsValue> {
    focus_map_point(map, &node.latlon, 16.5)
}

fn focus_map_edge(map: &JsValue, workspace: &WorkspaceJson, edge: &EdgeJson) -> Result<(), JsValue> {
    let Some(source) = workspace.nodes.get(&edge.source_id) else {
        return Ok(());
    };
    let Some(target) = workspace.nodes.get(&edge.target_id) else {
        return focus_map_node(map, source);
    };

    let min_lat = source.latlon.lat.min(target.latlon.lat);
    let max_lat = source.latlon.lat.max(target.latlon.lat);
    let min_lon = source.latlon.lon.min(target.latlon.lon);
    let max_lon = source.latlon.lon.max(target.latlon.lon);

    let lat_span = (max_lat - min_lat).abs();
    let lon_span = (max_lon - min_lon).abs();
    if lat_span <= 1e-7 && lon_span <= 1e-7 {
        return focus_map_point(map, &midpoint(&source.latlon, &target.latlon), 16.5);
    }

    let sw = js_sys::Array::new();
    sw.push(&JsValue::from_f64(min_lon));
    sw.push(&JsValue::from_f64(min_lat));
    let ne = js_sys::Array::new();
    ne.push(&JsValue::from_f64(max_lon));
    ne.push(&JsValue::from_f64(max_lat));
    let bounds = js_sys::Array::new();
    bounds.push(&sw.into());
    bounds.push(&ne.into());

    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &JsValue::from_str("padding"), &JsValue::from_f64(112.0))?;
    js_sys::Reflect::set(&options, &JsValue::from_str("duration"), &JsValue::from_f64(700.0))?;
    js_sys::Reflect::set(&options, &JsValue::from_str("maxZoom"), &JsValue::from_f64(16.5))?;
    let _ = call_method2(map, "fitBounds", &bounds.into(), &options.into())?;
    Ok(())
}
