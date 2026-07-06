# Tribes: Ascend Bevy Port — Architecture & Implementation Plan

## Overview

Port Tribes: Ascend (UDK/UE3) to Bevy (Rust) using decompiled game logic (3,717 `.uc` classes) and extracted assets (textures, meshes, sounds, maps, game objects). Three-tier architecture: realm broker, server-authoritative game servers, and clients. Server-authoritative netcode with client-side prediction and rollback.

**Repo structure:** Source lives in `src/port/` (git submodule, public `github.com/hmih/tribes`). Assets and decompilation work live in the top-level tascend repo (private). The port references assets via relative paths (`../../decompile/assets/...`).

**Targets:** Linux, Windows, macOS. Keyboard + mouse only (no gamepad/Xbox).

**Code style:** Rust correctness first. Enums over hardcoded integer IDs. Serde-friendly structured formats over INI. No `unwrap()` without justification.

---

## Topology

```
┌─────────────────────┐         gRPC (HTTP/2 + TLS)         ┌─────────────────────┐
│   Realm Server      │◀───────────────────────────────────│  Game Server         │
│  (tonic + sqlx)     │                                     │  (Bevy headless)    │
│  SQLite, no Bevy    │   RegisterServer, Heartbeat,        │                     │
│                     │   SubmitMatchResult                  │  Registers on boot  │
│  - User auth        │                                     │  Heartbeats to realm│
│  - Server directory │                                     │  Validates tokens   │
│  - Matchmaking      │                                     │                     │
│  - Match history    │                                     │                     │
└─────────▲───────────┘                                     └──────────▲──────────┘
          │                                                            │
          │ gRPC                                              UDP (lightyear)
          │                                                            │
┌─────────┴───────────┐                                     ┌──────────┴──────────┐
│   Client             │◀──────────────────────────────────▶│  Players             │
│  (full Bevy)         │           UDP (lightyear)          │                      │
│                      │                                    │                      │
│  2 connections:      │                                    │                      │
│  1. Realm (gRPC)    │                                    │                      │
│  2. Game server(UDP)│                                    │                      │
└──────────────────────┘                                    └──────────────────────┘
```

**Two connections per client**, completely decoupled:
1. **Realm connection (gRPC, HTTP/2 + TLS)** — login, server directory, matchmaking queue, post-match stat ingestion. Idle during gameplay.
2. **Game connection (UDP, lightyear)** — gameplay traffic to one game server. Hot path.

The realm never sees a gameplay packet. Game servers register with the realm over gRPC.

---

## Dependency Matrix

All verified for Bevy 0.19.0 compatibility as of 2026-07-06.

| Crate | Version | Role |
|-------|---------|------|
| `bevy` | 0.19.0 | Core engine |
| `avian3d` | 0.7.0 | Physics: colliders, spatial queries, `MoveAndSlide` system parameter |
| `lightyear` | 0.28.0 | Netcode: prediction, rollback, lag compensation, Avian integration |
| `leafwing-input-manager` | 0.21.0 | Input abstraction with `ActionDiff` for networking |
| `bevy_hanabi` | 0.19.0 | GPU particles (jetpacks, projectiles, explosions) |
| `bevy_kira_audio` | 0.26.0 | Audio (1,543 OGG sounds) |
| `bevy_egui` | 0.41.0 | Menus, server browser, scoreboard |
| `bevy_common_assets` | 0.17.0 | Typed JSON asset loaders (`json` feature) |
| `bevy_tweening` | 0.16.0 | UI animations, flag capture progress |
| `tonic` | 0.13 | gRPC (realm server + client calls) |
| `sqlx` | 0.8 | SQLite (compile-time checked queries) |
| `prost` | 0.13 | Protobuf codegen |
| `argon2` | 0.5 | Password hashing |

### lightyear Feature Flags

```
lightyear = { version = "0.28", features = [
  "client", "server",          # topology
  "avian3d",                   # Avian physics integration (lightyear_avian3d)
  "leafwing",                  # leafwing input networking (lightyear_inputs_leafwing)
  "deterministic",             # deterministic replication (lightyear_deterministic_replication)
  "prediction",                # client-side prediction + rollback
  "interpolation",             # remote entity interpolation
  "netcode",                   # netcode.io secure connections
  "replication",               # entity replication
  "udp",                       # UDP transport
] }
```

### avian3d Feature Flags

```
avian3d = { version = "0.7", features = [
  "3d", "f32", "parry-f32",    # required
  "enhanced-determinism",      # cross-arch deterministic math (for rollback + replays)
  "serialize",                 # Serde support (for replays)
  "collider-from-mesh",        # generate colliders from glTF meshes
  "xpbd_joints",               # joints for vehicle turrets etc.
] }
```

### Verified Sub-Crate Architecture

lightyear 0.28 is a facade crate. Verified optional sub-crates:
- `lightyear_avian3d` — Avian physics integration (system ordering, component sync)
- `lightyear_inputs_leafwing` — leafwing `ActionDiff` networking
- `lightyear_deterministic_replication` — input-only replication (replay foundation)
- `lightyear_prediction` — `Predicted` component, rollback, prediction history
- `lightyear_interpolation` — remote entity interpolation
- `lightyear_netcode` — netcode.io secure connections over UDP

avian3d 0.7 verified:
- `MoveAndSlide` — system parameter for kinematic character controllers (not a built-in controller)
- `SpatialQuery` — system parameter for raycasts, shapecasts, point projection
- `enhanced-determinism` — libm-based math for cross-platform determinism
- `character_controller` module — utilities for building custom controllers
- Example reference: `crates/avian3d/examples/kinematic_character_3d`

### Warning: Unverified lightyear APIs

The following were described in lightyear examples but are NOT confirmed as library APIs in 0.28:
- `LagCompensationSpatialQuery` — may be a pattern from the `fps` example, not a library type. Hitscan lag compensation may need a custom implementation using lightyear's prediction history + Avian's `SpatialQuery`.
- `PreSpawnedPlayerObject` — projectile prediction pattern from the `fps` example. May need to be implemented manually.

---

## Workspace Structure

```
tribes/                                ← public submodule (src/port/)
├── Cargo.toml                         # [workspace], resolver = "3"
├── proto/                             # .proto files
│   └── realm.proto                    # Realm + RealmAdmin services
├── crates/
│   ├── core/                          # Components, config, weapon definitions
│   │                                  # Shared by client + server
│   ├── protocol/                      # Lightyear protocol: MessageRegistry, ChannelRegistry, ComponentRegistry
│   ├── assets/                        # AssetLoaders for map/game-object JSON + T3D parser
│   │                                  # Uses bevy_common_assets
│   ├── physics/                       # Tribes movement: skiing, jetpack, projectile inheritance
│   │                                  # Uses avian3d MoveAndSlide
│   ├── input/                         # leafwing Action enum, InputMap (structured format, not INI)
│   │                                  # Keyboard + mouse only
│   ├── client/                        # Bevy plugins: rendering, prediction, audio, UI (egui)
│   │                                  # Custom WGSL billboard shader (~50 lines)
│   └── server/                        # Bevy plugins: authority, replication, scoring, lag comp, match lifecycle
│                                      # lightyear server + avian + tonic client
└── bins/
    ├── client/                        # boots client plugins
    ├── server/                        # boots server plugins + registers with realm
    └── realm/                         # boots tonic gRPC server + sqlx SQLite
```

**Key invariant:** `core` + `physics` + `protocol` are imported by both client and server. This guarantees simulation symmetry for rollback netcode.

**Asset paths:** Port code references assets in the parent tascend repo via:
```
../../decompile/assets/skeletal-meshes/...
../../decompile/assets/static-meshes/...
../../decompile/assets/sounds/...
../../decompile/assets/maps/...
../../decompile/assets/game-objects/...
```

---

## Asset Pipeline (Critical for Milestone 3)

### What we have

| Asset Type | Format | Count | Size | Status |
|-----------|--------|-------|------|--------|
| Skeletal meshes | `.glb` (glTF 2.0) | 269 + 73,079 anims | 1.2 GB | Valid, loads in Bevy |
| Static meshes | `.gltf` (glTF 2.0, inline buffer) | 140 (86k verts, 91k tris) | ~62 MB | **Fixed 2026-07-06**, loads in Bevy |
| Textures | `.png` | 2,137 | 2.0 GB | Valid, loads in Bevy |
| Sounds | `.ogg` | 1,543 | 35 MB | Valid, loads via bevy_kira_audio |
| Maps | `.json` (T3D text in properties) | 123 | 668 MB | Needs T3D parser |
| Game objects | `.json` (T3D text in properties) | 548 | 8.4 MB | Needs T3D parser |
| Manifest | `.json` (structured) | 1 | 6.4 MB | Valid, links code→assets |

### T3D Property Parser (todo: `crates/assets/`)

The map and game-object JSON files store UE3 property values as T3D text strings, not structured JSON:

```json
{ "name": "Location", "type": "StructProperty", "value": "Location=(X=-7220.08,Y=14720.0,Z=2472.0)" }
```

Inline sub-objects use `begin object name=... class=... ... end object` blocks. Some entries have `/* ERROR: ... */` markers from the decompiler.

We need a Rust T3D parser that:
1. Parses T3D text values into typed `Vec3`, `Rotator`, arrays, enums, etc.
2. Handles inline sub-object blocks
3. Skips error markers
4. Maps to serde-deserializable Rust types

This goes in `crates/assets/` alongside the `bevy_common_assets` loaders.

---

## Realm Server (no Bevy)

**Stack:** `tonic` (gRPC, HTTP/2 + TLS), `sqlx` (SQLite, compile-time checked queries), `prost` + `tonic-build` (protobuf codegen), `argon2` (password hashing), `tokio`, `tracing`.

Bevy has no dependency on the realm; the realm has no Bevy dependency.

### SQLite Schema (admin-managed, single file)

```sql
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
) STRICT;

CREATE TABLE sessions (
    token TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    expires_at TEXT NOT NULL
) STRICT;

CREATE TABLE servers (
    id INTEGER PRIMARY KEY,
    addr TEXT NOT NULL,
    region TEXT NOT NULL DEFAULT '',
    current_map TEXT,
    player_count INTEGER NOT NULL DEFAULT 0,
    max_players INTEGER NOT NULL,
    latency_ms INTEGER,
    last_heartbeat TEXT NOT NULL DEFAULT (datetime('now')),
    status TEXT NOT NULL DEFAULT 'idle'
) STRICT;

CREATE TABLE match_history (
    id INTEGER PRIMARY KEY,
    map TEXT NOT NULL,
    server_id INTEGER NOT NULL REFERENCES servers(id),
    started_at TEXT NOT NULL,
    ended_at TEXT NOT NULL,
    winning_team INTEGER
) STRICT;

CREATE TABLE player_stats (
    user_id INTEGER PRIMARY KEY REFERENCES users(id),
    kills INTEGER NOT NULL DEFAULT 0,
    deaths INTEGER NOT NULL DEFAULT 0,
    captures INTEGER NOT NULL DEFAULT 0,
    wins INTEGER NOT NULL DEFAULT 0,
    losses INTEGER NOT NULL DEFAULT 0,
    matches_played INTEGER NOT NULL DEFAULT 0
) STRICT;
```

### gRPC Protocol (`proto/realm.proto`)

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
}

service RealmAdmin {
  rpc RegisterServer(RegisterServerRequest) returns (ServerRegistration);
  rpc UpdateServerStatus(ServerStatus) returns (Empty);
  rpc UnregisterServer(ServerId) returns (Empty);
  rpc ValidateSession(ValidateSessionRequest) returns (ValidateSessionResponse);
  rpc SubmitMatchResult(MatchResult) returns (Empty);
}

message MatchResult {
  string map = 1;
  uint64 server_id = 2;
  repeated PlayerMatchResult players = 3;
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

## Matchmaking + Server Lifecycle

1. **Game server boots** → gRPC `RegisterServer(addr, region, max_players)` → realm returns `ServerRegistration{id}`
2. **Game server updates** → gRPC `UpdateServerStatus(id, current_map, player_count)` — on state changes
3. **Realm heartbeat** → realm pings game server's UDP port every 5 min. No response → delist
4. **Client queues** → gRPC `Queue(session_token)` → realm finds available server → returns `QueueResponse{server_addr, connect_token}`
5. **Client connects** → UDP to game server with `connect_token` → game server validates via gRPC → player joins
6. **Match ends** → game server calls `SubmitMatchResult` with per-player stats → realm stores in SQLite + updates `player_stats`

---

## Physics Approach

**Use Avian's `MoveAndSlide` system parameter for the kinematic character controller.** Confirmed API in Avian 0.7:
- `RigidBody::Kinematic` component
- `MoveAndSlide` system parameter for collision-aware movement
- `SpatialQuery` for raycasts/shapecasts (grounded check, projectile sweeps, hitscan)
- `enhanced-determinism` feature enabled for cross-arch consistency

**Custom Tribes movement layer on top:**
- `Skiing` component: while grounded + skiing input held, reduce friction
- `Jetpack` component: thrust along view vector + energy pool
- `AirControl` system: custom acceleration curve

Avian handles: collision shapes, raycasts, capsule sweeps, terrain/static mesh colliders. We hand-roll: integrator for the player, projectile motion, jetpack energy, skiing state machine.

### Movement Physics Reimplementation (Riskiest Work)

The actual physics integration was **native C++** in UDK's `PhysType_AccelCap` — not in the decompiled `.uc` files. We only have the input feeding logic and the constants. We must re-implement the integrator from scratch.

**Key constants (from `TrFamilyInfo.uc`, `TrPawn.uc`):**

| Constant | Value | Unit |
|----------|-------|------|
| `GroundSpeed` | 440 | uu/s |
| `AirSpeed` | 550 | uu/s |
| `AirControl` | 0.2 | factor |
| `JumpZ` | 322 | uu/s |
| `CustomGravityScale` | 0.8 | × 9.81 m/s² |
| `MaxJettingSpeed` | 2500 | uu/s |
| `TerminalJettingSpeed` | 3000 | uu/s |
| `MaxSkiSpeed` | 2500 | uu/s |
| `TerminalSkiSpeed` | 3000 | uu/s |
| `PeakSkiControlSpeed` | 1600 | uu/s |
| `SkiControlSigmaSquare` | 100000 | variance |
| `MaxSkiControlPct` | 0.65 | factor |
| `SkiSlopeGravityBoost` | 2.0 | multiplier |
| `FallVelocityTransfer` | 1.0 | factor |
| `MaxJetpackBoostGroundspeed` | 1600 | uu/s |
| `ForwardJettingPct` | 0.4 | bias factor |
| `JetpackInitTotalTime` | 2.4 | s |
| `JetpackInitAccelMultiplier` | 1500 | |
| `JetpackPowerPoolCost` | 30 | per 0.1s tick |
| `MaxStoppingDistance` | 500 | uu |

**Behaviors to replicate:**
- Speed caps per mode (ski/jet/ground), with terminal speeds above max
- Ski control authority = Gaussian falloff `exp(-v²/2σ²)` peaking at PeakSkiControlSpeed
- Ski slope gravity boost when facing downhill
- Fall velocity transfer: downward speed → forward ski speed on landing
- Jetpack initiating boost (2.4s window, throttled by MaxJetpackBoostGroundspeed)
- Forward-jet bias (ForwardJettingPct scales lateral accel)
- Air control reduces with speed (linear ramp between configurable range)
- Energy pool: cost per tick, recharge rate, min 10% to start jetpack

**Scale:** `f32` everywhere. Tribes maps are ~2 km; precision is fine.

---

## Netcode Design

- **Channels (lightyear):**
  - `Input` channel (reliable-ordered, client→server): leafwing `ActionDiff`
  - `Replication` channel (unreliable, server→client): entity state with delta compression
  - `Events` channel (reliable-ordered, bidirectional): flag grabs, captures, kills, damage, projectile spawns
- **Prediction:** player-controlled pawn is predicted with rollback. Other players are interpolated.
- **Lag compensation:** needed for hitscan weapons. Server must store historical snapshots for ~200ms. Implementation may be based on the lightyear `fps` example patterns (not confirmed as a library API in 0.28).
- **Tickrate:** 60 Hz server, 60 Hz client prediction, render at display rate. Original `.uc` constants tuned for 30 Hz — refactor as dt-independent rates.

---

## Input System

### Keyboard + Mouse Only

No gamepad/Xbox support. Cross-platform: Linux, Windows, macOS.

### leafwing Actions (Structured Format)

Replace the 45 KB INI file (`DefaultInput.ini`) with a Rust `enum` + serde-deserializable input config. The INI's two-tier system (GBA virtual actions → key bindings) maps directly to leafwing's `Action` enum + `InputMap`.

**Example structure (final design in `crates/input/`):**

```rust
#[derive(Actionlike, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameAction {
    MoveForward,
    MoveBackward,
    StrafeLeft,
    StrafeRight,
    Ski,
    Jetpack,
    Fire,
    Aim,
    ActivateMelee,
    ActivateBelt,
    ActivatePack,
    Reload,
    Use,
    SpotTarget,
    ShowScores,
    SwitchWeapon(u8),       // 0..9
    CallIn(u8),             // 1..6
    ClassHotkey(ClassType), // enum, not integer
    DropFlag,
    BehindView,
    VoteYes,
    VoteNo,
    // ... ~40-50 total actions
}
```

The ~106 GBA actions in the original INI collapse into ~40-50 leafwing actions. Many were unused, platform-specific, or debug-only.

`ActionDiff` is used for networking (lightyear `leafwing` feature), which sends only input *changes* — compact and predictable.

---

## Deferred: Replay System

Replays are deferred to after the initial implementation stabilizes. The design is:

Lightyear's `deterministic` feature + Avian's `enhanced-determinism` + leafwing's `ActionDiff` format = deterministic simulation driven by inputs. Recording inputs + initial snapshot = complete replay.

**Replay format (planned):**

```rust
struct ReplayFile {
    version: u32,
    protocol_hash: u64,
    map_name: String,
    match_id: u64,
    tick_rate: f64,
    player_ids: Vec<PlayerId>,
    initial_snapshot: Vec<u8>,     // serialized world state at tick 0
    inputs: Vec<InputFrame>,
    duration_ticks: u32,
}

struct InputFrame {
    tick: u32,
    inputs: Vec<(PlayerId, ActionDiff)>,
}
```

Replays are ~100 KB for a 20-min match. Stored as `BLOB` in realm SQLite later.

---

## Tribes-Specific Systems

1. **Projectile velocity inheritance** — `ProjectileSpawn { origin, velocity: player_vel + muzzle_vel, ... }` event. Predicted identically on both client and server (shared `core` code). Formula from `TrProjectile.uc`:
   ```
   forward_pct = min(dot(normalize(owner_vel), projectile_dir), max_inherit_pct)
   inherit_pct = max(base_inherit_pct, forward_pct)
   proj.vel.xy += inherit_pct * owner_vel.xy
   proj.vel.z += inherit_pct_z * owner_vel.z  // suppressed on flat ground while skiing
   ```
2. **Hitscan lag compensation** — server keeps ring buffer of past player capsule positions (~200ms). On hit request, rewind to client's tick at fire time, run shapecast. Risk: `LagCompensationSpatialQuery` not confirmed as a library API in lightyear 0.28. May need custom implementation.
3. **Skiing** — grounded check + custom friction state. Surface normal from contact query.
4. **Tickrate** — 60 Hz server + client. `.uc` constants refactored as dt-independent.

---

## Scope

- **Auth:** Basic user registration + login with argon2-hashed passwords.
- **Database:** SQLite, admin-managed. No Postgres.
- **Matchmaking:** Solo queue only, no parties.
- **Progression:** Everything unlocked from day 1.
- **Bots:** No bots.
- **Anti-cheat:** Not a concern for now.
- **Map scale:** `f32` floats, no origin rebasing.
- **Replays:** Deferred. Input design leaves the door open.
- **Input:** Keyboard + mouse. No gamepad. Structured serde format replacing INI.

---

## Milestones

1. **Workspace skeleton** — all crates stubbed, `bins/realm` boots tonic + SQLite, `bins/client`/`bins/server` boot Bevy with shared `core`. Proto compiled.
2. **Realm auth + server directory** — `Register`/`Login`/`ListServers`/`RegisterServer`/`Heartbeat`/`ValidateSession`. SQLite migrated.
3. **Load one map** — `Perdition` from JSON, render static meshes + skeletal meshes + spawns. T3D parser in `crates/assets/`. No physics yet.
4. **Tribes movement sandbox** — `MoveAndSlide` kinematic controller: walk/jump/jetpack/ski. Single-player, no netcode. **Riskiest milestone.**
5. **Spinfusor + projectile inheritance** — local only. Projectile spawning with velocity inheritance.
6. **Netcode integration** — lightyear client+server with `leafwing` inputs, 2 clients connect, verify prediction.
7. **Matchmaking flow** — queue, server assignment, token validation, connect to game server.
8. **CTF ruleset** — flag grab/cap/return, scoring, scoreboard (egui).
9. **Hitscan weapons** — sniper, laser, chain. Lag compensation (likely custom implementation).
10. **Loadouts + inventory** — port weapon classes from `.uc`. Everything unlocked.
11. **Stats ingestion** — post-match stats → SQLite, player stats display.
12. **Polish** — particles (hanabi), audio (kira), UI, server browser.
13. **Replay recording + playback** — deferred until the dust settles.

---

## Risks

1. **Movement feel:** The AccelCap integrator was native C++, and `.uc` constants are tuned for 30 Hz UE3. Replicating Tribes' movement feel at 60 Hz from scratch is the biggest risk. Milestone 4 is sandbox-only and timeboxed.
2. **Lag compensation:** `LagCompensationSpatialQuery` is not confirmed as a library API in lightyear 0.28. Hitscan lag comp may require a custom implementation using lightyear's prediction history + Avian's `SpatialQuery`.
3. **Projectile prediction:** `PreSpawnedPlayerObject` is a pattern from lightyear examples, not a library API. May need custom implementation following that pattern.
4. **gRPC + Bevy in same binary:** Client links tonic for gRPC calls. Tonic + Bevy both use tokio; should coexist, but watch for runtime contention.
5. **Cross-architecture determinism:** Avian's `enhanced-determinism` helps, but true x86 vs ARM determinism requires testing.