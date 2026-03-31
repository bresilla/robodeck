# robodeck

`robodeck` is an editor for `workspace.json`.

`workspace.json` is the portable wire format for the shared workspace model used by `zoneout` and consumed by higher-level systems such as `timenav`.

## Workspace Model

A workspace is one top-level object containing:

- `name`
- `root_zone_id`
- `coord_mode`
- `ref`
- `datum`
- `zones`
- `nodes`
- `edges`

The model combines two parallel structures:

- a zone hierarchy
- a navigation graph

## Coordinates

`coord_mode` is either:

- `global`
- `local`

`ref` is an optional global anchor. `datum` is optional workspace reference data.

In global mode, geometry is represented in lat/lon. In local mode, the workspace is interpreted relative to the chosen local reference frame.

## Zones

`zones` is a map of zone id to zone object.

A zone contains:

- `id`
- `name`
- `type`
- `parent_id`
- `child_ids`
- `node_ids`
- `polygon_latlon`
- `grid_enabled`
- `grid_resolution`
- `properties`

Zones are hierarchical. The root zone contains the whole site. Child zones provide nested semantic structure.

`polygon_latlon` stores the zone boundary. `properties` is free-form key/value metadata attached to the zone.

## Nodes

`nodes` is a map of node id to node object.

A node contains:

- `id`
- `name`
- `latlon`
- `zone_ids`
- `properties`

Nodes are the graph vertices in the workspace model.

## Edges

`edges` is a map of edge id to edge object.

An edge contains:

- `id`
- `source_id`
- `target_id`
- `directed`
- `weight`
- `zone_ids`
- `properties`

Edges connect nodes and define graph connectivity.

## Zone And Graph Relationship

Zones define semantic regions. Nodes and edges define connectivity.

Membership is represented explicitly:

- zones reference nodes through `node_ids`
- nodes reference zones through `zone_ids`
- edges can also reference zones through `zone_ids`

This allows the same graph structure to be associated with the semantic zone hierarchy without collapsing both concerns into one data structure.

## What robodeck Edits

`robodeck` edits:

- workspace metadata
- coordinate/reference fields
- zone hierarchy
- zone geometry
- graph nodes
- graph edges
- free-form properties
- raw `workspace.json`

## Run

Build the frontend and run the single server binary:

```bash
make serve
```

Or, if you want the explicit two-step form:

```bash
trunk build
cargo run --bin robodeck
```

Then open:

```text
http://127.0.0.1:38080
```

The single Rust server serves the built frontend from `dist/` and also exposes
the Zenoh API at `/api/zenoh`. The browser talks to the same process that owns
the Zenoh session, so runtime is one binary instead of a separate frontend and
backend server.

If you still want the old frontend-only dev server, use:

```bash
make serve-ui
```
