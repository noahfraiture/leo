const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const observations = {};
let video;
try {
    for (let index = 0; index < 100; index++) {
        video = document.getElementById("probe-video");
        if (video) break;
        await sleep(50);
    }
    if (!video) throw new Error("Video element missing");
    const targetStart = __INITIAL_SEEK_SECONDS__;
    for (let index = 0; index < 200; index++) {
        if (video.error) throw new Error("Decode error " + video.error.code);
        if (video.readyState >= 2 && !video.seeking && Math.abs(video.currentTime - targetStart) < 0.2) break;
        await sleep(50);
    }
    observations.initial = inspectVideoPixels(video);
    if (video.readyState < 2 || video.seeking || Math.abs(video.currentTime - targetStart) >= 0.2) {
        throw new Error("Initial metadata/decode/seek timeout");
    }
    const before = video.currentTime;
    await video.play();
    await sleep(500);
    video.pause();
    observations.played = await waitForVideoPixels(video);
    if (video.currentTime <= before || video.videoWidth === 0) throw new Error("Playback did not advance");
    const target = Math.min(video.duration - 0.2, video.duration * 0.6);
    video.currentTime = target;
    for (let index = 0; index < 100 && (video.seeking || Math.abs(video.currentTime - target) > 0.1); index++) {
        await sleep(50);
    }
    observations.seeked = await waitForVideoPixels(video, true);
    if (video.seeking || Math.abs(video.currentTime - target) > 0.1) throw new Error("Seek did not complete");
    if (!observations.played.pixels?.nonBlack || !observations.seeked.pixels?.nonBlack) {
        throw new Error("The known-lit diagnostic clip did not yield nonblack canvas pixels");
    }
    dioxus.send({ ok: true, targetSeconds: target, observations });
} catch (error) {
    if (video) observations.failure = inspectVideoPixels(video, true);
    dioxus.send({ ok: false, error: String(error), observations });
}
