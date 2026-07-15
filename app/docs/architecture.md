# Multi-camera training recording system

# 0. Vocabulary

| Term                   | Definition                                                           |
| ---------------------- | -------------------------------------------------------------------- |
| **Session**            | Full recording period across all cameras.                            |
| **Recording**          | Raw video file produced by one camera.                               |
| **Video**              | Recording exported to a standard usable format.                      |
| **Video stream**       | Continuous encoded video data.                                       |
| **Frame**              | One decoded image.                                                   |
| **Frame rate**         | Number of frames produced per second.                                |
| **Sampling rate**      | Number of frames selected per second.                                |
| **Sampling schedule**  | Sampling rate changes over time.                                     |
| **Sample**             | A selected frame.                                                    |
| **Sample sequence**    | Ordered samples selected from one recording.                         |
| **Frame index**        | Position of a frame in a recording.                                  |
| **Sample index**       | Position of a sample in a sample sequence.                           |
| **Frame timestamp**    | Frame position on the session timeline.                              |
| **Frame group**        | Frames from different recordings associated with the same timestamp. |
| **Frame batch**        | Ordered frame groups covering a bounded time range.                  |
| **Session sequence**   | Ordered frames groups covering the full session.                     |


# 1. Goal

Record training sessions using approximately three to five AXIS P3278-LV cameras.

The system must:

- record locally without depending on the internet
- record reliably for an entire training day
- continue recording if the operator laptop crashes or sleeps
- allow an operator to monitor all cameras
- support camera selection, digital zoom, quality profiles and annotations
- provide live frames to an AI analysis pipeline
- retain original high-quality recordings
- detect failures and recording gaps quickly
- make recordings accessible programmatically
- require little or no manual post-production
- allow exported or processed data to be taken home

This is not a traditional surveillance deployment. It is a session-oriented video capture and analysis system.

---

# 2. System architecture

```text
AXIS cameras
├── high-quality stream -> Synology Surveillance Station -> NAS disks
├── low-quality preview stream -> operator application
├── low-quality AI stream -> live analysis process
└── VAPIX API <- custom operator application

Synology NAS
├── Surveillance Station recording management
├── RAID/SHR storage
├── recording retention
├── camera reconnection and monitoring
└── recording catalogue

Operator laptop
├── camera grid
├── session controls
├── VAPIX camera controls
├── metadata and annotations
├── optional Surveillance Station UI
└── optional live AI processing

UPS
├── Synology NAS
└── PoE switch
    └── cameras
```

## Ownership boundary

This boundary is important to avoid conflicts.

### Surveillance Station owns

- archival recording
- reliable execution of recording start/stop requests
- the recording stream configuration
- reconnection after camera/network interruptions
- recording catalogue and health monitoring
- recording recovery
- storage rotation and retention enforcement

### Custom application owns

- session workflow
- operator interface
- recording requests sent through the Synology API
- low-resolution previews
- AI stream consumption
- timestamped custom metadata
- PTZ or digital-view controls
- operator presets
- session naming
- session retention, export and deletion decisions
- downstream processing status
- alarms presented to the operator

### Axis VAPIX provides direct device operations

- camera status and capabilities
- separate preview and AI streams
- supported PTZ controls
- stream profiles
- snapshots
- overlays
- camera-side events
- advanced settings that Surveillance Station does not expose

The custom application requests recording start/stop through Surveillance Station; it does not run a separate archival recorder through VAPIX.
The custom application decides when session data should be retained, exported or deleted, while Surveillance Station enforces storage rotation and maintains the recording catalogue.
Do not let both Surveillance Station and the custom application modify the same recording profile.
The VMS exists to simplify recording and own its reliability concerns.

---


# 3. Hardware checklist

## Cameras

For each camera:

- AXIS P3278-LV
- correct mounting hardware
- Cat 6 Ethernet cable
- optional high-endurance microSD card to keep recording in case of interruption with the NAS
  The VMS should handle the seamless flow
- Synchronized clock through NTP via the NAS.
- Static IP so that we don't need a router

## NAS


Responsible for durable storage and NTP.

Recommended:

```text
Synology DS224+
2 x 4 TB or 2 x 8 TB HDD
SHR-1
```

The DS224+ is normally diskless, supports Surveillance Station, and includes two Surveillance Station device licenses. Additional cameras require additional licenses.

### Capacity

For _three_ P3278-LV cameras, eight hours, at realistic H.265 bitrates:

| Approximate bitrate per camera | Total storage |
| -----------------------------: | ------------: |
|                       4 Mbit/s |         43 GB |
|                       8 Mbit/s |         86 GB |
|                      12 Mbit/s |        130 GB |
|                      20 Mbit/s |        216 GB |
|                      30 Mbit/s |        324 GB |

Even 4 TB usable storage gives substantial room for several sessions. Two 4 TB disks in SHR-1 provide approximately 4 TB usable before filesystem overhead.

## PoE switch

Preferred European option:

A managed switch is useful because the operator or administrator can:

- check whether a camera port is connected
- inspect PoE consumption
- restart a frozen camera by power-cycling its port
- monitor link errors
- separate cameras using VLANs later

## UPS

The UPS should power:

```text
Synology NAS
PoE switch
```

Synology DSM can receive UPS status through USB and enter safe mode before battery exhaustion, stopping services and protecting mounted storage.
Test compatibility for the exact UPS model, not only the brand.

## Operator laptop

Because the NAS/VMS handles recording, the laptop requirements are moderate.

---

# 4. Network design

## Network

To avoid an unnecessary router, use static IP addresses for the cameras and NAS. Every device should be connected to the PoE switch.

## Time synchronization

All of the following must use the same clock source:

- cameras
- Synology
- operator laptop
- AI service
- metadata service

This is more important than firing every start command in the exact same millisecond.

Store timestamps in UTC internally.

---

# 5. Application design

## Functional requirements

### Session operation

- Enable or disable individual cameras for a session.
- Define a sampling rate for each camera.
- Control digital zoom.
- Add timestamped notes and bookmarks.
- Show recording and camera health, warning the operator when action is required.

### Retention and export

- Group videos and metadata by session.
- Keep metadata aligned with the session timeline.
- Warn when storage is insufficient and propose an action.
- Export recordings to a standard format for manual analysis.
- Allow an operator to discard a session recording.

### Analysis

- Provide live streams to the AI analysis process.
- Support offline frame sampling and analysis.
- Optionally blur faces during offline processing.

## External integrations

### Synology Surveillance Station API

The custom application uses the Synology API to:

- query camera and recording status
- request recording start/stop
- access the recording catalogue and exports where supported

Surveillance Station retains the recording reliability and storage responsibilities defined in section 2.

### Axis VAPIX

The custom application uses VAPIX to:

- consume RTSP H.264/H.265 streams for previews and live AI
- query camera status and capabilities
- control PTZ on compatible cameras
- control digital cropping for live preview
- request snapshots
- select optional preview or AI stream profiles

VAPIX also provides MJPEG or JPEG snapshots that could be used for frame sampling instead of decoding recorded video. This would make sampling poll-based: delays would produce irregular sampling, and frames missed during temporary failures could not be recovered.

## Operator interface

The interface provides:

- a master session control that starts the session clock and requests recording for enabled cameras
- a live view of all cameras
- per-camera recording and health status
- warnings with suggested operator actions
- per-camera enable/disable, sampling rate and digital zoom controls
- timestamped notes and bookmarks
- optional playback of saved videos
- an action to discard a session recording

## Session metadata

Alongside the videos, the system saves an ordered list of timestamped events:

- session ID
- timestamp
- action
  - sampling rate: camera and rate
  - digital zoom: camera, position and zoom
  - bookmark: note
  - recording: camera and enabled/disabled state

Camera parameters are defined per session and camera, not per recording file.

## Offline processing pipeline

1. Decode each recording into frames. Sampling-rate events define its sampling schedule.
2. Extract a sample sequence from each recording according to that schedule.
3. Optionally blur faces.
4. Create frame groups.
5. Batch frame groups.
6. Send each frame batch with the system prompt and previous context.

System prompt:
```text
You are analyzing an exercise from sampled recording frames. Produce the following:

context: a high-level description of what is currently happening, using the previous batch context
actions: the exact actions and items visible in the frames. Follow this format: {format}

Context of the previous batch: {context}
Correct sequence of actions to compare to: {checklist}
```

7. Extract all `{actions}`, reformat them and compare them with the checklist.

## Open questions

- How should cameras with fixed IP addresses be discovered and registered by the custom application?
- How should the NAS provide NTP to the cameras, operator laptop and services on the isolated network?

---

# 6. Safety

Retention strategy:
- Once used at home, cleanup everything, except maybe metadata ?
- Warn when there's not enough space and suggestion an action.

Security:
- unique password per camera
- No accessible port in the container
- Encrypt data at rest for laptop and NAS.
- Empty SD-card after recovery protocol is performed to move on an encrypted space.
- Software require password
- Protected software from copy ?
