// Test-support only. Canvas checks decoded pixels, not macOS presentation or screen capture.
function inspectVideoPixels(video, includePng = false) {
    const quality = video.getVideoPlaybackQuality?.();
    const rect = video.getBoundingClientRect();
    const result = {
        state: {
            readyState: video.readyState,
            documentVisibility: document.visibilityState,
            elementRect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
            viewport: { width: innerWidth, height: innerHeight },
            networkState: video.networkState,
            width: video.videoWidth,
            height: video.videoHeight,
            currentTime: video.currentTime,
            duration: video.duration,
            paused: video.paused,
            seeking: video.seeking,
            ended: video.ended,
            error: video.error ? { code: video.error.code, message: video.error.message } : null,
            playbackQuality: quality ? {
                totalVideoFrames: quality.totalVideoFrames,
                droppedVideoFrames: quality.droppedVideoFrames,
                corruptedVideoFrames: quality.corruptedVideoFrames ?? null,
            } : null,
            webkitDecodedFrameCount: video.webkitDecodedFrameCount ?? null,
        },
    };
    try {
        if (video.readyState < 2 || !video.videoWidth || !video.videoHeight) {
            throw new Error("No decoded video frame is available");
        }
        const canvas = document.createElement("canvas");
        canvas.width = 320;
        canvas.height = Math.max(1, Math.round(320 * video.videoHeight / video.videoWidth));
        const context = canvas.getContext("2d", { willReadFrequently: true });
        context.drawImage(video, 0, 0, canvas.width, canvas.height);
        const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
        let min = 255, max = 0, sum = 0, sumSquares = 0, litPixels = 0;
        for (let index = 0; index < data.length; index += 4) {
            let lit = false;
            for (let channel = 0; channel < 3; channel++) {
                const value = data[index + channel];
                min = Math.min(min, value);
                max = Math.max(max, value);
                sum += value;
                sumSquares += value * value;
                lit ||= value > 16;
            }
            if (lit) litPixels++;
        }
        const count = canvas.width * canvas.height * 3;
        const mean = sum / count;
        result.pixels = {
            width: canvas.width, height: canvas.height, min, max, mean,
            standardDeviation: Math.sqrt(Math.max(0, sumSquares / count - mean * mean)),
            litPixelFraction: litPixels / (canvas.width * canvas.height),
            nonBlack: max > 16 && mean > 2,
        };
        if (includePng) result.framePng = canvas.toDataURL("image/png");
    } catch (error) {
        result.canvasError = String(error);
    }
    return result;
}

// WebKit can report metadata and settled seeking before publishing a drawable frame.
async function waitForVideoPixels(video, includePng = false) {
    const started = performance.now();
    let result = inspectVideoPixels(video);
    while (!result.pixels?.nonBlack && !result.state.error && performance.now() - started < 5000) {
        await new Promise(resolve => setTimeout(resolve, 50));
        result = inspectVideoPixels(video);
    }
    result = inspectVideoPixels(video, includePng);
    result.pixelWaitMs = performance.now() - started;
    return result;
}
