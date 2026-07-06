# Tribes: Ascend Bevy Port — Architecture & Implementation Plan

## Overview

Port Tribes: Ascend (UDK/UE3) to Bevy (Rust) using decompiled game logic (3,717 `.uc` classes) and extracted assets (textures, meshes, sounds, maps, game objects). Three-tier architecture: realm broker, server-authoritative game servers, and clients. Server-authoritative netcode with client-side prediction and rollback. Replays supported via deterministic input recording.

---

## Topology

```
┌─────────────────────┐         gRPC (HTTP/2 + TLS)         ┌─────────────────────┐
│   Realm Server      │◀───────────────────────────────────│  Game Server         │
│  (tonic + sqlx)     │                                     │  (Bevy headless)    │
│  SQLite, no Bevy    │   RegisterServer, Heartbeat,        │                     │
│                     │   SubmitMatchResult (+replay blob)  │  Registers on boot  │
│  - User auth        │                                     │  Heartbeats to realm│
│  - Server directory │                                     │  Validates tokens   │
│  - Matchmaking      │                                     │  Records replays    │
│  - Match history    │                                     │                     │
│  - Replay storage   │                                     │                     │
└─────────▲───────────┘                                     └──────────▲──────────┘
          │                                                            │
          │ gRPC                                              UDP (lightyear)
          │                                                            │
┌─────────┴───────────┐                                     ┌──────────┴──────────┐
│   Client             │◀──────────────────────────────────│  Players             │
│  (full Bevy)         │           UDP (lightyear)          │                      │
│                      │                                    │                      │
│  2 connections:      │                                    │                      │
│  1. Realm (gRPC)    │                                    │                      │
│  2. Game server(UDP)│                                    │                      │
│                      │                                    │                      │
│  Can also play replays│                                   │                      │
│  from SQLite blob   │                                    │                      │
└──────────────────────┘                                     └──────────────────────┘
```

**Two connections per client**, completely decoupled:
1. **Realm connection (gRPC, HTTP/2 + TLS)** — login, server directory, matchmaking queue, post-match stat ingestion, replay retrieval. Idle during gameplay.
2. **Game connection (UDP, lightyear)** — gameplay traffic to one game server. Hot path.

The realm never sees a gameplay packet. Game servers register with the realm over gRPC as a client of the realm.

---

## Dependency Matrix

All verified for Bevy 0.19 compatibility.

| Crate | Version | Role |
|-------|---------|------|
| `bevy` | 0.19 | Core engine |
| `avian3d` | 0.7 | Physics: colliders, spatial queries, `MoveAndSlide` kinematic controller |
| `lightyear` | 0.28 | Netcode: prediction, rollback, lag compensation, Avian integration |
| `leafwing-input-manager` | 0.21 | Input abstraction with `ActionDiff` for networking + replays |
| `bevy_hanabi` | 0.19 | GPU particles (jetpacks, projectiles, explosions) |
| `bevy_kira_audio` | 0.26 | Audio (1,543 OGG sounds) |
| `bevy_egui` | 0.41 | Menus, server browser, scoreboard |
| `bevy_common_assets` | 0.17 | Typed JSON asset loaders |
| `bevy_tweening` | 0.16 | UI animations, flag capture progress |
| `tonic` | 0.13 | gRPC (realm server + client calls) |
| `sqlx` | 0.8 | SQLite (compile-time checked queries) |
| `prost` | 0.13 | Protobuf codegen |
| `argon2` | 0.5 | Password hashing |

### Dropped (incompatible with Bevy 0.19)

- ~~`bevy_mod_raycast`~~ — stuck at Bevy 0.15. **Use Avian's `SpatialQuery::cast_ray`** for hitscan/ground checks. The lightyear `fps` example demonstrates `LagCompensationSpatialQuery` for rewinding hit detection.
- ~~`bevy_mod_billboard`~~ — stuck at Bevy 0.14. **Custom WGSL billboard shader** (~50 lines, or a mesh with camera-facing transform system). Implemented in `crates/client/`.

---

## Reuse from Lightyear Examples

The lightyear repo has examples that are nearly blueprints for our needs:

| Example | What we reuse |
|---------|---------------|
| `fps` | `PreSpawnedPlayerObject` for predicted projectiles, `LagCompensationSpatialQuery` for hitscan |
| `projectiles` | All 5 weapon types (hitscan, linear projectile, shotgun, physics projectile, homing), 3 replication methods (full entity, direction-only, ring buffer), room-based interest management |
| `avian_3d` | Avian + lightyear integration patterns, `enhanced-determinism` setup |
| `deterministic_replication` | Input-only replication (foundation for replays) |
| `auth` | `ConnectToken` flow (realm issues token → client connects to game server) |
| `lobby` | Runtime topology changes (could use for spectate/replay viewer) |

---

## Replay System

Lightyear has no built-in replay recorder, but its architecture makes replays nearly free.

### How it works

Lightyear's `deterministic` feature enables **input-only replication**: the simulation runs deterministically on each peer, and only inputs are sent over the wire. This is exactly what a replay is — a recording of inputs.

### Replay format (in `crates/protocol/`)

```rust
// Replay file format (bincode-serialized)
struct ReplayFile {
    version: u32,
    protocol_hash: u64,        // detects version mismatch on playback
    map_name: String,
    match_id: u64,
    tick_rate: f64,
    player_ids: Vec<PlayerId>,
    initial_snapshot: Vec<u8>,  // serialized world state at tick 0
    inputs: Vec<InputFrame>,    // one per tick
    duration_ticks: u32,
}

struct InputFrame {
    tick: u32,
    inputs: Vec<(PlayerId, InputData)>,  // InputData = leafwing ActionState
}
```

### Recording (game server side)

The game server already receives all player inputs via lightyear's `MessageReceiver`. We add a `ReplayRecorder` resource that:
1. Captures the initial world snapshot at match start (serialize all `Replicate` entities)
2. Appends each tick's inputs to a `Vec<InputFrame>`
3. On match end, serializes to `ReplayFile` and submits to realm via gRPC `SubmitMatchResult` (which accepts an optional replay blob)

### Playback (client side)

A "watch replay" mode that:
1. Loads `ReplayFile`
2. Spawns initial world from `initial_snapshot`
3. Runs the same deterministic simulation, but instead of receiving inputs from the network, feeds them from the `inputs` vector at each tick
4. The client is effectively a spectator connecting to a "ghost server"

This works because:
- `leafwing-input-manager`'s `ActionState` is `Serialize`/`Deserialize`
- Avian's `enhanced-determinism` feature ensures cross-platform deterministic math
- Lightyear's `FixedMain` schedule at 60 Hz provides a fixed tick
- The `physics` + `core` crates are shared between server and client, guaranteeing simulation symmetry

### Replay storage

Replays are small (~100 KB for a 20-min match: 72,000 ticks × ~8 players × ~16 bytes/input). Stored as a `BLOB` in the realm SQLite `match_history` table, or as files on the game server with a URL reference.

---

## Workspace Structure

```
tribes/
├── Cargo.toml                    # [workspace], resolver = "3"
├── proto/                        # .proto files
│   └── realm.proto               # Realm + RealmAdmin services
├── crates/
│   ├── core/                     # Components, config types, weapon definitions
│   │                             # Shared by client + server + replay viewer
│   ├── protocol/                 # Lightyear protocol: MessageRegistry, ChannelRegistry,
│   │                             # ComponentRegistry, ReplayFile format, InputData types
│   ├── assets/                   # AssetLoader impls for JSON (maps, game-objects, manifest)
│   │                             # Uses bevy_common_assets
│   ├── physics/                  # Tribes movement controller (MoveAndSlide),
│   │                             # projectile inheritance, skiing friction, jetpack
│   │                             # Uses avian3d + enhanced-determinism
│   ├── input/                    # leafwing Action enum, InputMap from DefaultInput.ini,
│   │                             # ActionDiff networking (lightyear leafwing feature)
│   ├── replay/                   # ReplayRecorder (server), ReplayPlayer (client),
│   │                             # ReplayFile serde format
│   ├── client/                   # Bevy plugins: rendering, prediction, audio, UI (egui)
│   │                             # lightyear client + avian + hanabi + kira + egui
│   ├── server/                   # Bevy plugins: authority, replication, scoring,
│   │                             # lag comp, match lifecycle, replay recording
│   │                             # lightyear server + avian + tonic client
│   └── realm/                    # tonic gRPC server + sqlx SQLite
│                                 # auth, matchmaking, server directory, replay storage
└── bins/
    ├── client/                    # boots client plugins (or replay player)
    ├── server/                    # boots server plugins + registers with realm
    └── realm/                     # boots tonic gRPC server
```

**Key invariant:** `core` + `physics` + `protocol` are imported by both client and server. This guarantees simulation symmetry, which is non-negotiable for rollback netcode and replays. The `physics` crate contains the Tribes movement constants ported from `TribesPlayerController.uc` / `TribesPawn.uc`.

---

## Realm Server (no Bevy)

**Stack:**
- `tonic` — gRPC server (HTTP/2 + TLS)
- `sqlx` — SQLite (compile-time checked queries, `sqlx::SqlitePool`)
- `prost` + `tonic-build` — protobuf codegen from `.proto` files
- `argon2` — password hashing (user-facing flow is plain, storage is hashed)
- `tokio` — async runtime
- `tracing` — structured logs

### SQLite schema (admin-managed, single file)

- `users(id, username, password_hash, created_at)`
- `sessions(token, user_id, expires_at)`
- `servers(id, addr, region, current_map, player_count, max_players, latency_ms, last_heartbeat, status)`
- `match_history(id, map, server_id, started_at, ended_at, winning_team, replay_blob)`
- `player_stats(user_id, kills, deaths, captures, wins, losses, matches_played)`

---

## gRPC Protocol (`proto/realm.proto`)

```protobuf
syntax = "proto3";
package tribes;

service Realm {
  rpc Register(RegisterRequest) returns (AuthResponse);
  rpc Login(LoginRequest) returns (AuthResponse);
  rpc ListServers(Empty) returns (ServerList);
  rpc Queue(Empty) returns (QueueResponse);
  rpc Dequeue(Empty) returns (Empty);
  rpc Heartbeat(Empty) returns (Empty);
  rpc GetStats(Empty) returns (PlayerStats);
  rpc GetReplay(GetReplayRequest) returns (ReplayBlob);
}

service RealmAdmin {
  rpc RegisterServer(RegisterServerRequest) returns (ServerRegistration);
  rpc UpdateServerStatus(ServerStatus) returns (Empty);
  rpc UnregisterServer(ServerId) returns (Empty);
  rpc ValidateSession(ValidateSessionRequest) returns (ValidateSessionResponse);
  rpc SubmitMatchResult(MatchResult) returns (Empty);
}

message MatchResult {
  uint64 match_id = 1;
  string map = 2;
  uint64 server_id = 3;
  repeated PlayerMatchResult players = 4;
  optional bytes replay_blob = 5;
}

message PlayerMatchResult {
  uint32 user_id = 1;
  uint32 kills = 2;
  uint32 deaths = 3;
  uint32 captures = 4;
  bool won = 5;
}
```

---

## Matchmaking + Server Lifecycle Flow

1. **Game server boots** → gRPC `RegisterServer(addr, region, max_players)` → realm returns `ServerRegistration{id}`
2. **Game server updates** → gRPC `UpdateServerStatus(id, current_map, player_count, latency_ms)` — sent on state changes
3. **Realm heartbeat** → realm pings game server's UDP port every 5 min. No response → delist
4. **Client queues** → gRPC `Queue(session_token)` → realm finds available server → returns `QueueResponse{server_addr, connect_token}`
5. **Client connects** → UDP to game server with `connect_token` → game server calls `ValidateSession` via gRPC → on success, player joins
6. **Match ends** → game server calls `SubmitMatchResult` with per-player stats + optional `replay_blob` → realm stores in SQLite + updates `player_stats`

---

## Physics Approach

**Use Avian's `MoveAndSlide` for the kinematic character controller.** Confirmed API in Avian 0.7 docs.rs:
- `RigidBody::Kinematic` component
- `MoveAndSlide` system parameter for collision-aware movement
- Reference example: `crates/avian3d/examples/kinematic_character_3d`
- `SpatialQuery` for raycasts/shapecasts (grounded check, projectile sweeps, hitscan)
- `enhanced-determinism` feature enabled for cross-arch consistency (required for lightyear rollback + replays)

**Custom Tribes movement layer on top:**
- `Skiing` component: while grounded + skiing input held, friction → 0
- `Jetpack` component: thrust along view vector + gravity modifier, energy pool
- `AirControl` system: custom acceleration curve ported from `TribesPlayerController.uc`
- Velocity inheritance for projectiles: `ProjectileSpawn` event captures full firing-state snapshot

Avian handles: collision shapes, raycasts, capsule sweeps, terrain/static mesh colliders. We hand-roll: integrator for the player, projectile motion, jetpack energy, skiing friction state machine.

---

## Netcode Design

- **Channels (lightyear):**
  - `Input` channel (reliable-ordered, client→server): leafwing input actions
  - `Replication` channel (unreliable, server→client): entity state with delta compression
  - `Events` channel (reliable-ordered, bidirectional): flag grabs, captures, kills, damage, projectile spawns
- **Prediction:** player-controlled pawn is predicted with rollback. Other players' pawns are interpolated.
- **Lag compensation:** enabled for hitscan weapons. Server stores historical snapshots for ~200ms.
- **Tickrate:** 60 Hz server, 60 Hz client prediction, render at display rate. Original `.uc` constants tuned for 30 Hz; refactor them as dt-independent acceleration/decay rates.

---

## Key Design Decisions

### Input: `leafwing-input-manager` over `bevy_enhanced_input`

Both work with lightyear. Choosing leafwing because:
- `ActionDiff` — compact network representation (only sends input *changes*, not full state)
- More mature (943★, 588 commits, 39 releases)
- `ActionState` is `Serialize`/`Deserialize` — directly usable in replays
- Lightyear's `leafwing` feature handles networking transparently

### Hitscan: Avian's `SpatialQuery` (no `bevy_mod_raycast`)

`bevy_mod_raycast` is stuck at Bevy 0.15. Avian 0.7 has `SpatialQuery::cast_ray` with collision filters, which covers all our needs:
- Sniper/laser hitscan: ray cast against player capsule colliders
- Grounded check for skiing: downward raycast
- Aim trace for UI: raycast from camera

The lightyear `fps` example already demonstrates `LagCompensationSpatialQuery` — Avian + lightyear's lag comp integration for rewinding hit detection.

### Billboards: custom (no `bevy_mod_billboard`)

`bevy_mod_billboard` is stuck at Bevy 0.14. A billboard is a mesh with a custom material that counters camera rotation in the vertex shader. ~50 lines of WGSL. Implemented in `crates/client/`.

### Replays: deterministic input recording

Replays leverage the fact that lightyear's `deterministic` feature + Avian's `enhanced-determinism` + leafwing's serializable `ActionState` = a fully deterministic simulation driven by inputs. Recording inputs + initial state = a complete replay. No extra infrastructure needed.

### Map scale

`f32` everywhere. No origin rebasing initially. Tribes maps are ~2 km across; precision is fine at that range with single-precision floats. Revisit if accuracy issues arise at extreme distances.

---

## Tribes-Specific Systems

1. **Projectile velocity inheritance** — `ProjectileSpawn { origin, velocity: player_vel + muzzle_vel, ... }` event. Predicted identically on both client and server (shared `core` code).
2. **Hitscan lag compensation** — server keeps ring buffer of past player capsule positions (~200ms). On hit request, rewind to client's tick at fire time, run shapecast against historical state.
3. **Skiing** — `MoveAndSlide` grounded check + custom friction state. Surface normal from contact query.
4. **Tickrate** — 60 Hz server, 60 Hz client prediction, render at display rate. Original `.uc` constants tuned for 30 Hz; refactor them as dt-independent acceleration/decay rates.

---

## Scope Decisions

- **Auth:** Basic user registration + login with plain passwords (hashed with argon2 in storage).
- **Database:** SQLite, admin-managed. No Postgres.
- **Matchmaking:** Solo queue only, no parties.
- **Progression:** Everything unlocked from day 1.
- **Bots:** No bots.
- **Anti-cheat:** Not a concern for now.
- **Replays:** Supported (recording on server, playback on client).
- **Map scale:** `f32` floats, no origin rebasing.

---

## Milestones

1. **Workspace skeleton** — all crates stubbed, `bins/realm` boots tonic + SQLite, `bins/client`/`bins/server` boot Bevy with shared `core`. Proto compiled.
2. **Realm auth + server directory** — `Register`/`Login`/`ListServers`/`RegisterServer`/`Heartbeat`/`ValidateSession`. SQLite migrated. Plain password login end-to-end.
3. **Load one map** — `Crossfire` from JSON, render static meshes + spawns. No physics yet.
4. **Tribes movement sandbox** — `MoveAndSlide` kinematic controller: walk/jump/jetpack/ski. Single-player, no netcode. **Riskiest milestone.**
5. **Spinfusor + projectile inheritance** — local only. `PreSpawnedPlayerObject` pattern from lightyear `fps` example.
6. **Netcode integration** — lightyear client+server with `leafwing` inputs, 2 clients connect, verify prediction.
7. **Matchmaking flow** — queue, server assignment, token validation, connect to game server.
8. **CTF ruleset** — flag grab/cap/return, scoring, scoreboard (egui).
9. **Hitscan weapons** — sniper, laser, chain. `LagCompensationSpatialQuery` from lightyear `fps`/`projectiles` examples.
10. **Replay recording** — `ReplayRecorder` on server, serialize inputs + initial state. `SubmitMatchResult` with `replay_blob`.
11. **Replay playback** — `ReplayPlayer` on client, feed recorded inputs through deterministic sim. "Watch replay" UI in egui menu.
12. **Loadouts + inventory** — port weapon classes from `.uc`. Everything unlocked.
13. **Stats ingestion** — post-match stats → SQLite, player stats display.
14. **Polish** — particles (hanabi), audio (kira), UI, server browser.

---

## Risks

- **Lightyear + Avian 0.19 integration maturity:** lightyear has an Avian integration crate (`lightyear_avian3d`) but it's newer. May need manual sync if integration breaks.
- **Tribes movement feel:** the `.uc` constants are tuned for 30 Hz UE3; getting the same feel at 60 Hz requires careful tuning. Mitigation: milestone 4 is sandbox-only, timeboxed.
- **gRPC + Bevy in same process:** client and realm-server binaries are separate crates, but the client links tonic for gRPC calls. Tonic + Bevy both use tokio; should coexist, but watch for runtime contention.
- **Determinism across architectures:** Avian's `enhanced-determinism` feature helps, but true cross-platform determinism (x86 vs ARM) requires testing. Replays recorded on one server should play back on any client.
