# Multi-camera training recording system

## 1. Goal

Record training sessions using approximately three to five AXIS P3278-LV cameras.

The system must:

* record locally without depending on the internet
* record reliably for an entire training day
* continue recording if the operator laptop crashes or sleeps
* allow an operator to monitor all cameras
* support camera selection, digital zoom, quality profiles and annotations
* provide live frames to an AI analysis pipeline
* retain original high-quality recordings
* detect failures and recording gaps quickly
* make recordings accessible programmatically
* require little or no manual post-production
* allow exported or processed data to be taken home

This is not a traditional surveillance deployment. It is a session-oriented video capture and analysis system.

---

# 2. Recommended architecture

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

* archival recording
* recording start/stop, unless testing shows direct control is needed
* the recording stream configuration
* storage and retention
* reconnection after camera/network interruptions
* recording catalogue and health monitoring
* recording recovery
* bookmarks where useful

### Custom application owns

* session workflow
* operator interface
* low-resolution previews
* AI stream consumption
* timestamped custom metadata
* PTZ or digital-view controls
* operator presets
* session naming
* downstream processing status
* alarms presented to the operator

### Axis VAPIX owns direct device operations

* camera status and capabilities
* separate preview and AI streams
* supported PTZ controls
* stream profiles
* snapshots
* overlays
* camera-side events
* advanced settings that Surveillance Station does not expose

Do not let both the VMS and the custom application repeatedly modify the same recording profile.

---

# 3. Important camera limitations

The AXIS P3278-LV is an 8 MP fixed dome camera. It supports H.264, H.265 and AV1, but its maximum frame rate is 25/30 fps, not 60 fps. It is also not a remotely movable PTZ camera, and Axis lists no remote PTRZ support.

Consequences:

* 4K/60 recording is not available with this model.
* The operator cannot physically pan or tilt the camera.
* Optical framing should be configured during installation.
* Live "zoom" must normally be digital crop/zoom.
* Digital zoom reduces effective resolution.
* If real physical camera movement between presets is required, at least one actual Axis PTZ camera is needed.

The camera supports PoE Class 3, so the proposed PoE switch has ample power capacity.

---

# 4. Hardware checklist

## Cameras

For each camera:

* AXIS P3278-LV
* correct mounting hardware
* Cat 6 Ethernet cable
* optional high-endurance microSD card
* unique device name
* fixed DHCP reservation or static IP
* strong individual credentials
* synchronized clock through NTP

Suggested names:

```text
room-a-wide
room-a-instructor
room-a-students-left
room-a-students-right
room-a-detail
```

Avoid names such as `camera1`, because names will appear in files, metadata and alerts.

## NAS

Recommended:

```text
Synology DS224+
2 x 4 TB or 2 x 8 TB HDD
SHR-1
```

The DS224+ is normally diskless, supports Surveillance Station, and includes two Surveillance Station device licenses. Additional cameras require additional licenses.

### Disk choice

Use HDDs, not SSDs, unless shock resistance or silence is worth the additional cost.

Suitable classes:

* Synology HAT3300/HAT3310
* Seagate IronWolf
* WD Red Plus
* compatible surveillance-grade drives

Check the exact model against Synology's compatibility list before purchase.

### Capacity

For three P3278-LV cameras, eight hours, at realistic H.265 bitrates:

| Approximate bitrate per camera | Total storage |
| -----------------------------: | ------------: |
|                       4 Mbit/s |         43 GB |
|                       8 Mbit/s |         86 GB |
|                      12 Mbit/s |        130 GB |
|                      20 Mbit/s |        216 GB |
|                      30 Mbit/s |        324 GB |

The camera is limited to 30 fps, so the previously discussed 4K/60 estimates do not apply.

Even 4 TB usable storage gives substantial room for several sessions. Two 4 TB disks in SHR-1 provide approximately 4 TB usable before filesystem overhead.

## PoE switch

Preferred European option:

```text
Teltonika TSW202
```

Requirements:

* at least five PoE+ ports
* enough normal Ethernet ports for the NAS and laptop/uplink
* adequate PoE power budget
* per-port power monitoring
* ability to remotely cycle power on a camera port
* gigabit Ethernet

A managed switch is useful because the operator or administrator can:

* check whether a camera port is connected
* inspect PoE consumption
* restart a frozen camera by power-cycling its port
* monitor link errors
* separate cameras using VLANs later

Confirm whether the selected Teltonika package includes the required power supply.

## UPS

The UPS should power:

```text
Synology NAS
PoE switch
optional local router
```

The laptop already has a battery.

Requirements:

* line-interactive
* USB communication supported by Synology
* approximately 950-1200 VA
* correct Belgian/European sockets
* enough runtime for short outages
* automatic NAS safe-mode support

Synology DSM can receive UPS status through USB and enter safe mode before battery exhaustion, stopping services and protecting mounted storage.

Suggested class:

* APC Back-UPS 950/1200 VA under Schneider Electric
* Eaton Ellipse USB
* equivalent Synology-compatible UPS

Test compatibility for the exact UPS model, not only the brand.

## Operator laptop

Because the NAS/VMS handles recording, the laptop requirements are moderate.

Recommended:

* recent Intel Core i5 or AMD Ryzen 5
* 16 GB RAM
* SSD
* wired gigabit Ethernet
* USB-C or HDMI for external monitor
* hardware H.264/H.265 decoding
* Windows or macOS depending on the software client and custom application

Use a wired network for the main session. Wi-Fi may remain available for internet access, but previews and camera control should use Ethernet.

## External monitor and conferencing bar

Recommended structure:

```text
32-inch 4K USB-C monitor
+
USB meeting bar with camera, microphone and speaker
```

The meeting bar is independent of the recording cameras.

Possible products:

* Logitech MeetUp
* Jabra PanaCast 50
* Poly Studio family

The monitor connects to the laptop over USB-C or HDMI. The meeting bar connects by USB and appears as a webcam, microphone and speaker.

---

# 5. Network design

## Simple network

```text
Router or DHCP server
       |
PoE switch
├── Camera 1
├── Camera 2
├── Camera 3
├── Camera 4
├── Camera 5
├── Synology NAS
└── Operator laptop
```

A router is useful even without internet because it can provide:

* DHCP
* DNS
* NTP forwarding
* predictable addressing
* local firewall rules

## IP addressing

Use DHCP reservations rather than manually configured static addresses where practical.

Example:

```text
NAS             192.168.50.10
Camera 1        192.168.50.21
Camera 2        192.168.50.22
Camera 3        192.168.50.23
Camera 4        192.168.50.24
Camera 5        192.168.50.25
Operator laptop DHCP
```

## Time synchronization

All of the following must use the same clock source:

* cameras
* Synology
* operator laptop
* AI service
* metadata service

This is more important than firing every start command in the exact same millisecond.

Store timestamps in UTC internally.

## Internet isolation

The system can work locally.

Recommended policy:

* cameras cannot access the public internet
* NAS internet access is disabled or restricted except during controlled updates
* operator laptop may have internet through a separate interface if required
* use local NTP or temporarily permit a trusted NTP source
* do not expose cameras or DSM directly to the internet

---

# 6. Surveillance Station configuration

## Camera compatibility

Before final purchase, check every exact camera model against Synology's camera compatibility database. Compatibility can vary by feature, including PTZ, audio, motion events and codecs.

## Camera licenses

The DS224+ includes two camera licenses. For five cameras, three additional device licenses are normally required.

## Recording profile

Keep the archival profile stable during a session.

Recommended starting profile:

```text
Resolution: 3840 x 2160
Frame rate: 25/30 fps
Codec: H.265
Bitrate control: Axis Zipstream or appropriate variable bitrate
Audio: only if required and legally permitted
```

Do not change the VMS recording stream repeatedly during a session unless testing proves that transitions are clean.

Create separate streams for:

```text
Archival: 4K, 25/30 fps
Operator preview: 720p or 1080p, 5-15 fps
AI analysis: 720p or 1080p, 1-10 fps
```

Axis supports multiple media-stream interfaces and standard tools such as FFmpeg and VLC. Its HTTP interface supports single and multipart images.

## Recording mode

For training sessions:

* manual session recording, or
* scheduled recording with manual override

Avoid relying exclusively on motion-triggered recording because important low-motion periods could be missed.

## Retention

Suggested policy:

```text
Keep recordings until:
- successfully exported/processed, and
- at least one additional safety day has passed

Then:
- delete oldest completed sessions when free space falls below threshold
```

Configure a storage reserve so the NAS never reaches 100%.

Suggested thresholds:

* warning below 20% free
* block new session start below the estimated capacity for one full session plus margin
* critical alarm below 10%

## Bookmarks

Surveillance Station supports bookmarks and external integration mechanisms, including custom event bookmarks and incoming event data.

Use VMS bookmarks when they improve human review.

Use your own metadata database for structured AI and production data.

## Export

Use Surveillance Station export when normal portable video files are needed.

For programmatic workflows, prefer:

* Surveillance Station API, when recordings are managed by the VMS
* direct camera/NAS files only when the recording method intentionally exposes ordinary files

Do not write software that depends on undocumented internal Surveillance Station directories.

---

# 7. Axis VAPIX capabilities

VAPIX is Axis's device-control API family. Exact APIs vary by model and AXIS OS, so the application must discover capabilities rather than assume every endpoint exists.

## Device information

Use VAPIX to retrieve:

* model
* serial number
* firmware/AXIS OS version
* supported codecs
* supported resolutions
* supported frame rates
* supported stream profiles
* storage capabilities
* PTZ/view-area capabilities
* available analytics/events

Cache capabilities, but re-check after firmware changes.

## Live video

Available approaches:

### RTSP H.264/H.265

Best for:

* continuous operator previews
* live AI analysis
* bandwidth-efficient streaming

### HTTP multipart MJPEG

Best for:

* very simple frame consumption
* prototypes
* cases where bandwidth is not important

### JPEG snapshots

Best for:

* one frame every few seconds
* health screenshots
* low-frequency AI sampling

Axis officially supports single images, multipart images and standard media streams through VAPIX.

## Stream profiles

Stream profiles can hold:

* codec
* resolution
* frame rate
* compression
* bitrate-related settings
* other camera-specific stream parameters

Use named profiles:

```text
archive
operator-preview
ai-low
ai-high
```

Do not mutate one shared profile from multiple systems.

## Adjustable live stream

Axis provides an adjustable RTSP live-stream API for changing a subset of stream settings without restarting the stream. However, Axis explicitly notes that the storage video profile is incompatible with that API.

Therefore:

* use it for preview or AI streams
* do not assume it can dynamically alter the archival recording stream

## Recording APIs

Axis Edge Storage APIs can start and stop continuous recordings. Scheduled and event-triggered recordings use the event/action system.

Recording groups can define:

* stream options
* segment storage
* retention
* encryption settings
* recording-group identity and description

In the recommended hybrid architecture, Surveillance Station should normally own recording. Use direct Axis recording APIs only when:

* testing camera-to-NAS edge recording
* implementing fallback recording
* the VMS cannot satisfy a specific requirement
* you explicitly decide to replace the VMS recorder

## PTZ and zoom

The P3278-LV is not a mechanical PTZ camera.

Your application can still offer:

* player-side digital zoom
* digital crop
* predefined crop regions
* view-area switching, if supported
* installation-time optical zoom/focus configuration

Do not label these controls "camera movement" if the physical camera does not move.

## Events

Axis event APIs can expose events such as:

* storage unavailable
* recording active/inactive
* camera restart
* motion or analytics events
* input/output state
* other device-specific states

Axis supports event streaming over WebSocket.

Use events for fast notifications, but periodically reconcile actual state because event delivery alone should not be your source of truth.

## Overlays

Axis can apply camera-side text or image overlays on supported devices.

Use overlays sparingly for immutable operational information:

* camera ID
* room ID
* timestamp
* session ID

Do not burn all AI annotations into the archival video. Keep structured metadata separately.

---

# 8. Synology API versus Axis API

## Use Synology API for

* centralized camera list
* VMS camera status
* recording catalogue
* recording search
* bookmarks
* downloads/exports
* VMS-level recording operations
* timeline integration
* VMS alerts

Synology provides a Surveillance Station Web API and promotes it for third-party integrations, including custom event bookmarks.

## Use Axis VAPIX for

* camera capabilities
* direct preview/AI streams
* snapshots
* detailed stream controls
* digital views
* PTZ on compatible cameras
* overlays
* low-level camera events
* camera settings not exposed by Synology

## Use the Surveillance Station UI for

* initial configuration
* diagnostics
* advanced recording review
* manual troubleshooting
* camera compatibility checks
* retention configuration
* emergency fallback when the custom app fails

## Do not embed the full VMS UI as your main architecture

An Electron/WebView wrapper is acceptable for a prototype, but it is fragile because of:

* authentication cookies
* TLS handling
* downloads
* popups
* full-screen behavior
* UI changes after Synology updates

Prefer:

```text
Custom application UI
+
Synology APIs
+
Axis VAPIX
+
button to open full Surveillance Station
```

---

# 9. Custom operator application

## Minimum viable interface

### Main session panel

* session name
* room
* instructor
* date
* "Start session"
* "Stop session"
* elapsed time
* recording status for every camera
* NAS storage remaining
* UPS state
* warning banner

### Camera grid

For each camera:

* low-resolution live preview
* name
* online/offline
* VMS recording status
* preview-stream status
* configured archival profile
* latest recording activity
* warning state
* enlarge button
* digital crop controls
* snapshot button

### Operator controls

* start/stop all
* enable/disable selected camera
* preview quality
* predefined crop/view
* add marker
* session annotation
* mark bad camera
* acknowledge alert
* open Surveillance Station
* open camera configuration

## Start-session workflow

1. Validate NAS free space.
2. Validate UPS state.
3. Validate cameras are reachable.
4. Validate camera clocks.
5. Validate Surveillance Station sees every camera.
6. Validate preview streams.
7. Create a session record.
8. Start recording through the VMS.
9. Confirm recording status per camera.
10. Wait for confirmation from all cameras/VMS.
11. Show session as active.
12. Begin metadata logging and AI analysis.
13. Alert clearly if any camera failed to start.

Concurrent start requests are sufficient. Millisecond-level synchronization is unnecessary. Record the confirmed start timestamp per camera.

## Stop-session workflow

1. Record requested stop time.
2. Stop recording.
3. Confirm stop per camera.
4. Flush metadata.
5. Verify recordings exist.
6. Calculate expected and actual duration.
7. Record known gaps.
8. Mark session as awaiting validation.
9. Optionally start automated processing/export.
10. Mark complete only after validation.

## Application state recovery

The app must not assume that its in-memory state is authoritative.

On startup:

1. Connect to Synology.
2. Connect to every camera.
3. Query actual recording state.
4. Reload active session metadata from SQLite.
5. Reconstruct the UI.
6. Warn if recording exists without a matching active session.
7. Never automatically stop unknown active recordings.

The laptop may restart while recording. The VMS should continue recording, and the application should recover.

---

# 10. Metadata model

Use SQLite for authoritative session metadata. Optionally export JSONL later.

## Suggested entities

### Session

```text
id
name
room
operator
instructor
planned_start
requested_start
actual_start
requested_stop
actual_stop
status
notes
```

### Camera session

```text
session_id
camera_id
recording_id
requested_start
confirmed_start
confirmed_stop
profile
expected_fps
resolution
codec
gap_duration
status
```

### Event

```text
timestamp
session_id
camera_id optional
type
payload JSON
source
```

Types might include:

```text
session_started
camera_selected
crop_changed
quality_changed
operator_marker
recording_interrupted
recording_resumed
camera_offline
nas_warning
ai_detection
manual_note
```

### Processing job

```text
session_id
type
status
started_at
completed_at
output_location
error
```

## Timestamp rules

* store UTC
* preserve source timestamp
* preserve receipt timestamp for external events
* use monotonic timers for local duration calculations
* maintain per-camera clock-drift measurements

---

# 11. Live AI analysis

## Recommended stream

Do not process the archival 4K stream unless needed.

Use:

```text
720p or 1080p
2-10 fps
H.264
separate stream profile
```

Pipeline:

```text
Axis RTSP stream
-> local decoder
-> frame sampler
-> optional motion/change filter
-> AI inference
-> timestamped result
-> metadata database
```

## Frame sampling

You probably do not need every frame.

Suggested modes:

| Mode               |           Sampling |
| ------------------ | -----------------: |
| Idle monitoring    |              1 fps |
| Normal analysis    |            2-5 fps |
| Detailed movement  |             10 fps |
| Offline extraction | Full archival rate |

The custom application may dynamically change only the AI sampling rate without changing the camera recording.

## Independence

Keep these independent:

```text
Archival path:
Camera -> VMS -> NAS

AI path:
Camera -> RTSP -> analysis process
```

An AI crash must not affect recording.

## Backpressure

The AI pipeline should:

* drop old live frames when overloaded
* never build an unbounded queue
* preserve timestamps
* report analysis lag
* distinguish "no detection" from "frame not analysed"
* optionally process missed periods from NAS later

## AI result format

Store structured output:

```json
{
  "timestamp": "2026-07-13T10:15:12.420Z",
  "camera": "room-a-instructor",
  "model": "example-model",
  "event": "gesture_detected",
  "confidence": 0.91,
  "region": {
    "x": 0.31,
    "y": 0.18,
    "width": 0.22,
    "height": 0.43
  }
}
```

Do not make AI-generated events the only record of what happened.

---

# 12. Gap detection and reliability

## What the VMS handles

A VMS is valuable for:

* reconnecting to cameras
* maintaining recording state
* retaining a centralized recording catalogue
* handling storage rotation
* producing alerts
* restarting recording after transient communication problems
* maintaining recording indexes
* isolating recording from the operator laptop

## What your app should still verify

Per camera:

* camera reachable
* VMS camera state healthy
* recording state active
* latest recording timestamp advancing
* expected stream profile active
* clock offset acceptable
* no recent disconnect event
* enough storage available

## Health states

Use clear states:

```text
GREEN
Recording confirmed and current.

YELLOW
Recording active but degraded, for example preview unavailable.

RED
Recording stopped, camera unavailable, or storage failure.

GREY
Camera intentionally disabled for this session.
```

Do not use a single ambiguous "online" icon.

## Alert actions

Every alert should tell the operator what to do.

Examples:

```text
Camera 3 is offline.
Check Ethernet cable or use "Power-cycle port 3".

Camera 2 is online but not recording.
Click "Retry recording".

NAS free storage is below one-session reserve.
Export or delete a completed session before continuing.

UPS is on battery.
Recording can continue for now. Investigate mains power.

Camera clock differs by 4.2 seconds.
Resynchronize before starting the session.
```

## Recovery policy

* Automatically retry transient operations with backoff.
* Do not endlessly restart a camera without informing the operator.
* Power-cycle a PoE port only after explicit operator action or carefully defined policy.
* Record every interruption and recovery as metadata.
* Preserve partial recording segments.
* Never silently discard corrupted or incomplete segments.

## SD-card fallback

Optional SD cards can provide a second recording path during NAS/network failure.

Advantages:

* camera can retain footage while the NAS path is unavailable
* reduces risk of a complete gap

Limitations:

* retrieving and reconciling recordings is more complex
* VMS support for automatic backfill must be verified for the exact integration
* card endurance and health must be monitored
* it does not help if the camera loses power

Use SD fallback only after testing the full recovery workflow.

---

# 13. Retention strategy

For training sessions, use session-level retention rather than generic surveillance retention.

## Suggested lifecycle

```text
RECORDING
-> VALIDATING
-> READY_FOR_PROCESSING
-> PROCESSED
-> EXPORTED
-> DELETABLE
-> DELETED
```

Only automatically delete sessions in `EXPORTED` or `DELETABLE`.

## Reserve calculation

Before starting:

```text
estimated session size
+ current active-session data
+ safety margin
< available NAS storage
```

Use a conservative estimate based on actual measured bitrate after pilot testing.

## Cleanup

Suggested policy:

* keep current session
* keep previous session
* keep unprocessed sessions
* delete oldest successfully exported/processed sessions
* never delete based only on file age
* send warning before cleanup
* maintain a deletion audit log

---

# 14. File and processing strategy

If Surveillance Station owns recordings, treat its recording storage as managed data.

For downstream work:

* use supported APIs to locate/download recordings
* export or copy completed recording segments
* retain session and camera identifiers
* use FFmpeg/ffprobe for validation and frame extraction
* preserve original timestamps where possible

Example offline process:

```text
Get recording files/API download
-> ffprobe validation
-> normalize timestamps
-> split into frames or short clips
-> join with session metadata
-> run AI/post-processing
-> generate reports/results
```

Avoid transcoding unless necessary. Remuxing between containers is faster and does not reduce quality.

---

# 15. Security checklist

* unique password per camera or centrally managed secret
* separate application account with minimum required permissions
* separate Synology API account
* HTTPS where feasible
* do not hardcode credentials
* encrypt credentials at rest
* camera network inaccessible from untrusted users
* no direct internet exposure
* firmware updates performed deliberately
* export camera and NAS configuration backups
* log operator actions
* restrict deletion permissions
* disable unused services
* document recovery credentials offline

Because people are being recorded, also define:

* consent process
* visible recording indicator
* access policy
* retention period
* lawful basis
* who may export footage
* where footage may be transported
* encryption for portable drives
* procedure for lost drives

---

# 16. Testing before real deployment

## Basic recording test

* record all cameras for at least one hour
* confirm expected bitrate and storage size
* confirm audio if used
* inspect recordings from beginning, middle and end
* verify timestamps align

## Laptop failure test

* start recording
* close the custom application
* disconnect/restart the laptop
* confirm recording continues
* reopen application
* confirm state recovery

## Network failure test

* disconnect one camera
* confirm alert appears promptly
* reconnect it
* confirm VMS recovery and resulting gap
* inspect metadata

## NAS interruption test

* simulate NAS service restart safely
* verify camera/VMS response
* test optional SD fallback
* confirm recovery behaviour

## Power test

* connect NAS and PoE switch to UPS
* unplug UPS input power
* confirm recording continues
* confirm DSM detects battery mode
* verify safe-mode configuration
* restore power
* test graceful recovery

## Storage-full test

Use a test volume or artificially low quota.

Confirm:

* warning occurs before exhaustion
* new session is blocked if insufficient capacity
* active recording behaviour is understood
* retention does not delete unexported sessions

## Camera reboot test

* reboot one camera during recording
* measure gap
* confirm automatic reconnection
* confirm operator alert
* confirm recording resumes

## AI overload test

* deliberately slow the AI worker
* confirm frames are dropped safely
* confirm recording remains unaffected
* confirm analysis lag is visible

## Firmware/update test

After any firmware or DSM/Surveillance Station update:

* run a shorter version of all critical tests
* verify API capabilities
* verify stream URLs
* verify recording profiles
* verify custom application authentication

---

# 17. Operational checklist

## Before the day

* NAS healthy
* both disks healthy
* UPS battery healthy
* enough storage for at least one full session plus margin
* all cameras reachable
* all clocks synchronized
* recording profiles correct
* operator preview working
* AI pipeline working
* camera lenses clean
* framing correct
* consent and recording notice ready
* external export disk available if required

## Before each session

* create session
* check all green indicators
* verify NAS free space
* verify mains/UPS state
* verify previews
* start recording
* wait for confirmed recording on every camera
* perform a quick operator marker test
* begin training only after confirmation

## During session

* operator watches status, not every frame continuously
* respond to red alerts
* add structured markers
* avoid modifying archival quality casually
* use digital crop or preview controls instead
* note any physical obstruction or framing issue

## After session

* stop recording
* confirm all cameras stopped
* validate segment duration
* inspect automatic gap report
* mark session complete
* start processing/export
* verify exported data before deleting anything
* retain at least one NAS copy until processing is confirmed

---

# 18. Recommended implementation phases

## Phase 1: Existing software only

* Synology Surveillance Station
* Axis cameras
* NAS recording
* standard Synology client
* manual markers/bookmarks
* no custom application

Goal: validate hardware, image quality, reliability and workflow.

## Phase 2: Lightweight companion app

Add:

* session creation
* five-camera preview grid
* start/stop workflow
* structured metadata
* VMS health display
* Axis capability discovery
* AI stream ingestion
* failure alerts

Do not replace VMS recording.

## Phase 3: Operator automation

Add:

* predefined digital crops
* custom annotations
* camera/profile presets
* processing/export automation
* automatic session validation
* storage forecasting
* actionable recovery buttons

## Phase 4: Evaluate VMS dependence

Only after real-world usage, decide whether the VMS is unnecessary.

Do not remove it merely because a custom implementation appears technically possible.

---

# 19. Final recommended setup

```text
3-5 AXIS P3278-LV cameras
Synology DS224+
2 x 4 TB HDD in SHR-1
Surveillance Station with enough camera licenses
Teltonika managed PoE+ switch
950-1200 VA USB-compatible UPS
Operator laptop, i5/Ryzen 5, 16 GB RAM, wired Ethernet
32-inch 4K monitor
USB conference bar
Optional high-endurance SD card per camera
Custom companion application
```

## Final software strategy

```text
Surveillance Station:
reliable archival recording, storage, reconnect, retention

Axis VAPIX:
camera-specific controls and independent streams

Custom application:
training-session workflow, metadata, monitoring and AI integration

Surveillance Station UI:
configuration, diagnostics and emergency fallback
```

This preserves the reliability of mature VMS software without preventing custom camera controls, live AI analysis or a tailored operator experience.
