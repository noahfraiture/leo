# Minimal Synology Surveillance Station API Mock

Date: 2026-07-19
Status: Approved
Tracking issue: https://github.com/noahfraiture/leo/issues/23

## Purpose

Provide a separate Synology Surveillance Station simulator that exposes the first Web API contracts Leo needs. Leo must later work with physical Surveillance Station by changing addresses and credentials rather than replacing its API client.

The simulator represents the NAS as one process that knows about several independently running Axis camera processes. It never shares Rust camera state with them.

## Sources

- Current signed-in API site: https://surveillance-api.synology.com
- Public Surveillance Station Web API v3.11 guide: https://global.download.synology.com/download/Document/Software/DeveloperGuide/Package/SurveillanceStation/All/enu/Surveillance_Station_Web_API.pdf
- Synology third-party integration overview: https://www.synology.com/en-global/surveillance/feature/3rd-party
- Synology P3278-LV compatibility entry: https://www.synology.com/en-global/compatibility/camera?query=P3278-LV

The public guide is dated 2021-12-30. Runtime API discovery and later physical-device validation remain authoritative when a real NAS is available.

## Scope

The first increment implements:

- `SYNO.API.Info.Query` version 1
- `SYNO.SurveillanceStation.Camera.List` version 9
- `SYNO.SurveillanceStation.ExternalRecording.Record` version 2
- Synology-compatible request dispatch, JSON envelopes, and documented errors
- independent in-memory recording state per configured camera
- network-derived `Normal` and `Disconnected` camera states

The first increment does not implement:

- authentication or sessions, tracked by issue #42
- recording catalogue or exports
- playable media or RTSP ingestion
- persistence across simulator restarts
- retention or storage management
- camera configuration, PTZ, snapshots, or other Surveillance Station APIs
- a simulator-specific multi-camera endpoint

## Process Topology

```text
Leo
├── VAPIX requests -> Axis camera process 1
├── VAPIX requests -> Axis camera process 2
└── Surveillance Station Web API -> Synology process
                                      ├── network reachability -> camera 1
                                      └── network reachability -> camera 2
```

Each virtual Axis camera runs as a separate process with its own socket address. The Synology simulator is a separate workspace crate and process. This matches the physical network boundary while allowing both implementations to remain small.

## Startup Configuration

The `synology` executable accepts:

1. its bind socket address
2. one or more Axis camera socket addresses

Camera IDs are assigned from `1` in argument order. Names are `camera-1`, `camera-2`, and so on. IDs and names are stable for a process invocation as long as argument order remains unchanged.

A configuration-file format, custom names, credentials, and persisted IDs are deferred until a consumer requires them.

## HTTP Surface

### API Discovery

Fixed route:

```text
GET /webapi/query.cgi
```

Accepted parameters:

- `api=SYNO.API.Info`
- `method=Query`
- `version=1`
- `query=ALL`, an exact implemented API name, or `SYNO.SurveillanceStation.`

The result advertises only implemented APIs:

```json
{
  "success": true,
  "data": {
    "SYNO.SurveillanceStation.Camera": {
      "path": "entry.cgi",
      "minVersion": 9,
      "maxVersion": 9
    },
    "SYNO.SurveillanceStation.ExternalRecording": {
      "path": "entry.cgi",
      "minVersion": 2,
      "maxVersion": 2
    }
  }
}
```

The simulator advertises narrow version ranges because it must not claim compatibility with versions it does not implement. Leo must select a version it understands within the discovered range rather than blindly selecting the server maximum.

### Camera List

Discovered route:

```text
GET /webapi/entry.cgi
```

Required parameters:

- `api=SYNO.SurveillanceStation.Camera`
- `method=List`
- `version=9`

The first increment returns all configured cameras. Each camera includes the fields Leo initially needs:

- `id`
- `name`
- `ip`
- `port`
- `status`
- `vendor`, set to `AXIS`
- `model`, set to `P3278-LV`
- `channel`, set to `1`

The response also contains `total`. Additional real Surveillance Station fields are intentionally omitted; Leo must tolerate unknown and absent optional fields.

Immediately before building the response, the simulator attempts a TCP connection to each camera socket address with a bounded timeout:

- reachable: status `1`, Normal
- connection failure or timeout: status `3`, Disconnected

This preserves the process and network failure boundary without depending on unfinished VAPIX endpoints or adding an HTTP client.

### External Recording

Discovered route:

```text
GET /webapi/entry.cgi
```

Required parameters:

- `api=SYNO.SurveillanceStation.ExternalRecording`
- `method=Record`
- `version=2`
- `cameraId=<integer>`
- `action=start|stop`

The documented operation accepts one camera ID. Leo may send independent requests concurrently for several selected cameras. The simulator does not provide an atomic or simulator-only batch endpoint.

For a known reachable camera, `start` sets its in-memory recording state to active and `stop` sets it to inactive. Repeating either action is idempotent in the simulator. Leo must not rely on this edge behavior until it is checked against physical Surveillance Station.

Success response:

```json
{
  "success": true,
  "data": {
    "success": true
  }
}
```

Recording state is independent per camera. Concurrent calls may therefore produce partial success, matching the public per-camera contract.

## Errors

JSON API failures use HTTP status `200` and the standard envelope:

```json
{
  "success": false,
  "error": {
    "code": 104
  }
}
```

Implemented codes:

| Code | Meaning |
| ---: | --- |
| 101 | Invalid or missing common parameters |
| 102 | API does not exist |
| 103 | Method does not exist |
| 104 | API version is not supported |
| 400 | External recording execution failed, including unknown or unreachable camera |
| 401 | External recording parameter is invalid |

Authentication-related errors and camera-disabled error `402` are deferred because the first startup configuration cannot create those states.

## State And Concurrency

The process owns a collection of configured cameras and one recording boolean per camera. Request handlers share this state safely. Network checks do not hold the state lock while waiting.

State resets on process restart. Catalogue entries, recording timestamps, and recovery are introduced only when their corresponding APIs are added.

## Testing

Tests exercise the Synology router without requiring completed Axis APIs:

- discovery returns only Camera and ExternalRecording with the expected paths and versions
- camera list reports a temporary listening socket as Normal
- camera list reports a closed socket as Disconnected
- start and stop update independent state for two cameras
- missing parameters, unknown API, unknown method, unsupported version, unknown camera, and unreachable camera return the expected Synology envelope and code

Temporary TCP listeners stand in for reachable cameras. Full Axis/Synology end-to-end tests are deferred until both simulators expose the required endpoints and are not a completion gate for issue #23.

The completion command for this increment is:

```sh
cargo test -p synology
```

The complete workspace test suite is not a gate while the camera crate is active unfinished work.

## Follow-Up Work

- Issue #42 adds `SYNO.API.Auth`, SID sessions, credential configuration, and protected calls.
- Recording catalogue and export APIs are added only when the CLI workflow needs them.
- RTSP ingestion and playable recording artifacts are added only after the virtual camera stream is available.
- Issue #26 validates API versions and behavior against physical Surveillance Station and replaces assumptions with observed behavior.
- Issue #32 adds deterministic cross-process failure scenarios after both simulators are ready.
