const config = await dioxus.recv();
// Capture this mount's element before waiting; a later mount can reuse its DOM ID.
const video = document.getElementById(config.video_id);
if (!video || video.dataset.previewOwner !== config.mount_id) return;
const deadline = Date.now() + 5000;
while (video.isConnected && !window.MediaMTXWebRTCReader && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 50));
}
if (!video.isConnected) return;
if (!window.MediaMTXWebRTCReader) {
  dioxus.send("MediaMTX reader failed to load");
  return;
}
const reader = new MediaMTXWebRTCReader({
  url: config.whep_url,
  user: "leo-preview",
  pass: config.access_password,
  onError: (error) => dioxus.send(error),
  onTrack: (event) => {
    video.srcObject = event.streams[0];
    dioxus.send(null);
  },
});
await dioxus.recv();
reader.close();
video.srcObject = null;
